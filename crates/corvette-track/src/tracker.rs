use crate::distance::distance_matrix;
use crate::matcher::{NanDistance, match_greedy};
use crate::track::{IdSource, Observation, Track, TrackConfig};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sighting<D> {
    pub label: String,
    pub observation: Observation<D>,
}

#[derive(Clone, Debug)]
pub struct Tracker<D> {
    config: TrackConfig,
    distance_threshold: f64,
    tracks: Vec<Track<D>>,
}

fn run_pass<D: Clone>(
    tracks: &mut [Track<D>],
    ids: &mut IdSource,
    threshold: f64,
    want_initializing: bool,
    candidates: Vec<Sighting<D>>,
) -> Result<Vec<Sighting<D>>, NanDistance> {
    if candidates.is_empty() {
        return Ok(candidates);
    }
    let mut object_of_column: Vec<usize> = Vec::new();
    for (index, track) in tracks.iter().enumerate() {
        if track.is_initializing() == want_initializing {
            object_of_column.push(index);
        }
    }
    let object_cells: Vec<(&str, [f64; 4])> = object_of_column
        .iter()
        .filter_map(|&index| tracks.get(index))
        .map(|track| (track.label(), track.estimate()))
        .collect();
    let candidate_cells: Vec<(&str, [i32; 4])> = candidates
        .iter()
        .map(|sighting| (sighting.label.as_str(), sighting.observation.edges))
        .collect();
    let table = distance_matrix(&candidate_cells, &object_cells);
    let matching = match_greedy(&table, threshold)?;
    let mut pool: Vec<Option<Sighting<D>>> = candidates.into_iter().map(Some).collect();
    for pair in &matching.pairs {
        let Some(sighting) = pool.get_mut(pair.candidate).and_then(Option::take) else {
            continue;
        };
        let Some(track_index) = object_of_column.get(pair.object).copied() else {
            continue;
        };
        if let Some(track) = tracks.get_mut(track_index) {
            track.hit(ids, sighting.observation);
        }
    }
    let mut leftover: Vec<Sighting<D>> = Vec::new();
    for &candidate_index in &matching.unmatched_candidates {
        if let Some(sighting) = pool.get_mut(candidate_index).and_then(Option::take) {
            leftover.push(sighting);
        }
    }
    Ok(leftover)
}

impl<D: Clone> Tracker<D> {
    #[must_use]
    pub const fn new(config: TrackConfig, distance_threshold: f64) -> Self {
        Self {
            config,
            distance_threshold,
            tracks: Vec::new(),
        }
    }

    // norfair tracker.py Tracker.update 144-260.
    /// # Errors
    /// Returns [`NanDistance`] when a pass distance table holds a NaN, raised at the same point as the library: after pruning, stepping and any first-pass hits, and before any new track is created.
    pub fn update(
        &mut self,
        ids: &mut IdSource,
        sightings: Vec<Sighting<D>>,
    ) -> Result<(), NanDistance> {
        self.tracks.retain(Track::hit_counter_is_positive);
        for track in &mut self.tracks {
            track.step();
        }
        let leftovers = run_pass(
            &mut self.tracks,
            ids,
            self.distance_threshold,
            false,
            sightings,
        )?;
        let leftovers = run_pass(
            &mut self.tracks,
            ids,
            self.distance_threshold,
            true,
            leftovers,
        )?;
        for sighting in leftovers {
            let track = Track::new(&self.config, ids, &sighting.label, sighting.observation);
            self.tracks.push(track);
        }
        Ok(())
    }

    #[must_use]
    pub fn tracks(&self) -> &[Track<D>] {
        &self.tracks
    }

    pub fn active_tracks(&self) -> impl Iterator<Item = &Track<D>> {
        self.tracks.iter().filter(|track| track.is_active())
    }
}
