//! Renders the plain-text comparison report: counts, offset statistics and unmatched listings.

use std::collections::HashSet;
use std::fmt::Write as _;

use super::{Event, Track};

pub(super) fn render(events: &[Event], tracks: &[Track], pairs: &[(usize, usize)]) -> String {
    let matched = pairs.len();
    let mut report = String::from("compare-events report\nmatching is greedy and deterministic\n");
    let counts: [(&str, usize); 5] = [
        ("events read", events.len()),
        ("tracks read", tracks.len()),
        ("events matched", matched),
        ("events without a track", events.len() - matched),
        ("tracks without an event", tracks.len() - matched),
    ];
    for (name, value) in counts {
        let _ = writeln!(report, "{name} {value}");
    }
    let _ = writeln!(report, "events matched {matched}/{}", events.len());
    let _ = writeln!(report, "tracks matched {matched}/{}", tracks.len());
    let mut start_offsets: Vec<f64> = Vec::new();
    let mut end_offsets: Vec<f64> = Vec::new();
    let mut identical = 0_usize;
    let mut missed = 0_usize;
    let mut extra = 0_usize;
    for (event, track) in pairs.iter().filter_map(|&(event_index, track_index)| {
        Some((events.get(event_index)?, tracks.get(track_index)?))
    }) {
        start_offsets.push(track.start - event.start);
        end_offsets.extend(
            event
                .end
                .zip(track.end)
                .map(|(event_end, track_end)| track_end - event_end),
        );
        identical += usize::from(event.zones == track.zones);
        missed += usize::from(event.zones.iter().any(|zone| !set_has(&track.zones, zone)));
        extra += usize::from(track.zones.iter().any(|zone| !set_has(&event.zones, zone)));
    }
    let _ = writeln!(report, "{}", offsets_line("start", &mut start_offsets));
    let _ = writeln!(report, "{}", offsets_line("end", &mut end_offsets));
    let _ = writeln!(
        report,
        "zones identical {identical} missed {missed} extra {extra}"
    );
    let matched_events: HashSet<usize> = pairs.iter().map(|&(index, _)| index).collect();
    let matched_tracks: HashSet<usize> = pairs.iter().map(|&(_, index)| index).collect();
    let _ = writeln!(report, "unmatched events:");
    for (index, event) in events.iter().enumerate() {
        if matched_events.contains(&index) {
            continue;
        }
        let end = end_text(event.end);
        let zones = zones_text(&event.zones);
        let _ = writeln!(
            report,
            "{} {} {} {} {end} {zones}",
            event.id, event.camera, event.label, event.start
        );
    }
    let mut unmatched: Vec<&Track> = tracks
        .iter()
        .enumerate()
        .filter(|(index, _)| !matched_tracks.contains(index))
        .map(|(_, track)| track)
        .collect();
    unmatched.sort_by(|left, right| {
        left.record
            .cmp(&right.record)
            .then_with(|| left.start.total_cmp(&right.start))
            .then_with(|| left.id.cmp(&right.id))
    });
    let _ = writeln!(report, "unmatched tracks:");
    for track in unmatched {
        let end = end_text(track.end);
        let zones = zones_text(&track.zones);
        let _ = writeln!(
            report,
            "{} {} {} {} {} {end} {zones} frames={}..{}",
            track.path,
            track.camera,
            track.label,
            track.id,
            track.start,
            track.first_frame,
            track.last_frame
        );
    }
    report
}

fn offsets_line(name: &str, values: &mut [f64]) -> String {
    values.sort_by(f64::total_cmp);
    let shown = |value: Option<f64>| {
        value.map_or_else(|| String::from("n/a"), |number| format!("{number}"))
    };
    format!(
        "{name} offsets count {} median {} p95 {} min {} max {}",
        values.len(),
        shown(median(values)),
        shown(p95(values)),
        shown(values.first().copied()),
        shown(values.last().copied())
    )
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let half = values.len() / 2;
    if values.len() % 2 == 1 {
        return values.get(half).copied();
    }
    let low = values.get(half - 1).copied()?;
    let high = values.get(half).copied()?;
    Some(f64::midpoint(low, high))
}

// Nearest-rank p95: the value at 1-based rank ceil(95 * n / 100) of the sorted values.
fn p95(values: &[f64]) -> Option<f64> {
    let rank = (95 * values.len()).div_ceil(100);
    values.get(rank.checked_sub(1)?).copied()
}

fn set_has(set: &[String], zone: &str) -> bool {
    set.binary_search_by(|entry| entry.as_str().cmp(zone))
        .is_ok()
}

fn end_text(value: Option<f64>) -> String {
    value.map_or_else(|| String::from("open"), |value| format!("{value}"))
}

fn zones_text(zones: &[String]) -> String {
    if zones.is_empty() {
        return String::from("-");
    }
    zones.join(",")
}
