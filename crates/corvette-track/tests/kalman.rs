use std::collections::BTreeMap;

use corvette_track::Kalman;

const FIXTURE: &str = include_str!("fixtures/kalman_cases.txt");
const NAMED_PAIRS: [(f64, f64); 6] = [
    (3.4, 0.1),
    (3.4, 0.03),
    (2.5, 0.05),
    (4.5, 0.25),
    (4.0, 0.2),
    (4.0, 0.1),
];
const NAMED_NAMES: [&str; 6] = [
    "named_a", "named_b", "named_c", "named_d", "named_e", "named_f",
];

fn from_hex(text: &str) -> f64 {
    let bits = u64::from_str_radix(text, 16).expect("a bit pattern");
    f64::from_bits(bits)
}

const fn bits_of(value: f64) -> u64 {
    value.to_bits()
}

fn hex_values(text: &str, count: usize) -> Vec<f64> {
    let values: Vec<f64> = text.split(',').map(from_hex).collect();
    assert_eq!(values.len(), count, "expected {count} values in {text}");
    values
}

const fn is_integer_value(value: f64) -> bool {
    value.is_finite() && value.to_bits() == value.trunc().to_bits()
}

fn count_entry(counts: &mut BTreeMap<&'static str, usize>, name: &'static str) {
    *counts.entry(name).or_insert(0) += 1;
}

fn check_row(
    r: f64,
    q: f64,
    init: [f64; 4],
    ops: &[Option<[f64; 4]>],
    states: &[&str],
    cov: [&str; 3],
) {
    assert_eq!(ops.len(), states.len());
    let mut kalman = Kalman::new(init, r, q);
    for (index, op) in ops.iter().enumerate() {
        let kind = match op {
            None => "predict",
            Some(_) => "update",
        };
        match op {
            None => kalman.predict(),
            Some(measurement) => kalman.update(*measurement),
        }
        let want = hex_values(states[index], 8);
        let position = kalman.position();
        let velocity = kalman.velocity();
        for coordinate in 0..4 {
            assert_eq!(
                position[coordinate].to_bits(),
                want[coordinate].to_bits(),
                "op {index} ({kind}) coordinate {coordinate} position"
            );
            assert_eq!(
                velocity[coordinate].to_bits(),
                want[coordinate + 4].to_bits(),
                "op {index} ({kind}) coordinate {coordinate} velocity"
            );
        }
    }
    let arrays = [
        kalman.position_variance(),
        kalman.position_velocity_covariance(),
        kalman.velocity_variance(),
    ];
    for (which, array) in arrays.iter().enumerate() {
        for (coordinate, value) in array.iter().enumerate() {
            assert_eq!(
                value.to_bits(),
                from_hex(cov[which]).to_bits(),
                "covariance {which} coordinate {coordinate}"
            );
        }
    }
}

#[test]
fn hand_rows_match_the_real_filter() {
    check_row(
        3.4,
        0.1,
        [100.0, 100.0, 200.0, 200.0],
        &[
            None,
            Some([104.0, 100.0, 204.0, 200.0]),
            None,
            Some([108.0, 100.0, 208.0, 200.0]),
        ],
        &[
            "4059000000000000,4059000000000000,4069000000000000,4069000000000000,0000000000000000,0000000000000000,0000000000000000,0000000000000000",
            "4059c3f8f01c3f8f,4059000000000000,406961fc780e1fc8,4069000000000000,3fd1a7b9611a7b96,0000000000000000,3fd1a7b9611a7b96,0000000000000000",
            "4059d5a0a97d5a0b,4059000000000000,40696ad054bead06,4069000000000000,3fd1a7b9611a7b96,0000000000000000,3fd1a7b9611a7b96,0000000000000000",
            "405a7a90e7d95bc6,4059000000000000,4069bd4873ecade3,4069000000000000,3ff0d4873ecade2c,0000000000000000,3ff0d4873ecade22,0000000000000000",
        ],
        ["3ffe126dd4c050d8", "3fe21c3cef47b5b1", "3fed73c6a1d935b9"],
    );
    check_row(
        3.4,
        0.03,
        [10.0, 20.0, 60.0, 90.0],
        &[
            None,
            Some([12.0, 23.0, 63.0, 95.0]),
            None,
            None,
            None,
            Some([20.0, 40.0, 70.0, 110.0]),
        ],
        &[
            "4024000000000000,4034000000000000,404e000000000000,4056800000000000,0000000000000000,0000000000000000,0000000000000000,0000000000000000",
            "40270eb991a6f0ec,40364b0b2d3d34b1,404f2585969e9a58,40577499fd842b4a,3fc1bda63b411bda,3fca9c7958e1a9c7,3fca9c7958e1a9c7,3fd62d0fca1162d0",
            "402755b02a93f55b,403680441feef805,404f40220ff77c02,40578ac70d4e3cad,3fc1bda63b411bda,3fca9c7958e1a9c7,3fca9c7958e1a9c7,3fd62d0fca1162d0",
            "40279ca6c380f9ca,4036b57d12a0bb59,404f5abe89505dac,4057a0f41d184e10,3fc1bda63b411bda,3fca9c7958e1a9c7,3fca9c7958e1a9c7,3fd62d0fca1162d0",
            "4027e39d5c6dfe39,4036eab605527ead,404f755b02a93f56,4057b7212ce25f73,3fc1bda63b411bda,3fca9c7958e1a9c7,3fca9c7958e1a9c7,3fd62d0fca1162d0",
            "4030543b3f2cadb7,40401b845f46afc9,4050b16a3fbe2a65,4059c6790f8955d3,3ff6e25c57055d4d,400793b8a9a9af76,3ff57fa3b1bcb8f2,4006310004610b1b",
        ],
        ["3ffd9bf4641ebf57", "3fe1721b3070ae50", "3fe9905feed04c0c"],
    );
}

#[test]
fn extreme_input_never_panics() {
    let extremes = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN_POSITIVE,
    ];
    let constants = [
        0.0,
        -1.0,
        3.4,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::MAX,
        f64::MIN_POSITIVE,
    ];
    for first in extremes {
        for second in extremes {
            for r in constants {
                for q in constants {
                    let mut kalman = Kalman::new([first, 0.0, second, 1.0], r, q);
                    kalman.predict();
                    kalman.update([second, first, 0.0, -1.0]);
                    kalman.predict();
                    kalman.update([1e9, -1e9, f64::MAX, -f64::MAX]);
                    assert_eq!(kalman.position().len(), 4);
                    assert_eq!(kalman.velocity().len(), 4);
                    assert_eq!(kalman.position_variance().len(), 4);
                    assert_eq!(kalman.position_velocity_covariance().len(), 4);
                    assert_eq!(kalman.velocity_variance().len(), 4);
                }
            }
        }
    }
    let mut plain = Kalman::new([1.0, 2.0, 3.0, 4.0], 3.4, 0.1);
    for _ in 0..1000 {
        plain.predict();
    }
    assert_eq!(
        plain.position().map(bits_of),
        [1.0_f64, 2.0, 3.0, 4.0].map(bits_of)
    );
    assert_eq!(plain.velocity().map(bits_of), [0.0_f64; 4].map(bits_of));
}

fn parse_op(token: &str, line: &str, line_no: usize) -> Option<[f64; 4]> {
    if token == "P" {
        return None;
    }
    let payload = token
        .strip_prefix("U:")
        .unwrap_or_else(|| panic!("line {line_no}: {line}"));
    let values = hex_values(payload, 4);
    let values: [f64; 4] = values.try_into().expect("a four-value measurement");
    Some(values)
}

fn replay_ops(
    kalman: &mut Kalman,
    ops: &[Option<[f64; 4]>],
    states: &[&str],
    line: &str,
    line_no: usize,
) {
    for (op_index, op) in ops.iter().enumerate() {
        let kind = match op {
            None => "predict",
            Some(_) => "update",
        };
        match op {
            None => kalman.predict(),
            Some(measurement) => kalman.update(*measurement),
        }
        let want = hex_values(states[op_index], 8);
        let position = kalman.position();
        let velocity = kalman.velocity();
        for coordinate in 0..4 {
            assert!(
                position[coordinate].to_bits() == want[coordinate].to_bits(),
                "line {line_no}: {line}\nop {op_index} ({kind}) coordinate {coordinate} position: got {:016x} want {:016x}",
                position[coordinate].to_bits(),
                want[coordinate].to_bits()
            );
            assert!(
                velocity[coordinate].to_bits() == want[coordinate + 4].to_bits(),
                "line {line_no}: {line}\nop {op_index} ({kind}) coordinate {coordinate} velocity: got {:016x} want {:016x}",
                velocity[coordinate].to_bits(),
                want[coordinate + 4].to_bits()
            );
        }
    }
}

fn compare_covariances(kalman: &Kalman, cov: &[f64], line: &str, line_no: usize) {
    let arrays = [
        kalman.position_variance(),
        kalman.position_velocity_covariance(),
        kalman.velocity_variance(),
    ];
    for (which, array) in arrays.iter().enumerate() {
        for coordinate in 0..4 {
            let want = cov[which * 4 + coordinate];
            assert!(
                array[coordinate].to_bits() == want.to_bits(),
                "line {line_no}: {line}\nfinal covariance {which} coordinate {coordinate}: got {:016x} want {:016x}",
                array[coordinate].to_bits(),
                want.to_bits()
            );
        }
    }
}

fn count_case(
    ops: &[Option<[f64; 4]>],
    init: [f64; 4],
    r: f64,
    q: f64,
    parsed: &mut BTreeMap<&'static str, usize>,
) {
    if ops.iter().any(Option::is_none) && ops.iter().any(Option::is_some) {
        count_entry(parsed, "both_pu");
    }
    let mut run = 0;
    let mut longest = 0;
    for op in ops {
        if op.is_none() {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    if longest >= 3 {
        count_entry(parsed, "run3p");
    }
    let mut big = false;
    let mut non_integer = false;
    let mut corner_values = init.to_vec();
    for measurement in ops.iter().filter_map(|op| *op) {
        corner_values.extend_from_slice(&measurement);
    }
    for value in corner_values {
        if is_integer_value(value) {
            if value.abs() >= 1_048_576.0 {
                big = true;
            }
        } else {
            non_integer = true;
        }
    }
    if non_integer {
        count_entry(parsed, "noninteger");
    }
    if big {
        count_entry(parsed, "bigint");
    }
    for (named_index, (named_r, named_q)) in NAMED_PAIRS.iter().enumerate() {
        if r.to_bits() == named_r.to_bits() && q.to_bits() == named_q.to_bits() {
            count_entry(parsed, NAMED_NAMES[named_index]);
        }
    }
}

fn replay_case(line: &str, line_no: usize, parsed: &mut BTreeMap<&'static str, usize>) {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(
        tokens.first().copied(),
        Some("kalman"),
        "line {line_no}: {line}"
    );
    assert_eq!(tokens[2], "r", "line {line_no}: {line}");
    assert_eq!(tokens[4], "q", "line {line_no}: {line}");
    assert_eq!(tokens[6], "init", "line {line_no}: {line}");
    assert_eq!(tokens[11], "ops", "line {line_no}: {line}");
    let r = from_hex(tokens[3]);
    let q = from_hex(tokens[5]);
    let init: [f64; 4] = tokens[7..11]
        .iter()
        .copied()
        .map(from_hex)
        .collect::<Vec<f64>>()
        .try_into()
        .expect("four creation values");
    let arrow = tokens
        .iter()
        .position(|token| *token == "=>")
        .unwrap_or_else(|| panic!("line {line_no}: no arrow: {line}"));
    let cov_at = tokens
        .iter()
        .position(|token| *token == "cov")
        .unwrap_or_else(|| panic!("line {line_no}: no cov: {line}"));
    let ops: Vec<Option<[f64; 4]>> = tokens[12..arrow]
        .iter()
        .map(|token| parse_op(token, line, line_no))
        .collect();
    let states = &tokens[arrow + 1..cov_at];
    assert_eq!(states.len(), ops.len(), "line {line_no}: {line}");
    let cov = hex_values(tokens[cov_at + 1], 12);
    let mut kalman = Kalman::new(init, r, q);
    replay_ops(&mut kalman, &ops, states, line, line_no);
    compare_covariances(&kalman, &cov, line, line_no);
    count_case(&ops, init, r, q, parsed);
}

fn parse_header_line<'a>(line: &'a str, key: &str, into: &mut BTreeMap<&'a str, usize>) {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() < 2 || tokens[1] != key {
        return;
    }
    let mut at = 2;
    while at + 1 < tokens.len() {
        into.insert(tokens[at], tokens[at + 1].parse().expect("a header count"));
        at += 2;
    }
}

#[test]
fn fixture_matches_numpy_bit_for_bit() {
    let mut header: BTreeMap<&str, usize> = BTreeMap::new();
    let mut floors: BTreeMap<&str, usize> = BTreeMap::new();
    let mut parsed: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut cases = 0;
    for (index, line) in FIXTURE.lines().enumerate() {
        if line.starts_with('#') {
            parse_header_line(line, "counts", &mut header);
            parse_header_line(line, "floors", &mut floors);
            continue;
        }
        replay_case(line, index + 1, &mut parsed);
        cases += 1;
    }
    assert_eq!(cases, 1200);
    assert_eq!(header["cases"], 1200);
    assert_eq!(header["nonfinite_excluded"], 0);
    for (name, want) in [
        ("tracking", 350),
        ("jumps", 250),
        ("gaps", 250),
        ("extreme", 150),
        ("floats", 200),
    ] {
        assert_eq!(header[name], want, "header {name}");
    }
    assert_eq!(floors["named_each"], 50);
    for name in NAMED_NAMES {
        assert!(header[name] >= floors["named_each"], "{name} floor");
        assert_eq!(header[name], parsed[name], "header vs parsed {name}");
    }
    for (name, floor) in [
        ("both_pu", 1000),
        ("run3p", 300),
        ("noninteger", 200),
        ("bigint", 150),
    ] {
        assert_eq!(floors[name], floor, "floors {name}");
        assert!(header[name] >= floor, "{name} floor");
        assert_eq!(header[name], parsed[name], "header vs parsed {name}");
    }
}
