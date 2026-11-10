use corvette_motion::{Contour, Plane, Shape, external_contours};

// record line: contours kind width height level runs count then count groups of x y width height twice_area
const FIXTURE: &str = include_str!("fixtures/contours_cases.txt");

struct Record {
    kind: String,
    index: usize,
    width: usize,
    height: usize,
    level: u8,
    runs: String,
    contours: Vec<Contour>,
}

fn records() -> Vec<Record> {
    let mut all = Vec::new();
    let mut kind = String::new();
    let mut index = 0usize;
    for line in FIXTURE.lines().skip(1) {
        let f: Vec<&str> = line.split(' ').collect();
        if f[1] != kind {
            kind = f[1].to_string();
            index = 0;
        }
        let width: usize = f[2].parse().expect("a width");
        let height: usize = f[3].parse().expect("a height");
        let level: u8 = f[4].parse().expect("a level");
        let runs = f[5].to_string();
        let count: usize = f[6].parse().expect("a count");
        let mut contours = Vec::with_capacity(count);
        for group in 0..count {
            let at = 7 + 5 * group;
            contours.push(Contour {
                x: f[at].parse().expect("an x"),
                y: f[at + 1].parse().expect("a y"),
                width: f[at + 2].parse().expect("a contour width"),
                height: f[at + 3].parse().expect("a contour height"),
                twice_area: f[at + 4].parse().expect("a twice area"),
            });
        }
        all.push(Record {
            kind: kind.clone(),
            index,
            width,
            height,
            level,
            runs,
            contours,
        });
        index += 1;
    }
    all
}

fn decode(record: &Record) -> Vec<u8> {
    let mut picture = Vec::with_capacity(record.width * record.height);
    let mut foreground = false;
    for run in record.runs.split(',') {
        let length: usize = run.parse().expect("a run length");
        picture.resize(
            picture.len() + length,
            if foreground { record.level } else { 0 },
        );
        foreground = !foreground;
    }
    picture
}

fn contours_at(rows: &[&str], level: u8) -> Vec<Contour> {
    let width = rows[0].chars().count();
    let picture: Vec<u8> = rows
        .iter()
        .flat_map(|row| row.chars().map(|c| u8::from(c == '#') * level))
        .collect();
    let shape = Shape::new(width, rows.len()).expect("a shape");
    external_contours(Plane::new(&picture, shape).expect("a plane"))
}

fn contours(rows: &[&str]) -> Vec<Contour> {
    contours_at(rows, 255)
}

const fn contour(x: usize, y: usize, width: usize, height: usize, twice_area: u64) -> Contour {
    Contour {
        x,
        y,
        width,
        height,
        twice_area,
    }
}

#[test]
fn the_contours_match_the_reference_for_every_record() {
    let header: Vec<&str> = FIXTURE
        .lines()
        .next()
        .expect("a header")
        .split(' ')
        .collect();
    let all = records();
    let mut rest = &all[..];
    for pair in header[1..].chunks_exact(2) {
        let (block, tail) = rest.split_at(pair[1].parse::<usize>().expect("a count"));
        assert!(
            block.iter().all(|record| record.kind == pair[0]),
            "records after the header block of kind {}",
            pair[0]
        );
        assert_eq!(
            block.len(),
            all.iter().filter(|record| record.kind == pair[0]).count(),
            "the header count for kind {}",
            pair[0]
        );
        rest = tail;
    }
    assert!(rest.is_empty(), "records after the last header kind");
    for record in &all {
        let picture = decode(record);
        let shape = Shape::new(record.width, record.height).expect("a shape");
        let plane = Plane::new(&picture, shape).expect("a plane");
        let found = external_contours(plane);
        assert_eq!(
            found.len(),
            record.contours.len(),
            "{} record {}: contour count, want {:?}, got {:?}",
            record.kind,
            record.index,
            record.contours,
            found
        );
        for (number, (got, want)) in found.iter().zip(&record.contours).enumerate() {
            assert_eq!(
                got, want,
                "{} record {}, contour {}",
                record.kind, record.index, number
            );
        }
    }
}

#[test]
fn a_solid_rectangle_has_the_area_of_its_centre_polygon() {
    assert_eq!(
        contours(&["#####", "#####", "#####", "#####"]),
        [contour(0, 0, 5, 4, 24)]
    );
    assert_eq!(contours(&["############"; 9]), [contour(0, 0, 12, 9, 176)]);
    for (width, height) in [(1, 1), (1, 5), (5, 1), (1, 9), (9, 1), (2, 1), (1, 2)] {
        let rows: Vec<String> = vec!["#".repeat(width); height];
        let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
        let twice = u64::try_from(2 * (width - 1) * (height - 1)).expect("a small area");
        assert_eq!(
            contours(&rows),
            [contour(0, 0, width, height, twice)],
            "solid {width} by {height}"
        );
    }
}

#[test]
fn an_isolated_pixel_and_thin_lines_have_zero_area() {
    assert_eq!(contours(&["...", ".#.", "..."]), [contour(1, 1, 1, 1, 0)]);
    assert_eq!(
        contours(&[".......", ".......", ".#####.", ".......", "......."]),
        [contour(1, 2, 5, 1, 0)]
    );
    assert_eq!(
        contours(&["..#..", "..#..", "..#..", "..#..", "..#.."]),
        [contour(2, 0, 1, 5, 0)]
    );
    assert_eq!(
        contours(&["#....", ".#...", "..#..", "...#.", "....#"]),
        [contour(0, 0, 5, 5, 0)]
    );
}

#[test]
fn components_inside_a_hole_are_not_reported() {
    assert_eq!(
        contours(&[
            ".......", "..###..", ".#...#.", ".#.#.#.", ".#...#.", "..###..", "......."
        ]),
        [contour(1, 1, 5, 5, 28)]
    );
    assert_eq!(
        contours(&[
            "....#....",
            "...#.#...",
            "..#...#..",
            ".#..#..#.",
            "..#...#..",
            "...#.#...",
            "....#...."
        ]),
        [contour(1, 0, 7, 7, 36)]
    );
    assert_eq!(
        contours(&[
            "###########",
            "#.........#",
            "#.#######.#",
            "#.#.....#.#",
            "#.#.###.#.#",
            "#.#.###.#.#",
            "#.#.....#.#",
            "#.#######.#",
            "#.........#",
            "###########"
        ]),
        [contour(0, 0, 11, 10, 180)]
    );
}

#[test]
fn corner_touching_blobs_are_one_contour_and_blobs_one_pixel_apart_are_two() {
    assert_eq!(contours(&["#..", ".#.", "..#"]), [contour(0, 0, 3, 3, 0)]);
    assert_eq!(contours(&["##.", ".##", "..#"]), [contour(0, 0, 3, 3, 3)]);
    assert_eq!(
        contours(&["##...", ".....", "...##"]),
        [contour(3, 2, 2, 1, 0), contour(0, 0, 2, 1, 0)]
    );
}

#[test]
fn the_list_is_in_reverse_discovery_order() {
    assert_eq!(
        contours(&[
            "#.......", "........", "......##", "......##", "........", "##......"
        ]),
        [
            contour(0, 5, 2, 1, 0),
            contour(6, 2, 2, 2, 2),
            contour(0, 0, 1, 1, 0),
        ]
    );
    assert_eq!(
        contours(&["#...#", ".....", ".....", "#...#"]),
        [
            contour(4, 3, 1, 1, 0),
            contour(0, 3, 1, 1, 0),
            contour(4, 0, 1, 1, 0),
            contour(0, 0, 1, 1, 0),
        ]
    );
}

#[test]
fn an_empty_picture_has_no_contours_and_a_full_picture_is_one() {
    for (width, height) in [(1, 1), (3, 5), (1, 9), (9, 1)] {
        let picture = vec![0u8; width * height];
        let shape = Shape::new(width, height).expect("a shape");
        let plane = Plane::new(&picture, shape).expect("a plane");
        assert!(external_contours(plane).is_empty(), "{width} by {height}");
    }
    assert_eq!(contours(&["#####"; 5]), [contour(0, 0, 5, 5, 32)]);
    assert_eq!(contours(&["#######"]), [contour(0, 0, 7, 1, 0)]);
    assert_eq!(contours(&["#"; 7]), [contour(0, 0, 1, 7, 0)]);
}

#[test]
fn any_non_zero_byte_is_foreground() {
    for rows in [
        &[".....", ".###.", ".#.#.", ".##..", "....."][..],
        &["#####", "#####", "#####", "#####"][..],
    ] {
        let first = contours_at(rows, 1);
        for level in [7u8, 128, 255] {
            assert_eq!(first, contours_at(rows, level), "level {level}");
        }
    }
}

#[test]
fn shapes_touching_each_border_and_corner_are_traced_whole() {
    assert_eq!(
        contours(&["#####", ".....", ".....", ".....", "....."]),
        [contour(0, 0, 5, 1, 0)]
    );
    assert_eq!(
        contours(&[".....", ".....", ".....", ".....", "#####"]),
        [contour(0, 4, 5, 1, 0)]
    );
    assert_eq!(
        contours(&["#....", "#....", "#....", "#....", "#...."]),
        [contour(0, 0, 1, 5, 0)]
    );
    assert_eq!(
        contours(&["....#", "....#", "....#", "....#", "....#"]),
        [contour(4, 0, 1, 5, 0)]
    );
    assert_eq!(
        contours(&["#....#", "......", "......", "......", "......", "#....#"]),
        [
            contour(5, 5, 1, 1, 0),
            contour(0, 5, 1, 1, 0),
            contour(5, 0, 1, 1, 0),
            contour(0, 0, 1, 1, 0),
        ]
    );
    assert_eq!(
        contours(&["..#..", "..#..", "#####", "..#..", "..#.."]),
        [contour(0, 0, 5, 5, 4)]
    );
    assert_eq!(
        contours(&["######", "#....#", "#....#", "#....#", "#....#", "######"]),
        [contour(0, 0, 6, 6, 50)]
    );
}
