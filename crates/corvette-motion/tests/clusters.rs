// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and `clippy.toml` does not forbid them.
use corvette_motion::{Region, Shape, box_area, box_inside, cluster_boundary, cluster_candidates};

const FIXTURE: &str = include_str!("fixtures/cluster_cases.txt");

fn fixture_lines(kind: &str) -> Vec<&'static str> {
    FIXTURE
        .lines()
        .filter(|line| line.starts_with(kind))
        .collect()
}

fn parts(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

fn edge(text: &str) -> i32 {
    text.parse().expect("an edge")
}

fn square(p: &[&str], at: usize) -> Region {
    Region {
        x1: edge(p[at]),
        y1: edge(p[at + 1]),
        x2: edge(p[at + 2]),
        y2: edge(p[at + 3]),
    }
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

fn boxes_of(text: &str) -> Vec<Region> {
    if text == "-" {
        return Vec::new();
    }
    text.split(';').map(square_of_group).collect()
}

fn square_of_group(group: &str) -> Region {
    let four: Vec<i32> = group
        .split(',')
        .map(|value| value.parse().expect("an edge"))
        .collect();
    Region {
        x1: four[0],
        y1: four[1],
        x2: four[2],
        y2: four[3],
    }
}

fn clusters_of(text: &str) -> Vec<Vec<usize>> {
    if text == "-" {
        return Vec::new();
    }
    text.split(';')
        .map(|group| {
            group
                .split(',')
                .map(|value| value.parse().expect("an index"))
                .collect()
        })
        .collect()
}

const fn bump(condition: bool, counter: &mut usize) {
    if condition {
        *counter += 1;
    }
}

#[test]
fn box_inside_matches_frigate() {
    let lines = fixture_lines("inside");
    assert!(
        lines.len() >= 250,
        "fixture carries {} inside lines",
        lines.len()
    );
    for line in lines {
        let p = parts(line);
        let expected = p[9] == "1";
        assert_eq!(
            box_inside(square(&p, 1), square(&p, 5)),
            expected,
            "fixture line: {line}"
        );
    }
}

#[test]
fn box_area_matches_frigate() {
    let lines = fixture_lines("area");
    assert!(
        lines.len() >= 150,
        "fixture carries {} area lines",
        lines.len()
    );
    for line in lines {
        let p = parts(line);
        let want: i64 = p[5].parse().expect("an area");
        assert_eq!(box_area(square(&p, 1)), Some(want), "fixture line: {line}");
    }
}

#[test]
fn cluster_boundary_matches_frigate() {
    let lines = fixture_lines("boundary");
    assert!(
        lines.len() >= 450,
        "fixture carries {} boundary lines",
        lines.len()
    );
    for line in lines {
        let p = parts(line);
        assert_eq!(
            cluster_boundary(square(&p, 1), floor_of(&p, 5)),
            Some(square(&p, 6)),
            "fixture line: {line}"
        );
    }
}

#[test]
fn cluster_candidates_match_frigate() {
    let lines = fixture_lines("candidates");
    assert!(
        lines.len() >= 350,
        "fixture carries {} candidates lines",
        lines.len()
    );
    for line in lines {
        let p = parts(line);
        let floor = floor_of(&p, 3);
        let boxes = boxes_of(p[4]);
        assert_eq!(
            cluster_candidates(frame(&p), floor, &boxes),
            Some(clusters_of(p[5])),
            "fixture line: {line}"
        );
    }
}

#[test]
fn cluster_fixture_reaches_the_rules() {
    let mut merged = 0;
    let mut split = 0;
    let mut denied = 0;
    for line in fixture_lines("candidates") {
        let p = parts(line);
        let floor = floor_of(&p, 3);
        let boxes = boxes_of(p[4]);
        let clusters = clusters_of(p[5]);
        split += usize::from(clusters.len() >= 2);
        merged += usize::from(clusters.iter().any(|cluster| cluster.len() >= 2));
        let mut cluster_of = vec![usize::MAX; boxes.len()];
        for (position, cluster) in clusters.iter().enumerate() {
            for &member in cluster {
                cluster_of[member] = position;
            }
        }
        let mut found = false;
        for first in 0..boxes.len() {
            for second in 0..boxes.len() {
                if first == second || cluster_of[first] == cluster_of[second] {
                    continue;
                }
                let boundary =
                    cluster_boundary(boxes[first], floor).expect("an in-domain boundary");
                if box_inside(boundary, boxes[second]) {
                    found = true;
                }
            }
        }
        bump(found, &mut denied);
    }
    let mut half_negative = 0;
    let mut inverted = 0;
    for line in fixture_lines("boundary") {
        let p = parts(line);
        let source = square(&p, 1);
        let result = square(&p, 6);
        let x_odd = (source.x2 - source.x1).abs() % 2 == 1;
        let y_odd = (source.y2 - source.y1).abs() % 2 == 1;
        let below = result.x1 < 0 || result.y1 < 0 || result.x2 < 0 || result.y2 < 0;
        bump((x_odd || y_odd) && below, &mut half_negative);
        bump(
            source.x1 > source.x2 || source.y1 > source.y2,
            &mut inverted,
        );
    }
    assert!(
        merged >= 150,
        "{merged} lines hold a cluster of two or more members"
    );
    assert!(split >= 100, "{split} lines hold two or more clusters");
    assert!(
        denied >= 40,
        "{denied} lines hold a boundary-admitted pair kept in different clusters"
    );
    assert!(
        half_negative >= 60,
        "{half_negative} boundary lines have a negative odd-span half-integer edge"
    );
    assert!(
        inverted >= 40,
        "{inverted} boundary lines have an inverted input box"
    );
}

#[test]
fn out_of_domain_inputs_return_none() {
    let good = Region {
        x1: 0,
        y1: 0,
        x2: 10,
        y2: 10,
    };
    let shape = Shape::new(640, 480).expect("a frame shape");
    for limit in [1_048_577_i32, -1_048_577_i32] {
        for edge_position in 0..4 {
            let mut box_at_limit = good;
            match edge_position {
                0 => box_at_limit.x1 = limit,
                1 => box_at_limit.y1 = limit,
                2 => box_at_limit.x2 = limit,
                _ => box_at_limit.y2 = limit,
            }
            assert_eq!(box_area(box_at_limit), None);
            assert_eq!(cluster_boundary(box_at_limit, 320), None);
            assert_eq!(cluster_candidates(shape, 320, &[box_at_limit]), None);
        }
    }
    for over in [1_048_577_usize, 9_223_372_036_854_775_808] {
        assert_eq!(cluster_boundary(good, over), None);
        assert_eq!(cluster_candidates(shape, over, &[]), None);
        assert_eq!(cluster_candidates(shape, over, &[good]), None);
    }
    let wide = Shape::new(1_048_577, 480).expect("a wide frame shape");
    let tall = Shape::new(640, 1_048_577).expect("a tall frame shape");
    assert_eq!(cluster_candidates(wide, 320, &[]), None);
    assert_eq!(cluster_candidates(tall, 320, &[]), None);
}

#[test]
fn domain_limit_inputs_return_some() {
    let shape = Shape::new(1_048_576, 1_048_576).expect("a domain-edge frame shape");
    let edge_box = Region {
        x1: -1_048_576,
        y1: 0,
        x2: 1_048_576,
        y2: 1,
    };
    let tall = Region {
        x1: 0,
        y1: -1_048_576,
        x2: 1,
        y2: 1_048_576,
    };
    let inside = Region {
        x1: 100,
        y1: 100,
        x2: 180,
        y2: 200,
    };
    assert!(box_area(edge_box).is_some());
    assert!(box_area(tall).is_some());
    assert!(cluster_boundary(edge_box, 1_048_576).is_some());
    assert!(cluster_boundary(tall, 1_048_576).is_some());
    assert!(cluster_candidates(shape, 1_048_576, &[edge_box, tall, inside]).is_some());
    assert!(cluster_candidates(shape, 320, &[edge_box, tall]).is_some());
}
