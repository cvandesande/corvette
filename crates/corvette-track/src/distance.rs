use corvette_yolo::round_f64_to_f32;

// The two sizes ascending as np.sort([a, b]) does: swap only when b < a, so any NaN makes the ratio NaN either way (norfair_tracker.py lines 62 to 63).
fn sorted_pair(a: f64, b: f64) -> (f64, f64) {
    if b < a { (b, a) } else { (a, b) }
}

#[must_use]
pub fn distance(detection: [i32; 4], estimate: [f64; 4]) -> f64 {
    let [dx1, dy1, dx2, dy2] = detection.map(f64::from);
    let [ex1, ey1, ex2, ey2] = estimate;
    let estimate_width = ex2 - ex1;
    let estimate_height = ey2 - ey1;
    let detection_width = dx2 - dx1;
    let detection_height = dy2 - dy1;
    let detection_x_sum = dx1 + dx2;
    let detection_x = detection_x_sum / 2.0;
    let detection_y = if dy2 > dy1 { dy2 } else { dy1 };
    let estimate_x_sum = ex1 + ex2;
    let estimate_x = estimate_x_sum / 2.0;
    let estimate_y = if ey2 > ey1 { ey2 } else { ey1 };
    let change_x = (detection_x - estimate_x) / estimate_width;
    let change_y = (detection_y - estimate_y) / estimate_height;
    let (small_width, large_width) = sorted_pair(estimate_width, detection_width);
    let (small_height, large_height) = sorted_pair(estimate_height, detection_height);
    let width_ratio = large_width / small_width - 1.0;
    let height_ratio = large_height / small_height - 1.0;
    // np.linalg.norm is sqrt of the dot: the four squares summed strictly left to right before the sqrt (norfair_tracker.py line 71).
    let horizontal_square = change_x * change_x;
    let vertical_square = change_y * change_y;
    let width_square = width_ratio * width_ratio;
    let height_square = height_ratio * height_ratio;
    let offset_total = horizontal_square + vertical_square;
    let size_total = offset_total + width_square;
    let final_total = size_total + height_square;
    final_total.sqrt()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Matrix {
    rows: usize,
    cols: usize,
    values: Vec<f32>,
}

impl Matrix {
    #[must_use]
    pub fn new(rows: usize, cols: usize, values: Vec<f32>) -> Option<Self> {
        if rows.checked_mul(cols) == Some(values.len()) {
            Some(Self { rows, cols, values })
        } else {
            None
        }
    }

    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    #[must_use]
    pub const fn cols(&self) -> usize {
        self.cols
    }

    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    #[must_use]
    pub fn get(&self, row: usize, col: usize) -> Option<f32> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        let index = row.checked_mul(self.cols)?.checked_add(col)?;
        self.values.get(index).copied()
    }
}

#[must_use]
pub fn distance_matrix(candidates: &[(&str, [i32; 4])], objects: &[(&str, [f64; 4])]) -> Matrix {
    let values: Vec<f32> = candidates
        .iter()
        .flat_map(|&(candidate_label, points)| {
            objects.iter().map(move |&(object_label, estimate)| {
                if candidate_label == object_label {
                    round_f64_to_f32(distance(points, estimate))
                } else {
                    f32::INFINITY
                }
            })
        })
        .collect();
    Matrix {
        rows: candidates.len(),
        cols: objects.len(),
        values,
    }
}
