use crate::filters::Candidate;

const LIMIT: i64 = 8_388_608;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    x_min: i64,
    y_min: i64,
    x_max: i64,
    y_max: i64,
}

impl Region {
    #[must_use]
    pub const fn new(x_min: i64, y_min: i64, x_max: i64, y_max: i64) -> Option<Self> {
        if x_min > LIMIT
            || x_min < -LIMIT
            || y_min > LIMIT
            || y_min < -LIMIT
            || x_max > LIMIT
            || x_max < -LIMIT
            || y_max > LIMIT
            || y_max < -LIMIT
        {
            None
        } else {
            Some(Self {
                x_min,
                y_min,
                x_max,
                y_max,
            })
        }
    }

    #[must_use]
    pub const fn x_min(&self) -> i64 {
        self.x_min
    }

    #[must_use]
    pub const fn y_min(&self) -> i64 {
        self.y_min
    }

    #[must_use]
    pub const fn x_max(&self) -> i64 {
        self.x_max
    }

    #[must_use]
    pub const fn y_max(&self) -> i64 {
        self.y_max
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    width: i64,
    height: i64,
}

impl Frame {
    #[must_use]
    pub const fn new(width: i64, height: i64) -> Option<Self> {
        if width < 1 || width > LIMIT || height < 1 || height > LIMIT {
            None
        } else {
            Some(Self { width, height })
        }
    }

    #[must_use]
    pub const fn width(&self) -> i64 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> i64 {
        self.height
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub x_min: i64,
    pub y_min: i64,
    pub x_max: i64,
    pub y_max: i64,
    pub area: i64,
    pub ratio: f64,
}

impl Geometry {
    #[must_use]
    pub const fn candidate<'a>(&self, label: &'a str, score: f32) -> Candidate<'a> {
        Candidate {
            label,
            score,
            x_min: self.x_min,
            y_min: self.y_min,
            x_max: self.x_max,
            y_max: self.y_max,
            area: self.area,
            ratio: self.ratio,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skipped {
    OutsideFrame,
    NonFinite,
}

fn to_f32(v: i64) -> f32 {
    match (
        i16::try_from(v.div_euclid(65536)),
        u16::try_from(v.rem_euclid(65536)),
    ) {
        (Ok(high), Ok(low)) => {
            let scaled = f32::from(high) * 65536.0;
            scaled + f32::from(low)
        }
        _ => f32::NAN,
    }
}

fn sat_trunc(v: f32) -> i64 {
    let bits = v.to_bits();
    let negative = bits >> 31 != 0;
    let exp = bits >> 23 & 0xFF;
    let mag: u32 = if exp < 127 {
        0
    } else if exp >= 156 {
        536_870_912
    } else if exp < 150 {
        (0x80_0000 | (bits & 0x7F_FFFF)) >> (150 - exp)
    } else {
        (0x80_0000 | (bits & 0x7F_FFFF)) << (exp - 150)
    };
    let magnitude = i64::from(mag);
    if negative { -magnitude } else { magnitude }
}

/// # Errors
/// `Skipped::NonFinite` when a sum is not finite, `Skipped::OutsideFrame` when a minimum edge reaches the last column or row.
pub fn map_box(raw: [f32; 4], region: Region, frame: Frame) -> Result<Geometry, Skipped> {
    let [top, left, bottom, right] = raw;
    let size_f = to_f32(region.x_max - region.x_min);
    let origin_left = to_f32(region.x_min);
    let origin_top = to_f32(region.y_min);
    let top_prod = top * size_f;
    let left_prod = left * size_f;
    let bottom_prod = bottom * size_f;
    let right_prod = right * size_f;
    let top_sum = top_prod + origin_top;
    let left_sum = left_prod + origin_left;
    let bottom_sum = bottom_prod + origin_top;
    let right_sum = right_prod + origin_left;
    if !(top_sum.is_finite()
        && left_sum.is_finite()
        && bottom_sum.is_finite()
        && right_sum.is_finite())
    {
        return Err(Skipped::NonFinite);
    }
    // frigate/video.py 704-707: clamp in f32, int() truncates toward zero, edges saturate at +-2^29
    let width_limit = frame.width - 1;
    let height_limit = frame.height - 1;
    let cap_right = to_f32(width_limit);
    let cap_bottom = to_f32(height_limit);
    let edge_left = sat_trunc(if left_sum > 0.0 { left_sum } else { 0.0 });
    let edge_top = sat_trunc(if top_sum > 0.0 { top_sum } else { 0.0 });
    let edge_right = sat_trunc(if right_sum < cap_right {
        right_sum
    } else {
        cap_right
    });
    let edge_bottom = sat_trunc(if bottom_sum < cap_bottom {
        bottom_sum
    } else {
        cap_bottom
    });
    if edge_left >= width_limit || edge_top >= height_limit {
        return Err(Skipped::OutsideFrame);
    }
    let box_width = edge_right - edge_left;
    let box_height = edge_bottom - edge_top;
    let box_area = box_width * box_height;
    let divisor = if box_height > 1 { box_height } else { 1 };
    let ratio = match (i32::try_from(box_width), i32::try_from(divisor)) {
        (Ok(w), Ok(d)) => f64::from(w) / f64::from(d),
        _ => f64::NAN,
    };
    Ok(Geometry {
        x_min: edge_left,
        y_min: edge_top,
        x_max: edge_right,
        y_max: edge_bottom,
        area: box_area,
        ratio,
    })
}
