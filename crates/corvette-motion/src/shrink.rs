use crate::decision::trunc_to_whole;
use crate::plane::{Image, Plane};
use crate::shape::Shape;

const GUARD: f64 = 1e-3;

/// Returns a picture of shape `to` holding 0 wherever the `OpenCV` area shrink of `mask` to that shape gives 0 and 255 everywhere else.
///
/// Returns `None` when `to` is larger than `mask` on a side or both side ratios are whole.
#[must_use]
pub fn shrink_keep_mask(mask: Plane<'_>, to: Shape) -> Option<Image> {
    let from = mask.shape();
    if to.width() > from.width() || to.height() > from.height() {
        return None;
    }
    if from.width().is_multiple_of(to.width()) && from.height().is_multiple_of(to.height()) {
        return None;
    }
    let columns = axis_table(from.width(), to.width())?;
    let rows = axis_table(from.height(), to.height())?;
    let mut row_sums = vec![0f64; from.height() * to.width()];
    for (y, cells) in row_sums.chunks_mut(to.width()).enumerate() {
        let start = y * from.width();
        let line = mask.data().get(start..start + from.width())?;
        for (dx, cell) in cells.iter_mut().enumerate() {
            let mut kept = 0.0;
            for entry in columns.entries_of(dx) {
                let kept_pixel = if line[entry.source] == 0 { 0.0 } else { 1.0 };
                let column_part = entry.weight * kept_pixel;
                kept += column_part;
            }
            *cell = kept;
        }
    }
    let mut out = vec![0u8; to.pixels()];
    for (dy, out_row) in out.chunks_mut(to.width()).enumerate() {
        for (dx, pixel) in out_row.iter_mut().enumerate() {
            let mut kept = 0.0;
            for entry in rows.entries_of(dy) {
                let row_part = entry.weight * row_sums[entry.source * to.width() + dx];
                kept += row_part;
            }
            *pixel = if kept * 255.0 < 0.5 { 0x00 } else { 0xFF };
        }
    }
    Image::new(out, to)
}

struct Entry {
    source: usize,
    weight: f64,
}

struct AxisTable {
    entries: Vec<Entry>,
    starts: Vec<usize>,
}

impl AxisTable {
    fn entries_of(&self, cell: usize) -> &[Entry] {
        &self.entries[self.starts[cell]..self.starts[cell + 1]]
    }

    fn push_entry(&mut self, source: usize, weight: f64) {
        self.entries.push(Entry { source, weight });
    }
}

fn axis_table(from: usize, to: usize) -> Option<AxisTable> {
    let scale = whole(from) / whole(to);
    let mut table = AxisTable {
        entries: Vec::new(),
        starts: Vec::with_capacity(to + 1),
    };
    for d in 0..to {
        table.starts.push(table.entries.len());
        let a = whole(d) * scale;
        let b = a + scale;
        let cell = scale.min(whole(from) - a);
        let last = trunc_to_whole(b).min(from - 1);
        let first = trunc_to_whole(a.ceil()).min(last);
        if whole(first) - a > GUARD {
            let source = first.checked_sub(1)?;
            table.push_entry(source, (whole(first) - a) / cell);
        }
        for s in first..last {
            table.push_entry(s, 1.0 / cell);
        }
        if b - whole(last) > GUARD {
            table.push_entry(last, (b - whole(last)).min(1.0).min(cell) / cell);
        }
    }
    table.starts.push(table.entries.len());
    Some(table)
}

fn whole(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}
