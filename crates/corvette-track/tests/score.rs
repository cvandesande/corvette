use corvette_track::Scores;
use std::collections::BTreeMap;

const FIXTURE: &str = include_str!("fixtures/score_cases.txt");

const HAND_ROWS: &str = r"scenario one-seen-score-below-threshold threshold 0.7 initial 0
step seen 0.5 computed 0.5 top 0.5 confirmed 0 history 1 0.5
end
scenario one-seen-score-at-threshold-confirms threshold 0.7 initial 0
step seen 0.7 computed 0.7 top 0.7 confirmed 1 history 1 0.7
end
scenario one-ulp-below-threshold-stays-unconfirmed threshold 0.7 initial 0
step seen 0.6999999999999998 computed 0.6999999999999998 top 0.6999999999999998 confirmed 0 history 1 0.6999999999999998
end
scenario one-ulp-above-threshold-confirms threshold 0.7 initial 0
step seen 0.7000000000000001 computed 0.7000000000000001 top 0.7000000000000001 confirmed 1 history 1 0.7000000000000001
end
scenario even-count-averages-the-two-middle threshold 0.7 initial 1 0.9
step seen 0.5 computed 0.7 top 0.7 confirmed 1 history 2 0.9 0.5
end
scenario intermittent-detections-keep-it-unconfirmed threshold 0.7 initial 4 0.6 0.62 0.58 0.0
step predicted - computed 0.58 top 0.58 confirmed 0 history 5 0.6 0.62 0.58 0.0 0.0
end
scenario confirmed-stays-confirmed-when-the-median-falls threshold 0.7 initial 3 0.8 0.8 0.8
step seen 0.9 computed 0.8 top 0.8 confirmed 1 history 4 0.8 0.8 0.8 0.9
step predicted - computed 0.8 top 0.8 confirmed 1 history 5 0.8 0.8 0.8 0.9 0.0
step predicted - computed 0.8 top 0.8 confirmed 1 history 6 0.8 0.8 0.8 0.9 0.0 0.0
step predicted - computed 0.8 top 0.8 confirmed 1 history 7 0.8 0.8 0.8 0.9 0.0 0.0 0.0
step predicted - computed 0.4 top 0.8 confirmed 1 history 8 0.8 0.8 0.8 0.9 0.0 0.0 0.0 0.0
step predicted - computed 0.0 top 0.8 confirmed 1 history 9 0.8 0.8 0.8 0.9 0.0 0.0 0.0 0.0 0.0
step predicted - computed 0.0 top 0.8 confirmed 1 history 10 0.8 0.8 0.8 0.9 0.0 0.0 0.0 0.0 0.0 0.0
step predicted - computed 0.0 top 0.8 confirmed 1 history 10 0.8 0.8 0.9 0.0 0.0 0.0 0.0 0.0 0.0 0.0
step predicted - computed 0.0 top 0.8 confirmed 1 history 10 0.8 0.9 0.0 0.0 0.0 0.0 0.0 0.0 0.0 0.0
end
scenario history-keeps-the-last-ten threshold 0.7 initial 4 0.1 0.2 0.3 0.4
step seen 0.9 computed 0.3 top 0.3 confirmed 0 history 5 0.1 0.2 0.3 0.4 0.9
step seen 0.9 computed 0.35 top 0.35 confirmed 0 history 6 0.1 0.2 0.3 0.4 0.9 0.9
step seen 0.9 computed 0.4 top 0.4 confirmed 0 history 7 0.1 0.2 0.3 0.4 0.9 0.9 0.9
step seen 0.9 computed 0.65 top 0.65 confirmed 0 history 8 0.1 0.2 0.3 0.4 0.9 0.9 0.9 0.9
step seen 0.9 computed 0.9 top 0.9 confirmed 1 history 9 0.1 0.2 0.3 0.4 0.9 0.9 0.9 0.9 0.9
step seen 0.9 computed 0.9 top 0.9 confirmed 1 history 10 0.1 0.2 0.3 0.4 0.9 0.9 0.9 0.9 0.9 0.9
step seen 0.9 computed 0.9 top 0.9 confirmed 1 history 10 0.2 0.3 0.4 0.9 0.9 0.9 0.9 0.9 0.9 0.9
step seen 0.9 computed 0.9 top 0.9 confirmed 1 history 10 0.3 0.4 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9
step seen 0.9 computed 0.9 top 0.9 confirmed 1 history 10 0.4 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9
step seen 0.9 computed 0.9 top 0.9 confirmed 1 history 10 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9
step seen 0.9 computed 0.9 top 0.9 confirmed 1 history 10 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9
step seen 0.9 computed 0.9 top 0.9 confirmed 1 history 10 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9 0.9
end
scenario initial-history-longer-than-ten-is-trimmed-at-the-first-update threshold 0.7 initial 12 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1
step seen 0.9 computed 0.1 top 0.1 confirmed 0 history 10 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.1 0.9
end
scenario top-score-never-falls threshold 0.9 initial 1 0.95
step seen 0.95 computed 0.95 top 0.95 confirmed 1 history 2 0.95 0.95
step predicted - computed 0.95 top 0.95 confirmed 1 history 3 0.95 0.95 0.0
step predicted - computed 0.475 top 0.95 confirmed 1 history 4 0.95 0.95 0.0 0.0
step seen 0.2 computed 0.2 top 0.95 confirmed 1 history 5 0.95 0.95 0.0 0.0 0.2
end
scenario only-predicted-frames-give-zero threshold 0.7 initial 1 0.9
step predicted - computed 0.45 top 0.45 confirmed 0 history 2 0.9 0.0
step predicted - computed 0.0 top 0.45 confirmed 0 history 3 0.9 0.0 0.0
step predicted - computed 0.0 top 0.45 confirmed 0 history 4 0.9 0.0 0.0 0.0
step predicted - computed 0.0 top 0.45 confirmed 0 history 5 0.9 0.0 0.0 0.0 0.0
step predicted - computed 0.0 top 0.45 confirmed 0 history 6 0.9 0.0 0.0 0.0 0.0 0.0
end
scenario threshold-zero-confirms-on-the-first-update threshold 0.0 initial 0
step predicted - computed 0.0 top 0.0 confirmed 1 history 1 0.0
end
scenario threshold-above-one-never-confirms threshold 1.5 initial 2 1.0 1.0
step seen 1.0 computed 1.0 top 1.0 confirmed 0 history 3 1.0 1.0 1.0
step seen 1.0 computed 1.0 top 1.0 confirmed 0 history 4 1.0 1.0 1.0 1.0
step seen 1.0 computed 1.0 top 1.0 confirmed 0 history 5 1.0 1.0 1.0 1.0 1.0
end
";

#[derive(Debug)]
struct StepRow {
    predicted: bool,
    score_bits: Option<u64>,
    computed_bits: u64,
    top_bits: u64,
    confirmed: bool,
    history_len: usize,
    history_bits: Vec<u64>,
}

#[derive(Debug)]
struct Scenario {
    name: String,
    threshold: f64,
    initial: Vec<f64>,
    steps: Vec<StepRow>,
}

fn broken(tag: &str, line_number: usize, detail: &str) -> ! {
    panic!("{tag} line {line_number}: {detail}");
}

fn as_f64(text: &str, tag: &str, line_number: usize, what: &str) -> f64 {
    text.parse()
        .unwrap_or_else(|_| broken(tag, line_number, what))
}

fn as_usize(text: &str, tag: &str, line_number: usize, what: &str) -> usize {
    text.parse()
        .unwrap_or_else(|_| broken(tag, line_number, what))
}

fn parse_rows(tag: &str, text: &str) -> Vec<Scenario> {
    let mut scenarios: Vec<Scenario> = Vec::new();
    let mut current: Option<Scenario> = None;
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split(' ').collect();
        match fields[0] {
            "scenario" => {
                if fields[2] != "threshold" || fields[4] != "initial" || fields.len() < 6 {
                    broken(tag, line_number, "scenario fields");
                }
                let count = as_usize(fields[5], tag, line_number, "initial count");
                if fields.len() != 6 + count {
                    broken(tag, line_number, "scenario initial values");
                }
                current = Some(Scenario {
                    name: fields[1].to_string(),
                    threshold: as_f64(fields[3], tag, line_number, "threshold"),
                    initial: fields[6..]
                        .iter()
                        .map(|text| as_f64(text, tag, line_number, "initial score"))
                        .collect(),
                    steps: Vec::new(),
                });
            }
            "step" => {
                if fields[3] != "computed"
                    || fields[5] != "top"
                    || fields[7] != "confirmed"
                    || fields[9] != "history"
                    || fields.len() < 11
                {
                    broken(tag, line_number, "step fields");
                }
                let length = as_usize(fields[10], tag, line_number, "history length");
                if fields.len() != 11 + length {
                    broken(tag, line_number, "step history values");
                }
                let score = match fields[1] {
                    "seen" => Some(as_f64(fields[2], tag, line_number, "score")),
                    "predicted" => {
                        if fields[2] != "-" {
                            broken(tag, line_number, "predicted step with a score");
                        }
                        None
                    }
                    other => broken(tag, line_number, &format!("unknown step kind {other}")),
                };
                let Some(scenario) = current.as_mut() else {
                    broken(tag, line_number, "step without scenario");
                };
                scenario.steps.push(StepRow {
                    predicted: score.is_none(),
                    score_bits: score.map(f64::to_bits),
                    computed_bits: as_f64(fields[4], tag, line_number, "computed").to_bits(),
                    top_bits: as_f64(fields[6], tag, line_number, "top").to_bits(),
                    confirmed: fields[8] == "1",
                    history_len: length,
                    history_bits: fields[11..]
                        .iter()
                        .map(|text| as_f64(text, tag, line_number, "history value").to_bits())
                        .collect(),
                });
            }
            "end" => {
                let Some(owned) = current.take() else {
                    broken(tag, line_number, "end without scenario");
                };
                scenarios.push(owned);
            }
            other => broken(tag, line_number, &format!("unknown line kind {other}")),
        }
    }
    scenarios
}

fn replay(scenario: &Scenario, tag: &str) {
    let mut scores = Scores::new(&scenario.initial);
    for (index, step) in scenario.steps.iter().enumerate() {
        let number = index + 1;
        scores.update(step.score_bits.map(f64::from_bits), scenario.threshold);
        assert_eq!(
            scores.computed().to_bits(),
            step.computed_bits,
            "{tag} scenario {}: step {number}: computed",
            scenario.name
        );
        assert_eq!(
            scores.top().to_bits(),
            step.top_bits,
            "{tag} scenario {}: step {number}: top",
            scenario.name
        );
        assert_eq!(
            scores.confirmed(),
            step.confirmed,
            "{tag} scenario {}: step {number}: confirmed",
            scenario.name
        );
        assert_eq!(
            scores.history().len(),
            step.history_len,
            "{tag} scenario {}: step {number}: history length",
            scenario.name
        );
        for (slot, value) in scores.history().iter().enumerate() {
            assert_eq!(
                value.to_bits(),
                step.history_bits[slot],
                "{tag} scenario {}: step {number}: history slot {slot}",
                scenario.name
            );
        }
    }
}

fn header_map(text: &str, marker: &str) -> BTreeMap<String, u64> {
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(marker) {
            let tokens: Vec<&str> = rest.split_whitespace().collect();
            let mut map = BTreeMap::new();
            let mut index = 0;
            while index + 1 < tokens.len() {
                map.insert(
                    tokens[index].to_string(),
                    tokens[index + 1].parse().expect("header number"),
                );
                index += 2;
            }
            return map;
        }
    }
    panic!("fixture header has no {marker} line");
}

fn bump(stats: &mut BTreeMap<String, u64>, key: &str) {
    *stats.entry(key.to_string()).or_insert(0) += 1;
}

const fn next_toward_zero(value: f64) -> f64 {
    let bits = value.to_bits();
    if bits == 0 || bits == 0x8000_0000_0000_0000 {
        return 0.0;
    }
    if bits & 0x8000_0000_0000_0000 == 0 {
        f64::from_bits(bits - 1)
    } else {
        f64::from_bits(bits + 1)
    }
}

fn recompute(scenarios: &[Scenario]) -> BTreeMap<String, u64> {
    let mut stats: BTreeMap<String, u64> = BTreeMap::new();
    for scenario in scenarios {
        bump(&mut stats, "scenarios");
        match scenario.initial.len() {
            0 => bump(&mut stats, "initial_zero"),
            1 => bump(&mut stats, "initial_one"),
            4 => bump(&mut stats, "initial_four"),
            10 => bump(&mut stats, "initial_ten"),
            12 => bump(&mut stats, "initial_twelve"),
            _ => {}
        }
        if scenario.threshold.to_bits() == 0
            || scenario.threshold.to_bits() == 0x8000_0000_0000_0000
        {
            bump(&mut stats, "threshold_zero");
        }
        if scenario.threshold > 1.0 {
            bump(&mut stats, "threshold_above_one");
        }
        let threshold_bits = scenario.threshold.to_bits();
        let ulp_bits = next_toward_zero(scenario.threshold).to_bits();
        let mut was_confirmed = false;
        let mut ever_confirmed = false;
        for (index, step) in scenario.steps.iter().enumerate() {
            bump(&mut stats, "steps");
            if step.predicted {
                bump(&mut stats, "predicted_steps");
            }
            if step.history_len == 10 {
                bump(&mut stats, "full_history");
            }
            if step.history_len % 2 == 0 {
                bump(&mut stats, "even_history");
            } else {
                bump(&mut stats, "odd_history");
            }
            if step.computed_bits == threshold_bits {
                bump(&mut stats, "equal_threshold");
            }
            if step.computed_bits == ulp_bits {
                bump(&mut stats, "ulp_below");
            }
            if step.confirmed && !was_confirmed {
                bump(&mut stats, "flips");
            }
            was_confirmed = step.confirmed;
            if step.confirmed {
                ever_confirmed = true;
                if index == 0 {
                    bump(&mut stats, "first_step_confirm");
                }
                if f64::from_bits(step.computed_bits) < scenario.threshold {
                    bump(&mut stats, "sticky_steps");
                }
            }
            if f64::from_bits(step.top_bits) > f64::from_bits(step.computed_bits) {
                bump(&mut stats, "top_above_computed");
            }
        }
        if !ever_confirmed {
            bump(&mut stats, "never_confirm");
        }
    }
    stats
}

const STATED_FLOORS: [(&str, u64); 20] = [
    ("scenarios", 300),
    ("steps", 6000),
    ("predicted_steps", 1500),
    ("full_history", 3000),
    ("even_history", 2000),
    ("odd_history", 500),
    ("equal_threshold", 100),
    ("ulp_below", 10),
    ("flips", 120),
    ("never_confirm", 30),
    ("first_step_confirm", 20),
    ("sticky_steps", 300),
    ("top_above_computed", 1000),
    ("initial_zero", 25),
    ("initial_one", 25),
    ("initial_four", 25),
    ("initial_ten", 10),
    ("initial_twelve", 10),
    ("threshold_zero", 10),
    ("threshold_above_one", 10),
];

#[test]
fn fixture_matches_the_real_library() {
    let scenarios = parse_rows("fixture", FIXTURE);
    let counts: BTreeMap<String, u64> = header_map(FIXTURE, "# counts ");
    assert_eq!(scenarios.len(), 300, "fixture scenario count");
    assert_eq!(
        u64::try_from(scenarios.len()).expect("scenario count"),
        counts.get("scenarios").copied().expect("header scenarios"),
        "header scenario count disagrees"
    );
    let total: u64 = scenarios
        .iter()
        .map(|scenario| u64::try_from(scenario.steps.len()).expect("step count"))
        .sum();
    assert_eq!(
        total,
        counts.get("steps").copied().expect("header steps"),
        "header step count disagrees"
    );
    for scenario in &scenarios {
        replay(scenario, "fixture");
    }
}

#[test]
fn fixture_floors_are_met_by_the_parsed_rows() {
    let scenarios = parse_rows("fixture", FIXTURE);
    let stated: BTreeMap<String, u64> = header_map(FIXTURE, "# floors ");
    for (name, floor) in STATED_FLOORS {
        assert_eq!(stated.get(name), Some(&floor), "fixture floors {name}");
    }
    let declared: BTreeMap<String, u64> = header_map(FIXTURE, "# counts ");
    let recomputed = recompute(&scenarios);
    for (name, value) in &declared {
        assert_eq!(
            recomputed.get(name),
            Some(value),
            "fixture counts {name} disagrees with the parsed rows"
        );
    }
    for (name, floor) in STATED_FLOORS {
        assert!(
            recomputed.get(name).copied().unwrap_or(0) >= floor,
            "fixture recomputed {name} below its floor"
        );
    }
}

#[test]
fn hand_scenarios_match_the_real_library() {
    let scenarios = parse_rows("hand rows", HAND_ROWS);
    assert_eq!(scenarios.len(), 13, "hand scenario count");
    let total: usize = scenarios.iter().map(|scenario| scenario.steps.len()).sum();
    assert_eq!(total, 41, "hand step count");
    for scenario in &scenarios {
        replay(scenario, "hand rows");
    }
}

#[test]
fn new_state_before_any_update() {
    let empty = Scores::new(&[]);
    assert_eq!(
        empty.computed().to_bits(),
        0.0f64.to_bits(),
        "empty computed"
    );
    assert_eq!(empty.top().to_bits(), 0.0f64.to_bits(), "empty top");
    assert!(!empty.confirmed(), "empty starts unconfirmed");
    assert!(empty.history().is_empty(), "empty history");
    let initial = [0.2; 12];
    let started = Scores::new(&initial);
    assert_eq!(
        started.computed().to_bits(),
        0.0f64.to_bits(),
        "starting computed"
    );
    assert_eq!(started.top().to_bits(), 0.0f64.to_bits(), "starting top");
    assert!(!started.confirmed(), "starting unconfirmed");
    assert_eq!(
        started.history().to_vec(),
        initial.to_vec(),
        "starting history kept untrimmed"
    );
}

#[test]
fn update_never_panics_on_extreme_input() {
    let extremes: [f64; 8] = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        5e-324,
        -0.0,
    ];
    let mut scores = Scores::new(&[]);
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    for index in 0..2000 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let drawn = state >> 33;
        let score = if drawn.is_multiple_of(16) {
            None
        } else {
            Some(extremes[(drawn % 8) as usize])
        };
        scores.update(score, extremes[((drawn >> 7) % 8) as usize]);
        assert!(
            scores.history().len() <= 10,
            "extreme update {index} grew the history past ten"
        );
    }
}
