use corvette_track::{Zone, ZoneSet, polygon_from_relative};
use std::collections::BTreeMap;

const FIXTURE: &str = include_str!("fixtures/zones_cases.txt");

const HAND_ROWS: &str = r"scenario flicker-reset-needs-the-full-count frame 200 200
zone z0 inertia 3 labels 0 poly 4 0,0 100,0 100,100 0,100
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 0
frame 1 label person x 150 y 50 edge 0 excl 0 diff 0 cur 0 ent 0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 1 z0 ent 1 z0
end
scenario re-entry-needs-the-whole-inertia-again frame 200 200
zone z0 inertia 3 labels 0 poly 4 0,0 100,0 100,100 0,100
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 1 z0 ent 1 z0
frame 1 label person x 150 y 50 edge 0 excl 0 diff 0 cur 0 ent 1 z0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 1 z0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 1 z0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 1 z0 ent 1 z0
end
scenario entered-zone-stays-in-entered-after-leaving frame 200 200
zone z0 inertia 2 labels 0 poly 4 0,0 100,0 100,100 0,100
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 1 z0 ent 1 z0
frame 1 label person x 150 y 50 edge 0 excl 0 diff 0 cur 0 ent 1 z0
end
scenario entered-list-keeps-every-zone-ever-entered frame 400 200
zone z0 inertia 1 labels 0 poly 4 0,0 100,0 100,100 0,100
zone z1 inertia 1 labels 0 poly 4 200,0 300,0 300,100 200,100
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 1 z0 ent 1 z0
frame 1 label person x 250 y 50 edge 0 excl 0 diff 0 cur 1 z1 ent 2 z0 z1
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 1 z0 ent 2 z0 z1
end
scenario membership-uses-the-box-bottom-not-the-centre frame 200 200
zone z0 inertia 1 labels 0 poly 4 0,90 100,90 100,100 0,100
frame 1 label person x 50 y 95 edge 0 excl 0 diff 0 cur 1 z0 ent 1 z0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 1 z0
end
scenario on-vertex-point-counts-as-inside frame 200 200
zone z0 inertia 1 labels 0 poly 4 10,10 60,10 60,40 10,40
frame 1 label person x 10 y 10 edge 0 excl 0 diff 0 cur 1 z0 ent 1 z0
end
scenario on-edge-midpoint-counts-as-inside frame 200 200
zone z0 inertia 1 labels 0 poly 4 10,10 60,10 60,40 10,40
frame 1 label person x 35 y 10 edge 0 excl 0 diff 0 cur 1 z0 ent 1 z0
end
scenario label-list-excludes-other-labels-from-entry frame 200 200
zone z0 inertia 1 labels 1 car poly 4 0,0 100,0 100,100 0,100
zone z1 inertia 1 labels 0 poly 4 0,0 100,0 100,100 0,100
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 1 z1 ent 1 z1
end
scenario empty-label-list-admits-any-label frame 200 200
zone z0 inertia 2 labels 0 poly 4 0,0 100,0 100,100 0,100
frame 1 label truck x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 0
frame 1 label truck x 50 y 50 edge 0 excl 0 diff 0 cur 1 z0 ent 1 z0
end
scenario forget-restarts-the-count-and-the-entered-list frame 200 200
zone z0 inertia 2 labels 0 poly 4 0,0 100,0 100,100 0,100
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 1 z0 ent 1 z0
forget 1
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 0 ent 0
frame 1 label person x 50 y 50 edge 0 excl 0 diff 0 cur 1 z0 ent 1 z0
end
";

struct Spec {
    name: String,
    inertia: u32,
    labels: Vec<String>,
    relative: Vec<[f64; 2]>,
    polygon: Vec<[i32; 2]>,
}

enum Event {
    Frame {
        id: u64,
        label: String,
        x: i32,
        y: i32,
        edge: bool,
        excl: bool,
        diff: bool,
        cur: Vec<String>,
        ent: Vec<String>,
    },
    Forget(u64),
}

struct Scenario {
    width: u32,
    height: u32,
    zones: Vec<Spec>,
    events: Vec<Event>,
}

fn broken(tag: &str, line_number: usize, detail: &str) -> ! {
    panic!("{tag} line {line_number}: {detail}");
}

fn as_number<T: std::str::FromStr>(text: &str, tag: &str, line_number: usize, what: &str) -> T
where
    T::Err: std::fmt::Display,
{
    text.parse()
        .unwrap_or_else(|error| broken(tag, line_number, &format!("{what}: {error}")))
}

fn as_bool(text: &str, tag: &str, line_number: usize, what: &str) -> bool {
    match text {
        "0" => false,
        "1" => true,
        _ => broken(tag, line_number, &format!("{what}: not 0 or 1")),
    }
}

fn parse_point(text: &str, tag: &str, line_number: usize) -> [i32; 2] {
    let parts: Vec<&str> = text.split(',').collect();
    if parts.len() != 2 {
        broken(tag, line_number, "point wants x,y");
    }
    [
        as_number(parts[0], tag, line_number, "point x"),
        as_number(parts[1], tag, line_number, "point y"),
    ]
}

fn parse_hex_pair(text: &str, tag: &str, line_number: usize) -> [f64; 2] {
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() != 2 {
        broken(tag, line_number, "relative point wants hex:hex");
    }
    let mut pair = [0.0; 2];
    for (slot, part) in pair.iter_mut().zip(parts) {
        let bits = u64::from_str_radix(part, 16)
            .unwrap_or_else(|error| broken(tag, line_number, &format!("float bits: {error}")));
        *slot = f64::from_bits(bits);
    }
    pair
}

fn parse_zone(tag: &str, line_number: usize, f: &[&str]) -> Spec {
    if f.len() < 7 || f[2] != "inertia" || f[4] != "labels" {
        broken(tag, line_number, "zone fields");
    }
    let label_count: usize = as_number(f[5], tag, line_number, "label count");
    let tail = 6 + label_count;
    if f.len() < tail + 2 || f[tail] != "poly" && f[tail] != "rel" {
        broken(tag, line_number, "zone tail");
    }
    let count: usize = as_number(f[tail + 1], tag, line_number, "point count");
    let (relative, polygon) = if f[tail] == "poly" {
        if f.len() != tail + 2 + count {
            broken(tag, line_number, "zone point values");
        }
        let polygon: Vec<[i32; 2]> = f[tail + 2..]
            .iter()
            .map(|text| parse_point(text, tag, line_number))
            .collect();
        (Vec::new(), polygon)
    } else {
        if f.len() < tail + 4 + count || f[tail + 2 + count] != "pixel" {
            broken(tag, line_number, "relative zone values");
        }
        let pixel_count: usize = as_number(f[tail + 3 + count], tag, line_number, "pixel count");
        if f.len() != tail + 4 + count + pixel_count {
            broken(tag, line_number, "pixel zone values");
        }
        let relative: Vec<[f64; 2]> = f[tail + 2..tail + 2 + count]
            .iter()
            .map(|text| parse_hex_pair(text, tag, line_number))
            .collect();
        let polygon: Vec<[i32; 2]> = f[tail + 4 + count..]
            .iter()
            .map(|text| parse_point(text, tag, line_number))
            .collect();
        (relative, polygon)
    };
    Spec {
        name: f[1].to_string(),
        inertia: as_number(f[3], tag, line_number, "inertia"),
        labels: f[6..tail].iter().map(ToString::to_string).collect(),
        relative,
        polygon,
    }
}

fn parse_frame(tag: &str, line_number: usize, f: &[&str]) -> Event {
    if f.len() < 17
        || f[2] != "label"
        || f[4] != "x"
        || f[6] != "y"
        || f[8] != "edge"
        || f[10] != "excl"
        || f[12] != "diff"
        || f[14] != "cur"
    {
        broken(tag, line_number, "frame fields");
    }
    let cur_count: usize = as_number(f[15], tag, line_number, "current count");
    if f.len() < 18 + cur_count || f[16 + cur_count] != "ent" {
        broken(tag, line_number, "frame current values");
    }
    let ent_count: usize = as_number(f[17 + cur_count], tag, line_number, "entered count");
    if f.len() != 18 + cur_count + ent_count {
        broken(tag, line_number, "frame entered values");
    }
    Event::Frame {
        id: as_number(f[1], tag, line_number, "object id"),
        label: f[3].to_string(),
        x: as_number(f[5], tag, line_number, "x"),
        y: as_number(f[7], tag, line_number, "y"),
        edge: as_bool(f[9], tag, line_number, "edge flag"),
        excl: as_bool(f[11], tag, line_number, "excl flag"),
        diff: as_bool(f[13], tag, line_number, "diff flag"),
        cur: f[16..16 + cur_count]
            .iter()
            .map(ToString::to_string)
            .collect(),
        ent: f[18 + cur_count..18 + cur_count + ent_count]
            .iter()
            .map(ToString::to_string)
            .collect(),
    }
}

fn parse_rows(tag: &str, text: &str) -> Vec<Scenario> {
    let mut scenarios: Vec<Scenario> = Vec::new();
    let mut current: Option<(u32, u32, Vec<Spec>, Vec<Event>)> = None;
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split(' ').collect();
        match f.first().copied() {
            Some("scenario") => {
                if f.len() != 5 || f[2] != "frame" {
                    broken(tag, line_number, "scenario wants frame w h");
                }
                current = Some((
                    as_number(f[3], tag, line_number, "frame width"),
                    as_number(f[4], tag, line_number, "frame height"),
                    Vec::new(),
                    Vec::new(),
                ));
            }
            Some("zone") => {
                let spec = parse_zone(tag, line_number, &f);
                if let Some((_, _, zones, _)) = current.as_mut() {
                    zones.push(spec);
                } else {
                    broken(tag, line_number, "zone without a scenario");
                }
            }
            Some("frame") => {
                let event = parse_frame(tag, line_number, &f);
                if let Some((_, _, _, events)) = current.as_mut() {
                    events.push(event);
                } else {
                    broken(tag, line_number, "frame without a scenario");
                }
            }
            Some("forget") => {
                if f.len() != 2 {
                    broken(tag, line_number, "forget fields");
                }
                if let Some((_, _, _, events)) = current.as_mut() {
                    events.push(Event::Forget(as_number(
                        f[1],
                        tag,
                        line_number,
                        "object id",
                    )));
                } else {
                    broken(tag, line_number, "forget without a scenario");
                }
            }
            Some("end") => {
                if f.len() != 1 {
                    broken(tag, line_number, "end fields");
                }
                let Some((width, height, zones, events)) = current.take() else {
                    broken(tag, line_number, "end without a scenario");
                };
                scenarios.push(Scenario {
                    width,
                    height,
                    zones,
                    events,
                });
            }
            _ => broken(tag, line_number, "unknown line kind"),
        }
    }
    if current.is_some() {
        broken(tag, 0, "trailing scenario without an end");
    }
    scenarios
}

fn build_zone_set(scenario: &Scenario, tag: &str) -> ZoneSet {
    let mut zones = Vec::new();
    for spec in &scenario.zones {
        if !spec.relative.is_empty() {
            let computed = polygon_from_relative(&spec.relative, scenario.width, scenario.height)
                .unwrap_or_else(|| broken(tag, 0, "relative polygon rejected"));
            assert_eq!(
                computed, spec.polygon,
                "{tag}: relative polygon must match the recorded pixels"
            );
        }
        zones.push(Zone {
            name: spec.name.clone(),
            polygon: spec.polygon.clone(),
            labels: spec.labels.clone(),
            inertia: spec.inertia,
        });
    }
    ZoneSet::new(zones)
}

fn replay_scenario(index: usize, scenario: &Scenario) {
    let tag = format!("scenario {index}");
    let mut zone_set = build_zone_set(scenario, &tag);
    for event in &scenario.events {
        match event {
            Event::Frame {
                id,
                label,
                x,
                y,
                cur,
                ent,
                ..
            } => {
                let state = zone_set.observe(*id, label, *x, *y);
                assert_eq!(
                    &state.current, cur,
                    "{tag}: current zones at point {x},{y} with label {label}"
                );
                assert_eq!(
                    &state.entered, ent,
                    "{tag}: entered zones at point {x},{y} with label {label}"
                );
            }
            Event::Forget(id) => zone_set.forget(*id),
        }
    }
}

fn header_map(marker: &str) -> BTreeMap<String, u64> {
    let mut map = BTreeMap::new();
    for line in FIXTURE.lines() {
        let Some(rest) = line.strip_prefix(marker) else {
            continue;
        };
        let tokens: Vec<&str> = rest.split(' ').collect();
        for pair in tokens.chunks(2) {
            assert_eq!(pair.len(), 2, "header {marker} has an odd number of fields");
            let value: u64 = pair[1].parse().expect("header number");
            map.insert(pair[0].to_string(), value);
        }
    }
    map
}

#[derive(Default)]
struct Counts {
    scenarios: u64,
    frames: u64,
    on_edge: u64,
    disagreement: u64,
    label_excluded: u64,
    forgets: u64,
}

fn count_rows(scenarios: &[Scenario]) -> Counts {
    let mut counts = Counts::default();
    for scenario in scenarios {
        counts.scenarios += 1;
        for event in &scenario.events {
            match event {
                Event::Frame {
                    edge, excl, diff, ..
                } => {
                    counts.frames += 1;
                    counts.on_edge += u64::from(*edge);
                    counts.disagreement += u64::from(*diff);
                    counts.label_excluded += u64::from(*excl);
                }
                Event::Forget(_) => counts.forgets += 1,
            }
        }
    }
    counts
}

const REQUIRED_FLOORS: [(&str, u64); 5] = [
    ("scenarios", 120),
    ("on_edge", 40),
    ("disagreement", 40),
    ("label_excluded", 40),
    ("forgets", 40),
];

#[test]
fn fixture_rows_replay_frame_by_frame() {
    let scenarios = parse_rows("fixture", FIXTURE);
    assert!(!scenarios.is_empty(), "fixture has no scenarios");
    for (index, scenario) in scenarios.iter().enumerate() {
        replay_scenario(index + 1, scenario);
    }
}

#[test]
fn fixture_counts_match_the_header_and_the_floors() {
    let counts = count_rows(&parse_rows("fixture", FIXTURE));
    let stated = header_map("# counts ");
    let floors = header_map("# floors ");
    let recomputed: [(&str, u64); 6] = [
        ("scenarios", counts.scenarios),
        ("frames", counts.frames),
        ("forgets", counts.forgets),
        ("on_edge", counts.on_edge),
        ("disagreement", counts.disagreement),
        ("label_excluded", counts.label_excluded),
    ];
    for (name, value) in recomputed {
        assert_eq!(
            stated.get(name).or_else(|| floors.get(name)),
            Some(&value),
            "recomputed {name}"
        );
    }
    for (name, floor) in REQUIRED_FLOORS {
        assert!(
            floors.get(name) >= Some(&floor),
            "floor {name} below {floor}"
        );
        assert!(
            stated.get(name).or_else(|| floors.get(name)) >= Some(&floor),
            "count {name} below {floor}"
        );
    }
}

#[test]
fn hand_written_scenarios_replay() {
    let scenarios = parse_rows("hand", HAND_ROWS);
    assert!(
        scenarios.len() >= 6,
        "hand rows carry fewer than six scenarios"
    );
    for (index, scenario) in scenarios.iter().enumerate() {
        replay_scenario(index + 1, scenario);
    }
}

#[test]
fn polygon_from_relative_rounds_like_the_reference() {
    assert_eq!(
        polygon_from_relative(&[[0.0, 0.0], [1.0, 1.0], [0.5, 0.25]], 101, 51),
        Some(vec![[0, 0], [101, 51], [50, 12]])
    );
    assert_eq!(
        polygon_from_relative(&[[0.5, 0.5], [1.0, 0.0], [0.0, 1.0]], 3, 2),
        Some(vec![[1, 1], [3, 0], [0, 2]])
    );
    assert_eq!(
        polygon_from_relative(&[[0.0, 0.0], [1.0, 1.0]], 64, 48),
        None
    );
    assert_eq!(polygon_from_relative(&[], 64, 48), None);
}

#[test]
fn empty_zone_set_observes_without_entries() {
    let mut zone_set = ZoneSet::new(Vec::new());
    let state = zone_set.observe(7, "person", 10, 20);
    assert!(state.current.is_empty());
    assert!(state.entered.is_empty());
}

#[test]
fn extreme_coordinates_do_not_overflow() {
    let zone = Zone {
        name: "z0".to_string(),
        polygon: vec![
            [i32::MIN, i32::MIN],
            [i32::MAX, i32::MIN],
            [i32::MIN, i32::MAX],
        ],
        labels: Vec::new(),
        inertia: 1,
    };
    let mut zone_set = ZoneSet::new(vec![zone]);
    let inside = zone_set.observe(1, "person", -1_000_000_000, -1_500_000_000);
    assert_eq!(inside.current, vec!["z0".to_string()]);
    let on_edge = zone_set.observe(2, "person", 1_000_000_000, i32::MIN);
    assert_eq!(on_edge.current, vec!["z0".to_string()]);
    let outside = zone_set.observe(3, "person", 1_000_000_000, 1_500_000_000);
    assert!(outside.current.is_empty());
}
