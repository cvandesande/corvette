use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectMask {
    width: usize,
    height: usize,
    bytes: Vec<u8>,
}

impl ObjectMask {
    /// Builds a row-major `width` by `height` mask where a zero byte masks its cell and any other byte leaves it open.
    #[must_use]
    pub fn new(width: usize, height: usize, bytes: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let cells = width.checked_mul(height)?;
        if bytes.len() != cells {
            return None;
        }
        Some(Self {
            width,
            height,
            bytes,
        })
    }

    fn cell(&self, candidate: &Candidate<'_>) -> Cell {
        let Ok(height) = i64::try_from(self.height) else {
            return Cell::Outside;
        };
        let Ok(width) = i64::try_from(self.width) else {
            return Cell::Outside;
        };
        let row = candidate.y_max.min(height - 1);
        let mid = candidate.x_min.saturating_add(candidate.x_max) / 2;
        let col = mid.min(width - 1);
        let Some(row) = resolve(row, height) else {
            return Cell::Outside;
        };
        let Some(col) = resolve(col, width) else {
            return Cell::Outside;
        };
        if self.bytes.get(row * self.width + col) == Some(&0) {
            Cell::Masked
        } else {
            Cell::Open
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    Open,
    Masked,
    Outside,
}

fn resolve(index: i64, length: i64) -> Option<usize> {
    let placed = if (0..length).contains(&index) {
        index
    } else if (-length..0).contains(&index) {
        length + index
    } else {
        return None;
    };
    usize::try_from(placed).ok()
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectFilter {
    pub min_area: i64,
    pub max_area: i64,
    pub min_score: f64,
    pub min_ratio: f64,
    pub max_ratio: f64,
    pub mask: Option<ObjectMask>,
}

impl Default for ObjectFilter {
    fn default() -> Self {
        Self {
            min_area: 0,
            max_area: 24_000_000,
            min_score: 0.5,
            min_ratio: 0.0,
            max_ratio: 24_000_000.0,
            mask: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate<'a> {
    pub label: &'a str,
    pub score: f32,
    pub x_min: i64,
    pub y_min: i64,
    pub x_max: i64,
    pub y_max: i64,
    pub area: i64,
    pub ratio: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    NotTracked,
    MinArea,
    MaxArea,
    MinScore,
    MinRatio,
    MaxRatio,
    Masked,
    OutsideMask,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectFilters {
    track: Vec<String>,
    filters: BTreeMap<String, ObjectFilter>,
}

impl ObjectFilters {
    #[must_use]
    pub const fn new(track: Vec<String>, filters: BTreeMap<String, ObjectFilter>) -> Self {
        Self { track, filters }
    }

    /// Returns the first stage that drops `candidate`, in the order Frigate's `is_object_filtered` checks them, or `None` when every stage keeps it.
    #[must_use]
    pub fn first_rejection(&self, candidate: &Candidate<'_>) -> Option<Rejection> {
        if !self.track.iter().any(|tracked| tracked == candidate.label) {
            return Some(Rejection::NotTracked);
        }
        let filter = self.filters.get(candidate.label)?;
        if filter.min_area > candidate.area {
            return Some(Rejection::MinArea);
        }
        if filter.max_area < candidate.area {
            return Some(Rejection::MaxArea);
        }
        // The score compares in f64: min_score is a double against the double that holds the 32-bit score exactly.
        if filter.min_score > f64::from(candidate.score) {
            return Some(Rejection::MinScore);
        }
        if filter.min_ratio > candidate.ratio {
            return Some(Rejection::MinRatio);
        }
        if filter.max_ratio < candidate.ratio {
            return Some(Rejection::MaxRatio);
        }
        match filter.mask.as_ref().map(|mask| mask.cell(candidate)) {
            Some(Cell::Masked) => return Some(Rejection::Masked),
            Some(Cell::Outside) => return Some(Rejection::OutsideMask),
            Some(Cell::Open) | None => {}
        }
        None
    }

    /// Returns whether any stage drops `candidate`.
    #[must_use]
    pub fn is_filtered(&self, candidate: &Candidate<'_>) -> bool {
        self.first_rejection(candidate).is_some()
    }
}
