// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and `clippy.toml` does not forbid them.
use corvette_motion::{Bounds, Margin, Region, Shape, calculate_region, calculate_region_f64};

const FLOAT_FIXTURE: &str = include_str!("fixtures/region_float_cases.txt");
const REGION_FIXTURE: &str = include_str!("fixtures/region_cases.txt");

fn parts(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

fn frame(p: &[&str]) -> Shape {
    Shape::new(
        p[1].parse().expect("a frame width"),
        p[2].parse().expect("a frame height"),
    )
    .expect("a frame shape")
}

fn floor_of(p: &[&str], at: usize) -> usize {
    p[at].parse().expect("a floor")
}

fn margin_of(word: &str) -> Margin {
    match word {
        "cluster" => Margin::Cluster,
        "exact" => Margin::Exact,
        _ => Margin::Motion,
    }
}

fn bounds(p: &[&str], at: usize) -> Bounds {
    Bounds {
        x1: p[at].parse().expect("a float bound"),
        y1: p[at + 1].parse().expect("a float bound"),
        x2: p[at + 2].parse().expect("a float bound"),
        y2: p[at + 3].parse().expect("a float bound"),
    }
}

fn square(p: &[&str], at: usize) -> Region {
    Region {
        x1: p[at].parse().expect("an edge"),
        y1: p[at + 1].parse().expect("an edge"),
        x2: p[at + 2].parse().expect("an edge"),
        y2: p[at + 3].parse().expect("an edge"),
    }
}

/// The four bound texts as `i32` edges when every one names a whole number, else `None`.
fn whole_bounds(texts: [&str; 4]) -> Option<Region> {
    let edges: Vec<i32> = texts
        .iter()
        .map(|text| text.strip_suffix(".0").unwrap_or(text).parse().ok())
        .collect::<Option<Vec<i32>>>()?;
    Some(Region {
        x1: edges[0],
        y1: edges[1],
        x2: edges[2],
        y2: edges[3],
    })
}

#[test]
fn calculate_region_f64_matches_frigate() {
    let lines: Vec<&str> = FLOAT_FIXTURE
        .lines()
        .filter(|line| line.starts_with("fregion "))
        .collect();
    assert!(
        lines.len() >= 800,
        "fixture carries {} fregion lines",
        lines.len()
    );
    for line in lines {
        let p = parts(line);
        let expected = Some(square(&p, 9));
        assert_eq!(
            calculate_region_f64(frame(&p), bounds(&p, 5), floor_of(&p, 3), margin_of(p[4])),
            expected,
            "fixture line: {line}"
        );
        if let Some(integer_bounds) = whole_bounds([p[5], p[6], p[7], p[8]]) {
            assert_eq!(
                calculate_region(frame(&p), integer_bounds, floor_of(&p, 3), margin_of(p[4])),
                expected,
                "the integer path on a whole-number line: {line}"
            );
        }
    }
}

#[test]
fn float_region_agrees_with_integer_region() {
    let mut checked = 0;
    for line in REGION_FIXTURE
        .lines()
        .filter(|line| line.starts_with("region "))
    {
        let p = parts(line);
        assert_eq!(
            calculate_region_f64(frame(&p), bounds(&p, 3), floor_of(&p, 7), margin_of(p[8])),
            Some(square(&p, 9)),
            "fixture line: {line}"
        );
        checked += 1;
    }
    assert!(
        checked >= 600,
        "checked {checked} region lines as float bounds"
    );
}

#[test]
fn region_float_fixture_reaches_the_rules() {
    let mut floor_decided = 0;
    let mut left = 0;
    let mut top = 0;
    let mut right = 0;
    let mut bottom = 0;
    let mut over = 0;
    let mut inverted = 0;
    let mut fractional = 0;
    let mut cluster = 0;
    let mut motion = 0;
    let mut exact = 0;
    for line in FLOAT_FIXTURE
        .lines()
        .filter(|line| line.starts_with("fregion "))
    {
        let p = parts(line);
        let (fw, fh) = (
            p[1].parse::<i64>().expect("a frame width"),
            p[2].parse::<i64>().expect("a frame height"),
        );
        let floor = p[3].parse::<i64>().expect("a floor");
        let box_edges = bounds(&p, 5);
        let out = square(&p, 9);
        let side = i64::from(out.x2 - out.x1);
        match p[4] {
            "cluster" => cluster += 1,
            "motion" => motion += 1,
            "exact" => exact += 1,
            other => panic!("an unknown margin word {other}"),
        }
        if side == floor {
            floor_decided += 1;
        }
        if out.x1 == 0 {
            left += 1;
        }
        if out.y1 == 0 {
            top += 1;
        }
        if i64::from(out.x2) == fw {
            right += 1;
        }
        if i64::from(out.y2) == fh {
            bottom += 1;
        }
        if side > fw || side > fh {
            over += 1;
        }
        if box_edges.x1 > box_edges.x2 || box_edges.y1 > box_edges.y2 {
            inverted += 1;
        }
        let fractions = [box_edges.x1, box_edges.y1, box_edges.x2, box_edges.y2];
        if fractions.iter().any(|value| value.fract() != 0.0) {
            fractional += 1;
        }
    }
    assert!(
        floor_decided >= 100,
        "the floor decided the side on {floor_decided} lines"
    );
    assert!(left >= 100, "the region's x1 was 0 on {left} lines");
    assert!(top >= 100, "the region's y1 was 0 on {top} lines");
    assert!(
        right >= 40,
        "the region ended at the frame width on {right} lines"
    );
    assert!(
        bottom >= 40,
        "the region ended at the frame height on {bottom} lines"
    );
    assert!(over >= 10, "the side exceeded the frame on {over} lines");
    assert!(
        inverted >= 20,
        "the fixture carries {inverted} inverted boxes"
    );
    assert!(
        fractional >= 300,
        "{fractional} lines carry a fractional bound"
    );
    assert!(motion >= 250, "{motion} lines use the motion margin");
    assert!(exact >= 250, "{exact} lines use the exact margin");
    assert!(cluster >= 160, "{cluster} lines use the cluster margin");
}

#[test]
fn out_of_domain_inputs_return_none() {
    let frame = Shape::new(640, 480).expect("a frame shape");
    let bounds = Bounds {
        x1: 0.0,
        y1: 0.0,
        x2: 100.0,
        y2: 100.0,
    };
    for bad in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1_048_576.5,
        -1_048_576.5,
    ] {
        for slot in 0..4 {
            let mut offside = bounds;
            match slot {
                0 => offside.x1 = bad,
                1 => offside.y1 = bad,
                2 => offside.x2 = bad,
                _ => offside.y2 = bad,
            }
            assert_eq!(
                calculate_region_f64(frame, offside, 320, Margin::Cluster),
                None,
                "a bound of {bad} in position {slot}"
            );
        }
    }
    let wide = Shape::new(1_048_577, 480).expect("a wide frame shape");
    assert_eq!(
        calculate_region_f64(wide, bounds, 320, Margin::Motion),
        None
    );
    let tall = Shape::new(640, 1_048_577).expect("a tall frame shape");
    assert_eq!(calculate_region_f64(tall, bounds, 320, Margin::Exact), None);
    assert_eq!(
        calculate_region_f64(frame, bounds, 1_048_577, Margin::Cluster),
        None
    );
}

#[test]
fn domain_limit_inputs_return_some() {
    let frame = Shape::new(1_048_576, 1_048_576).expect("a domain-edge frame shape");
    let bounds = Bounds {
        x1: -1_048_576.0,
        y1: -1_048_576.0,
        x2: 1_048_576.0,
        y2: 1_048_576.0,
    };
    for margin in [Margin::Cluster, Margin::Exact, Margin::Motion] {
        assert!(calculate_region_f64(frame, bounds, 1_048_576, margin).is_some());
    }
}

#[test]
fn exact_margin_keeps_the_longest_edge() {
    let frame = Shape::new(1920, 1080).expect("a frame shape");
    let startup = Bounds {
        x1: 433.6,
        y1: 201.1,
        x2: 1246.4,
        y2: 1013.9,
    };
    assert_eq!(
        calculate_region_f64(frame, startup, 320, Margin::Exact),
        Some(Region {
            x1: 434,
            y1: 201,
            x2: 1246,
            y2: 1013,
        })
    );
    let small = Shape::new(640, 480).expect("a frame shape");
    let tight = Bounds {
        x1: 100.0,
        y1: 100.0,
        x2: 399.0,
        y2: 399.0,
    };
    assert_eq!(
        calculate_region_f64(small, tight, 4, Margin::Exact),
        Some(Region {
            x1: 101,
            y1: 101,
            x2: 397,
            y2: 397,
        })
    );
}

#[test]
fn odd_floor_keeps_the_half_size() {
    let frame = Shape::new(640, 480).expect("a frame shape");
    let whole = Bounds {
        x1: 100.0,
        y1: 100.0,
        x2: 100.0,
        y2: 100.0,
    };
    let quarter_steps = Bounds {
        x1: 100.5,
        y1: 200.5,
        x2: 103.5,
        y2: 203.5,
    };
    let rows = [
        (5_usize, whole, (97, 97, 102, 102)),
        (5, quarter_steps, (99, 199, 104, 204)),
        (7_usize, whole, (96, 96, 103, 103)),
        (7, quarter_steps, (98, 198, 105, 205)),
        (9_usize, whole, (95, 95, 104, 104)),
        (9, quarter_steps, (97, 197, 106, 206)),
    ];
    let margins = [Margin::Cluster, Margin::Motion, Margin::Exact];
    for (floor, bounds, (x1, y1, x2, y2)) in rows {
        let want = Some(Region { x1, y1, x2, y2 });
        for margin in margins {
            assert_eq!(
                calculate_region_f64(frame, bounds, floor, margin),
                want,
                "floor {floor} with margin {margin:?}"
            );
        }
        if bounds == whole {
            let integer_box = Region {
                x1: 100,
                y1: 100,
                x2: 100,
                y2: 100,
            };
            for margin in margins {
                assert_eq!(
                    calculate_region(frame, integer_box, floor, margin),
                    want,
                    "the integer path on floor {floor} with margin {margin:?}"
                );
            }
        }
    }
}
