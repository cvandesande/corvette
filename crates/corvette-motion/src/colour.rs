use crate::crop::RegionError;
use crate::plane::Plane;

const CY: i32 = 1_220_542;
const CUB: i32 = 2_116_026;
const CUG: i32 = -409_993;
const CVG: i32 = -852_492;
const CVR: i32 = 1_673_527;
const ROUND: i32 = 1 << 19;
const SHIFT: u32 = 20;

/// The bytes `cv2.cvtColor(canvas, cv2.COLOR_YUV2RGB_I420)` returns: `side * side * 3`, rows top to bottom, each pixel R, G, B.
///
/// # Errors
/// - `CanvasShape` unless the plane is `side` columns by `side + side / 2` rows with `side` a multiple of 4 and at least 4.
pub fn i420_to_rgb(canvas: Plane<'_>) -> Result<Vec<u8>, RegionError> {
    let side = canvas.shape().width();
    let rows = canvas.shape().height();
    if !side.is_multiple_of(4) || side < 4 || rows != side + side / 2 {
        return Err(RegionError::CanvasShape);
    }
    let y_bytes = side.checked_mul(side).ok_or(RegionError::CanvasShape)?;
    let out_len = y_bytes.checked_mul(3).ok_or(RegionError::CanvasShape)?;
    let data = canvas.data();
    let mut rgb = vec![0u8; out_len];
    let half = side / 2;
    for y in 0..side {
        let row = y.checked_mul(side).ok_or(RegionError::CanvasShape)?;
        let packed = y_bytes
            + (y / 4).checked_mul(side).ok_or(RegionError::CanvasShape)?
            + (y / 2 % 2) * half;
        let v_base = packed + y_bytes / 4;
        let (Some(luma), Some(u_row), Some(v_row)) = (
            data.get(row..row + side),
            data.get(packed..packed + half),
            data.get(v_base..v_base + half),
        ) else {
            return Err(RegionError::CanvasShape);
        };
        for pair in 0..half {
            let (Some(&u), Some(&v)) = (u_row.get(pair), v_row.get(pair)) else {
                return Err(RegionError::CanvasShape);
            };
            let (uu, vv) = (i32::from(u) - 128, i32::from(v) - 128);
            let ruv = ROUND + CVR * vv;
            let guv = ROUND + CVG * vv + CUG * uu;
            let buv = ROUND + CUB * uu;
            for column in 0..2 {
                let x = pair * 2 + column;
                let Some(&luma_byte) = luma.get(x) else {
                    return Err(RegionError::CanvasShape);
                };
                let yy = (i32::from(luma_byte) - 16).max(0) * CY;
                let Some(pixel) = rgb.get_mut((row + x) * 3..(row + x) * 3 + 3) else {
                    return Err(RegionError::CanvasShape);
                };
                pixel.copy_from_slice(&[channel(yy + ruv), channel(yy + guv), channel(yy + buv)]);
            }
        }
    }
    Ok(rgb)
}

fn channel(sum: i32) -> u8 {
    let value = sum >> SHIFT;
    u8::try_from(value.clamp(0, 255)).unwrap_or(u8::MAX)
}
