// Rules: frigate/track/tracked_object.py lines 115 to 148 at v0.17.2.

#[derive(Debug, Clone, PartialEq)]
pub struct Scores {
    history: Vec<f64>,
    computed: f64,
    top: f64,
    confirmed: bool,
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        return sorted.get(middle).copied().unwrap_or(0.0);
    }
    let lower = sorted.get(middle - 1).copied().unwrap_or(0.0);
    let upper = sorted.get(middle).copied().unwrap_or(0.0);
    let middle_sum = lower + upper;
    middle_sum / 2.0
}

impl Scores {
    #[must_use]
    pub fn new(initial: &[f64]) -> Self {
        Self {
            history: initial.to_vec(),
            computed: 0.0,
            top: 0.0,
            confirmed: false,
        }
    }

    pub fn update(&mut self, score: Option<f64>, threshold: f64) {
        self.history.push(score.unwrap_or(0.0));
        if self.history.len() > 10 {
            let excess = self.history.len() - 10;
            self.history.drain(..excess);
        }
        self.computed = median(&self.history);
        if self.computed > self.top {
            self.top = self.computed;
        }
        let below_threshold = self.computed < threshold;
        self.confirmed = self.confirmed || !below_threshold;
    }

    #[must_use]
    pub fn history(&self) -> &[f64] {
        &self.history
    }

    #[must_use]
    pub const fn computed(&self) -> f64 {
        self.computed
    }

    #[must_use]
    pub const fn top(&self) -> f64 {
        self.top
    }

    #[must_use]
    pub const fn confirmed(&self) -> bool {
        self.confirmed
    }
}
