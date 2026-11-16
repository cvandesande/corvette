// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and `clippy.toml` does not forbid them.
use corvette_motion::{Margin, Region, Shape, calculate_region, cluster_region, min_region_size};

const FIXTURE: &str = include_str!("fixtures/region_cases.txt");

fn fixture_lines(kind: &str) -> Vec<&'static str> {
    FIXTURE
        .lines()
        .filter(|line| line.starts_with(kind))
        .collect()
}

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
        _ => Margin::Motion,
    }
}

fn square_flat(values: impl Iterator<Item = &'static str>) -> Region {
    let four: Vec<i32> = values.map(|v| v.parse().expect("an edge")).collect();
    Region {
        x1: four[0],
        y1: four[1],
        x2: four[2],
        y2: four[3],
    }
}

fn square(p: &[&'static str], at: usize) -> Region {
    square_flat(p[at..at + 4].iter().copied())
}

fn edge(text: &str) -> i32 {
    text.parse().expect("an edge")
}

fn count(text: &str) -> i64 {
    text.parse().expect("a small count")
}

const fn bump(condition: bool, counter: &mut usize) {
    if condition {
        *counter += 1;
    }
}

#[test]
fn min_region_size_matches_frigate() {
    let lines = fixture_lines("min");
    assert!(
        lines.len() >= 300,
        "fixture carries {} min lines",
        lines.len()
    );
    for line in lines {
        let p = parts(line);
        let model = Shape::new(
            p[1].parse().expect("a model width"),
            p[2].parse().expect("a model height"),
        )
        .expect("a model shape");
        assert_eq!(
            min_region_size(model),
            p[3].parse().expect("a region size"),
            "line {line}"
        );
    }
}

#[test]
fn calculate_region_matches_frigate() {
    let lines = fixture_lines("region");
    assert!(
        lines.len() >= 600,
        "fixture carries {} region lines",
        lines.len()
    );
    for line in lines {
        let p = parts(line);
        assert_eq!(
            calculate_region(frame(&p), square(&p, 3), floor_of(&p, 7), margin_of(p[8])),
            Some(square(&p, 9)),
            "fixture line: {line}"
        );
    }
}

#[test]
fn cluster_region_matches_frigate() {
    let lines = fixture_lines("cluster");
    assert!(
        lines.len() >= 200,
        "fixture carries {} cluster lines",
        lines.len()
    );
    for line in lines {
        let p = parts(line);
        let cluster: Vec<usize> = if p[4] == "-" {
            Vec::new()
        } else {
            p[4].split(',')
                .map(|v| v.parse().expect("an index"))
                .collect()
        };
        let boxes: Vec<Region> = if p[5] == "-" {
            Vec::new()
        } else {
            p[5].split(';')
                .map(|group| square_flat(group.split(',')))
                .collect()
        };
        assert_eq!(
            cluster_region(frame(&p), floor_of(&p, 3), &cluster, &boxes),
            Some(square(&p, 6)),
            "fixture line: {line}"
        );
    }
}

#[test]
fn region_fixture_reaches_the_clamps() {
    let mut floor_decided = 0;
    let mut left = 0;
    let mut right = 0;
    let mut top = 0;
    let mut bottom = 0;
    let mut over = 0;
    let mut saw_cluster = false;
    let mut saw_motion = false;
    for line in fixture_lines("region") {
        let p = parts(line);
        let (fw, fh) = (count(p[1]), count(p[2]));
        let (x1, y1) = (edge(p[3]), edge(p[4]));
        let floor = count(p[7]);
        let out = square(&p, 9);
        saw_cluster |= p[8] == "cluster";
        saw_motion |= p[8] == "motion";
        bump(i64::from(out.x2 - out.x1) == floor, &mut floor_decided);
        bump(out.x1 == 0 && x1 > 0, &mut left);
        bump(i64::from(out.x2) == fw, &mut right);
        bump(out.y1 == 0 && y1 > 0, &mut top);
        bump(i64::from(out.y2) == fh, &mut bottom);
        bump(i64::from(out.x2) > fw || i64::from(out.y2) > fh, &mut over);
    }
    assert!(
        floor_decided >= 20,
        "the floor decided the size on {floor_decided} lines"
    );
    assert!(left >= 20, "the left clamp fired on {left} lines");
    assert!(right >= 20, "the right clamp fired on {right} lines");
    assert!(top >= 20, "the top clamp fired on {top} lines");
    assert!(bottom >= 20, "the bottom clamp fired on {bottom} lines");
    assert!(over >= 20, "the result hung over the frame on {over} lines");
    assert!(
        saw_cluster && saw_motion,
        "both margins must appear in the fixture"
    );
}

#[test]
fn out_of_domain_inputs_return_none() {
    let frame = Shape::new(640, 480).expect("a frame shape");
    let bounds = square_flat(["0", "0", "100", "100"].iter().copied());
    for limit in [1_048_577, -1_048_577] {
        let mut far = bounds;
        far.x1 = limit;
        assert_eq!(calculate_region(frame, far, 320, Margin::Cluster), None);
        far.y2 = limit;
        assert_eq!(calculate_region(frame, far, 320, Margin::Cluster), None);
    }
    let wide = Shape::new(1_048_577, 480).expect("a wide frame shape");
    assert_eq!(calculate_region(wide, bounds, 320, Margin::Motion), None);
    let tall = Shape::new(640, 1_048_577).expect("a tall frame shape");
    assert_eq!(calculate_region(tall, bounds, 320, Margin::Cluster), None);
    assert_eq!(
        calculate_region(frame, bounds, 1_048_577, Margin::Cluster),
        None
    );
    assert_eq!(cluster_region(frame, 320, &[0], &[]), None);
    assert_eq!(cluster_region(frame, 320, &[3], &[bounds]), None);
}

#[test]
fn region_fixture_has_inverted_and_empty_boxes() {
    let mut both = 0;
    let mut one = 0;
    let mut empty = 0;
    for line in fixture_lines("region") {
        let p = parts(line);
        let (x1, y1) = (edge(p[3]), edge(p[4]));
        let (x2, y2) = (edge(p[5]), edge(p[6]));
        bump(x2 < x1 && y2 < y1, &mut both);
        bump((x2 < x1) != (y2 < y1), &mut one);
        bump(x1 == x2 && y1 == y2, &mut empty);
    }
    assert!(
        both >= 30,
        "the fixture carries {both} lines with both axes inverted"
    );
    assert!(
        one >= 20,
        "the fixture carries {one} lines with exactly one axis inverted"
    );
    assert!(
        empty >= 10,
        "the fixture carries {empty} lines with a zero-size box"
    );
}

#[test]
fn domain_limit_inputs_return_some() {
    let frame = Shape::new(1_048_576, 1_048_576).expect("a domain-edge frame shape");
    let bounds = square_flat(
        ["-1048576", "-1048576", "1048576", "1048576"]
            .iter()
            .copied(),
    );
    let inside = square_flat(["-1048576", "0", "1048576", "1"].iter().copied());
    for margin in [Margin::Cluster, Margin::Motion] {
        assert!(calculate_region(frame, bounds, 1_048_576, margin).is_some());
        assert!(calculate_region(frame, inside, 4, margin).is_some());
        assert!(cluster_region(frame, 320, &[0, 1], &[bounds, inside]).is_some());
    }
}
