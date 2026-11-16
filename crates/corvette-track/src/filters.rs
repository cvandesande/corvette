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

    fn masks(&self, candidate: &Candidate<'_>) -> bool {
        let row = u64::from(candidate.y_max);
        let row = usize::try_from(row).map_or(self.height - 1, |row| row.min(self.height - 1));
        let mid = u64::from(candidate.x_min) + u64::from(candidate.x_max);
        let col = usize::try_from(mid / 2).map_or(self.width - 1, |col| col.min(self.width - 1));
        self.bytes.get(row * self.width + col) == Some(&0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectFilter {
    pub min_area: u64,
    pub max_area: u64,
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
    pub x_min: u32,
    pub y_min: u32,
    pub x_max: u32,
    pub y_max: u32,
    pub area: u64,
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
        if filter
            .mask
            .as_ref()
            .is_some_and(|mask| mask.masks(candidate))
        {
            return Some(Rejection::Masked);
        }
        None
    }

    /// Returns whether any stage drops `candidate`.
    #[must_use]
    pub fn is_filtered(&self, candidate: &Candidate<'_>) -> bool {
        self.first_rejection(candidate).is_some()
    }
}
