use crate::shape::Shape;

const MAX_SIDE: usize = 16384;
const COEF_SCALE: f32 = 2048.0; // OpenCV's fixed-point coefficient grid, 1 << 11

struct Tap {
    start: usize,
    next: usize,
    w0: i32,
    w1: i32,
}

/// Resizes an interleaved RGB picture of `from` pixels to `to` by linear interpolation.
/// Returns `None` on a wrong input length, a side over `MAX_SIDE`, or a size overflow.
#[must_use]
pub fn resize_linear_rgb(src: &[u8], from: Shape, to: Shape) -> Option<Vec<u8>> {
    if from.width() > MAX_SIDE
        || from.height() > MAX_SIDE
        || to.width() > MAX_SIDE
        || to.height() > MAX_SIDE
    {
        return None;
    }
    let out_len = to.pixels().checked_mul(3)?;
    if src.len() != from.pixels().checked_mul(3)? {
        return None;
    }
    if from == to {
        return Some(src.to_vec());
    }
    if from.width() == 2 * to.width() && from.height() == 2 * to.height() {
        return box_average(src, from, to); // the exact-2 geometry takes OpenCV's area path
    }
    let columns = axis_taps(from.width(), to.width())?;
    let rows = axis_taps(from.height(), to.height())?;
    let row_bytes = from.width() * 3;
    let mut out = vec![0u8; out_len];
    for (out_row, tap) in out.chunks_mut(to.width() * 3).zip(&rows) {
        let above = horizontal_row(src, tap.start, row_bytes, &columns)?;
        let below = if tap.next == tap.start {
            above.clone()
        } else {
            horizontal_row(src, tap.next, row_bytes, &columns)?
        };
        for ((a, b), byte) in above.iter().zip(&below).zip(out_row) {
            // The `>> 4` per horizontal half and the `>> 16` per vertical weight before the
            // final `+ 2` shift are OpenCV's fixed-point cast to uchar, not single-rounding.
            let value = ((tap.w0 * (a >> 4)) >> 16) + ((tap.w1 * (b >> 4)) >> 16) + 2;
            *byte = u8::try_from((value >> 2).clamp(0, 255)).unwrap_or(0); // clamp keeps 0..=255
        }
    }
    Some(out)
}

fn axis_taps(src_side: usize, dst_side: usize) -> Option<Vec<Tap>> {
    let scale =
        1.0 / (f64::from(u32::try_from(dst_side).ok()?) / f64::from(u32::try_from(src_side).ok()?));
    let mut taps = Vec::with_capacity(dst_side);
    for d in 0..dst_side {
        let scaled = (f64::from(u32::try_from(d).ok()?) + 0.5) * scale; // OpenCV's double pixel-centre mapping, rounded twice on purpose
        let fx = round_to_f32(scaled - 0.5); // quantization to float changes weights and cannot be skipped
        let (start, frac) = if fx < 0.0 {
            (0usize, 0.0f32)
        } else {
            let s = usize::try_from(whole_value(fx.floor())?).ok()?;
            if s >= src_side - 1 {
                (src_side - 1, 0.0)
            } else {
                (s, fx - f32::from(u16::try_from(s).ok()?))
            }
        };
        let w0 = i32::try_from(whole_value(
            ((1.0f32 - frac) * COEF_SCALE).round_ties_even(),
        )?)
        .ok()?;
        let w1 = i32::try_from(whole_value((frac * COEF_SCALE).round_ties_even())?).ok()?;
        taps.push(Tap {
            start,
            next: (start + 1).min(src_side - 1),
            w0,
            w1,
        });
    }
    Some(taps)
}

fn whole_value(value: f32) -> Option<u32> {
    let bits = value.to_bits();
    let exponent = (bits >> 23) & 0xFF;
    if exponent < 127 {
        return Some(0); // the domain is integral non-negative f32 below 2^24, so below 1.0 it is 0
    }
    let shift = 150u32.checked_sub(exponent)?; // 150 = 127 + 23: shifts the 24-bit significand down
    if shift > 23 {
        return None;
    }
    Some(((bits & 0x7F_FFFF) | 0x80_0000) >> shift)
}

fn round_to_f32(value: f64) -> f32 {
    let bits = value.to_bits();
    let sign = bits >> 63;
    let exponent = (bits >> 52) & 0x7FF;
    let fraction = bits & 0xF_FFFF_FFFF_FFFF;
    if exponent == 0x7FF {
        return if sign == 0 {
            f32::INFINITY
        } else {
            f32::NEG_INFINITY
        };
    }
    if exponent == 0 {
        return f32::from_bits(u32::try_from(sign << 31).unwrap_or(0));
    }
    let target = exponent + 127 - 1023; // values below 2^-97 are outside this module's domain
    let kept = fraction >> 29;
    let dropped = fraction & 0x1FFF_FFFF;
    let half = 1 << 28;
    let round_up = dropped > half || (dropped == half && (kept & 1) == 1);
    let (target, kept) = if !round_up {
        (target, kept)
    } else if kept == 0x7F_FFFF {
        (target + 1, 0)
    } else {
        (target, kept + 1)
    };
    f32::from_bits(u32::try_from((sign << 31) | (target << 23) | kept).unwrap_or(0))
}

fn horizontal_row(src: &[u8], row: usize, row_bytes: usize, taps: &[Tap]) -> Option<Vec<i32>> {
    let base = row.checked_mul(row_bytes)?;
    let line = src.get(base..base.checked_add(row_bytes)?)?;
    let mut out = vec![0i32; taps.len().checked_mul(3)?];
    for (tap, cell) in taps.iter().zip(out.chunks_mut(3)) {
        let left = line.get(tap.start * 3..tap.start * 3 + 3)?;
        let right = line.get(tap.next * 3..tap.next * 3 + 3)?;
        for ((l, r), slot) in left.iter().zip(right).zip(cell) {
            *slot = i32::from(*l) * tap.w0 + i32::from(*r) * tap.w1;
        }
    }
    Some(out)
}

fn box_average(src: &[u8], from: Shape, to: Shape) -> Option<Vec<u8>> {
    let row_bytes = from.width().checked_mul(3)?;
    let mut out = Vec::with_capacity(to.pixels().checked_mul(3)?);
    let mut rows = src.chunks_exact(row_bytes); // two whole source rows feed each output row
    while let (Some(above), Some(below)) = (rows.next(), rows.next()) {
        for pair in above.chunks_exact(6).zip(below.chunks_exact(6)) {
            let (a, b) = (pair.0.get(..3)?, pair.0.get(3..)?); // 6 bytes = two pixels of one row
            let (c, d) = (pair.1.get(..3)?, pair.1.get(3..)?);
            out.extend(
                a.iter()
                    .zip(b)
                    .zip(c.iter().zip(d))
                    .map(|((a, b), (c, d))| {
                        let sum = u32::from(*a) + u32::from(*b) + u32::from(*c) + u32::from(*d) + 2;
                        u8::try_from(sum >> 2).unwrap_or(0) // the sum of four bytes plus 2 fits a byte after >>2
                    }),
            );
        }
    }
    Some(out)
}
