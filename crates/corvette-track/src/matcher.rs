use corvette_yolo::round_f64_to_f32;

use crate::distance::Matrix;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pair {
    pub candidate: usize,
    pub object: usize,
    pub distance: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Matching {
    pub pairs: Vec<Pair>,
    pub unmatched_candidates: Vec<usize>,
    pub unmatched_objects: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NanDistance {
    pub candidate: usize,
    pub object: usize,
}

impl std::fmt::Display for NanDistance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the distance table has a NaN at candidate {}, object {}",
            self.candidate, self.object
        )
    }
}

impl std::error::Error for NanDistance {}

// Greedy global-minimum matching in the shape of norfair 2.3.0 Tracker.match_dets_and_objs: take the first row-major cell that holds the strictly smallest value below the limit, mark its row and column used, repeat.
/// # Errors
/// Returns [`NanDistance`] naming the first NaN cell in row-major order, checked before any matching.
pub fn match_greedy(table: &Matrix, threshold: f64) -> Result<Matching, NanDistance> {
    let rows = table.rows();
    let cols = table.cols();
    if rows == 0 || cols == 0 {
        return Ok(Matching {
            pairs: Vec::new(),
            unmatched_candidates: (0..rows).collect(),
            unmatched_objects: (0..cols).collect(),
        });
    }
    for (candidate, row) in table.values().chunks(cols).enumerate() {
        for (object, cell) in row.iter().enumerate() {
            if cell.is_nan() {
                return Err(NanDistance { candidate, object });
            }
        }
    }
    let limit = round_f64_to_f32(threshold);
    let mut row_used = vec![false; rows];
    let mut col_used = vec![false; cols];
    let mut pairs: Vec<Pair> = Vec::new();
    loop {
        let mut best: Option<(usize, usize, f32)> = None;
        for (candidate, row) in table.values().chunks(cols).enumerate() {
            if row_used.get(candidate) != Some(&false) {
                continue;
            }
            for (object, cell) in row.iter().enumerate() {
                let below = matches!(cell.partial_cmp(&limit), Some(std::cmp::Ordering::Less));
                if col_used.get(object) != Some(&false) || !below {
                    continue;
                }
                let takes_it = match best {
                    None => true,
                    Some((_, _, best_cell)) => *cell < best_cell,
                };
                if takes_it {
                    best = Some((candidate, object, *cell));
                }
            }
        }
        let Some((candidate, object, distance)) = best else {
            break;
        };
        pairs.push(Pair {
            candidate,
            object,
            distance,
        });
        if let Some(used) = row_used.get_mut(candidate) {
            *used = true;
        }
        if let Some(used) = col_used.get_mut(object) {
            *used = true;
        }
    }
    let unmatched_candidates = row_used
        .iter()
        .enumerate()
        .filter(|(_, used)| !**used)
        .map(|(index, _)| index)
        .collect();
    let unmatched_objects = col_used
        .iter()
        .enumerate()
        .filter(|(_, used)| !**used)
        .map(|(index, _)| index)
        .collect();
    Ok(Matching {
        pairs,
        unmatched_candidates,
        unmatched_objects,
    })
}
