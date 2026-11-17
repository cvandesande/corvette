use crate::kalman::Kalman;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackConfig {
    hit_counter_max: u32,
    initialization_delay: u32,
    past_detections_length: u32,
    r: f64,
    q: f64,
}

impl TrackConfig {
    #[must_use]
    pub const fn new(
        hit_counter_max: u32,
        initialization_delay: u32,
        past_detections_length: u32,
        r: f64,
        q: f64,
    ) -> Option<Self> {
        if initialization_delay >= hit_counter_max {
            return None;
        }
        Some(Self {
            hit_counter_max,
            initialization_delay,
            past_detections_length,
            r,
            q,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdSource {
    created: u64,
    assigned: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation<D> {
    pub edges: [i32; 4],
    pub data: D,
}

#[derive(Clone, Debug)]
pub struct Track<D> {
    config: TrackConfig,
    label: String,
    hit_counter: i64,
    age: u64,
    is_initializing: bool,
    initializing_id: u64,
    id: Option<u64>,
    last_detection: Observation<D>,
    past_detections: Vec<(u64, Observation<D>)>,
    kalman: Kalman,
}

impl<D: Clone> Track<D> {
    // norfair tracker.py TrackedObject.__init__ 472-535 and _acquire_ids 739.
    #[must_use]
    pub fn new(
        config: &TrackConfig,
        ids: &mut IdSource,
        label: &str,
        first: Observation<D>,
    ) -> Self {
        let is_initializing = 1 <= i64::from(config.initialization_delay);
        ids.created = ids.created.saturating_add(1);
        let initializing_id = ids.created;
        let id = if is_initializing {
            None
        } else {
            ids.assigned = ids.assigned.saturating_add(1);
            Some(ids.assigned)
        };
        let past_detections = if config.past_detections_length == 0 {
            Vec::new()
        } else {
            vec![(0, first.clone())]
        };
        let kalman = Kalman::new(first.edges.map(f64::from), config.r, config.q);
        Self {
            config: *config,
            label: label.to_owned(),
            hit_counter: 1,
            age: 0,
            is_initializing,
            initializing_id,
            id,
            last_detection: first,
            past_detections,
            kalman,
        }
    }

    // norfair tracker.py tracker_step 537-548.
    pub fn step(&mut self) {
        self.hit_counter = self.hit_counter.saturating_sub(1);
        self.age = self.age.saturating_add(1);
        self.kalman.predict();
    }

    // norfair tracker.py hit 615-686 and _conditionally_add_to_past_detections 700-716.
    pub fn hit(&mut self, ids: &mut IdSource, observation: Observation<D>) {
        let age = self.age;
        let length = self.config.past_detections_length;
        if length > 0 {
            let limit = usize::try_from(length).unwrap_or(usize::MAX);
            if self.past_detections.len() < limit {
                self.past_detections.push((age, observation.clone()));
            } else if self
                .past_detections
                .first()
                .is_some_and(|(oldest_age, _)| age >= oldest_age.saturating_mul(u64::from(length)))
            {
                self.past_detections.remove(0);
                self.past_detections.push((age, observation.clone()));
            }
        }
        self.last_detection = observation;
        self.hit_counter = self
            .hit_counter
            .saturating_add(2)
            .min(i64::from(self.config.hit_counter_max));
        if self.is_initializing && self.hit_counter > i64::from(self.config.initialization_delay) {
            self.is_initializing = false;
            ids.assigned = ids.assigned.saturating_add(1);
            self.id = Some(ids.assigned);
        }
        let edges = self.last_detection.edges.map(f64::from);
        self.kalman.update(edges);
    }

    #[must_use]
    pub const fn hit_counter(&self) -> i64 {
        self.hit_counter
    }

    #[must_use]
    pub const fn age(&self) -> u64 {
        self.age
    }

    #[must_use]
    pub const fn is_initializing(&self) -> bool {
        self.is_initializing
    }

    #[must_use]
    pub const fn initializing_id(&self) -> u64 {
        self.initializing_id
    }

    #[must_use]
    pub const fn id(&self) -> Option<u64> {
        self.id
    }

    #[must_use]
    pub const fn hit_counter_is_positive(&self) -> bool {
        self.hit_counter >= 0
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        !self.is_initializing && self.hit_counter >= 0
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn estimate(&self) -> [f64; 4] {
        self.kalman.position()
    }

    #[must_use]
    pub fn velocity(&self) -> [f64; 4] {
        self.kalman.velocity()
    }

    #[must_use]
    pub const fn last_detection(&self) -> &Observation<D> {
        &self.last_detection
    }

    #[must_use]
    pub fn past_detections(&self) -> &[(u64, Observation<D>)] {
        &self.past_detections
    }
}
