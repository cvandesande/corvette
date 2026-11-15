use corvette_motion::{Shape, fill_polygons};

// case line: case <kind> <index> <width> <height> <polygons joined by ;, or -> = <hex bits, 1 where covered>
const FIXTURE: &str = include_str!("fixtures/polygon_cases.txt");

struct Record {
    kind: String,
    index: usize,
    width: usize,
    height: usize,
    polygons: Vec<Vec<(f64, f64)>>,
    covered: Vec<bool>,
}

fn records() -> Vec<Record> {
    let mut all = Vec::new();
    for line in FIXTURE.lines().skip(1) {
        let fields: Vec<&str> = line.split(' ').collect();
        let width: usize = fields[3].parse().expect("a width");
        let height: usize = fields[4].parse().expect("a height");
        let mut polygons = Vec::new();
        if fields[5] != "-" {
            for string in fields[5].split(';') {
                let values: Vec<f64> = string
                    .split(',')
                    .map(|token| token.parse().expect("a fraction"))
                    .collect();
                let polygon = values
                    .chunks_exact(2)
                    .map(|pair| (pair[0], pair[1]))
                    .collect();
                polygons.push(polygon);
            }
        }
        let mut covered = Vec::with_capacity(width * height);
        for digit in fields[7].chars() {
            let nibble = digit.to_digit(16).expect("a hex digit");
            for bit in (0..4).rev() {
                covered.push(nibble & (1 << bit) != 0);
            }
        }
        covered.truncate(width * height);
        all.push(Record {
            kind: fields[1].to_string(),
            index: fields[2].parse().expect("an index"),
            width,
            height,
            polygons,
            covered,
        });
    }
    all
}

fn check_worked(name: &str, rows: &[&str], polygons: &[Vec<(f64, f64)>]) {
    let width = rows[0].chars().count();
    let want: Vec<u8> = rows
        .iter()
        .flat_map(|row| row.chars())
        .map(|c| if c == '#' { 0 } else { 255 })
        .collect();
    let shape = Shape::new(width, rows.len()).expect("a shape");
    let picture = fill_polygons(shape, polygons).expect("a picture");
    assert_eq!(picture.data(), want.as_slice(), "worked picture {name}");
}

#[test]
fn the_fill_matches_the_deployed_mask_for_every_case() {
    let header: Vec<&str> = FIXTURE
        .lines()
        .next()
        .expect("a header")
        .split(' ')
        .collect();
    let all = records();
    for pair in header[1..].chunks(2) {
        let want: usize = pair[1].parse().expect("a count");
        let read = all.iter().filter(|r| r.kind == pair[0]).count();
        assert_eq!(read, want, "header count for {}", pair[0]);
    }
    for record in &all {
        let shape = Shape::new(record.width, record.height).expect("a shape");
        let picture = fill_polygons(shape, &record.polygons).expect("a picture");
        assert_eq!(picture.shape().width(), record.width);
        assert_eq!(picture.shape().height(), record.height);
        for y in 0..record.height {
            for x in 0..record.width {
                let want: u8 = u8::from(!record.covered[y * record.width + x]) * 255;
                assert_eq!(
                    picture.data()[y * record.width + x],
                    want,
                    "case {} {} first differing pixel row {y} column {x}",
                    record.kind,
                    record.index
                );
            }
        }
    }
}

#[test]
fn the_three_deployed_cameras_have_the_measured_zero_counts() {
    let all = records();
    let cameras: Vec<&Record> = all.iter().filter(|r| r.kind == "camera").collect();
    assert_eq!(cameras.len(), 3);
    let wanted = [(704, 576, 4928usize), (704, 576, 7521), (640, 480, 5980)];
    for (record, (width, height, zeros)) in cameras.iter().zip(wanted) {
        assert_eq!((record.width, record.height), (width, height));
        let shape = Shape::new(width, height).expect("a shape");
        let picture = fill_polygons(shape, &record.polygons).expect("a picture");
        let mut found = 0usize;
        for &byte in picture.data() {
            found += usize::from(byte == 0);
        }
        assert_eq!(found, zeros);
    }
}

#[test]
fn the_worked_pictures_are_reproduced() {
    check_worked(
        "A",
        &[
            "........", "..#####.", "..#####.", "..#####.", "..#####.", "........",
        ],
        &[vec![(0.25, 0.25), (0.75, 0.25), (0.75, 0.75), (0.25, 0.75)]],
    );
    check_worked(
        "B",
        &[
            ".#########",
            "..#######.",
            "..#######.",
            "...#####..",
            "...#####..",
            "....###...",
            "....###...",
            ".....#....",
        ],
        &[vec![(0.1, 0.1), (0.9, 0.1), (0.5, 0.9)]],
    );
    check_worked(
        "C",
        &[
            ".....#####",
            ".....#####",
            ".....#####",
            ".....#####",
            ".....#####",
            "..........",
            "..........",
            "..........",
        ],
        &[vec![(0.5, 0.0), (0.5, 0.5), (1.0, 0.5), (1.0, 0.0)]],
    );
    check_worked(
        "D",
        &[
            "..........",
            "..#######.",
            "..#######.",
            "..#######.",
            "..########",
            "..########",
            "..########",
            ".....#####",
        ],
        &[
            vec![(0.2, 0.2), (0.8, 0.2), (0.8, 0.8), (0.2, 0.8)],
            vec![(0.5, 0.5), (1.0, 0.5), (1.0, 1.0), (0.5, 1.0)],
        ],
    );
    check_worked(
        "E",
        &["......", "......", "......", "......", "......"],
        &[],
    );
}

#[test]
fn an_input_outside_the_rules_is_refused() {
    let frame = Shape::new(10, 8).expect("a shape");
    for bad in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
        let polygons = [vec![(0.0, 0.0), (bad, 0.5), (1.0, 1.0)]];
        assert!(
            fill_polygons(frame, &polygons).is_none(),
            "coordinate {bad}"
        );
    }
    assert!(fill_polygons(frame, &[vec![(0.0, 0.0), (1.0, 1.0)]]).is_none());
    assert!(fill_polygons(frame, &[vec![]]).is_none());
    let wide = Shape::new(65_536, 1).expect("a wide shape");
    assert!(fill_polygons(wide, &[vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]]).is_none());
}

#[test]
fn the_limits_of_the_rules_are_accepted() {
    let frame = Shape::new(10, 8).expect("a shape");
    assert!(fill_polygons(frame, &[vec![(0.0, 0.0), (0.0, 0.0), (1.0, 1.0)]]).is_some());
    let edge = Shape::new(65_535, 1).expect("an edge shape");
    assert!(fill_polygons(edge, &[vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]]).is_some());
    let empty = fill_polygons(frame, &[]).expect("an all-255 picture");
    assert_eq!(empty.data().len(), 80);
    assert!(empty.data().iter().all(|&b| b == 255));
}
