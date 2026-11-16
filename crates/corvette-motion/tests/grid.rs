// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and `clippy.toml` does not forbid them.
use corvette_motion::{
    Bounds, Margin, Region, Shape, SizeCell, SizeGrid, calculate_region, calculate_region_f64,
    cluster_region_from_grid, region_from_grid, startup_regions,
};

const FIXTURE: &str = include_str!("fixtures/grid_cases.txt");

fn fixture_lines(kind: &str) -> Vec<&'static str> {
    FIXTURE
        .lines()
        .filter(|line| line.starts_with(kind))
        .collect()
}

fn parts(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}

fn frame(p: &[&str], at: usize) -> Shape {
    Shape::new(
        p[at].parse().expect("a frame width"),
        p[at + 1].parse().expect("a frame height"),
    )
    .expect("a frame shape")
}

fn floor_of(p: &[&str], at: usize) -> usize {
    p[at].parse().expect("a floor")
}

fn grid_id_of(p: &[&str]) -> usize {
    p[1].parse().expect("a grid id")
}

fn square(p: &[&str], at: usize) -> Region {
    let four: Vec<i32> = p[at..at + 4]
        .iter()
        .map(|value| value.parse().expect("an edge"))
        .collect();
    Region {
        x1: four[0],
        y1: four[1],
        x2: four[2],
        y2: four[3],
    }
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

fn boxes_of(text: &str) -> Vec<Region> {
    text.split(';').map(square_of_group).collect()
}

fn cluster_of(text: &str) -> Vec<usize> {
    if text == "-" {
        return Vec::new();
    }
    text.split(';')
        .map(|value| value.parse().expect("an index"))
        .collect()
}

fn regions_of(text: &str) -> Vec<Region> {
    if text == "-" {
        return Vec::new();
    }
    text.split(';').map(square_of_group).collect()
}

fn grid_from_spec(text: &str) -> SizeGrid {
    let mut grid = SizeGrid::empty();
    if text != "-" {
        for entry in text.split(';') {
            let field: Vec<&str> = entry.split(',').collect();
            let cell = SizeCell {
                samples: field[2].parse().expect("a sample count"),
                mean: field[3].parse().expect("a mean"),
                std_dev: field[4].parse().expect("a standard deviation"),
            };
            let x = field[0].parse().expect("a grid x");
            let y = field[1].parse().expect("a grid y");
            assert!(grid.set(x, y, cell), "cell {entry} outside the grid");
        }
    }
    grid
}

fn grids() -> Vec<SizeGrid> {
    let lines = fixture_lines("grid ");
    assert_eq!(lines.len(), 30, "fixture carries 30 grid lines");
    let mut grids = Vec::new();
    for line in lines {
        let p = parts(line);
        assert_eq!(grid_id_of(&p), grids.len(), "grid ids run in order");
        grids.push(grid_from_spec(p[2]));
    }
    grids
}

fn cell_of(frame: Shape, plain: Region) -> (usize, usize) {
    let width = i32::try_from(frame.width()).expect("a small frame side");
    let height = i32::try_from(frame.height()).expect("a small frame side");
    let centre_twice_x = i64::from(plain.x1 * 2 + plain.x2.min(width) - plain.x1);
    let centre_twice_y = i64::from(plain.y1 * 2 + plain.y2.min(height) - plain.y1);
    let grid_x =
        usize::try_from(centre_twice_x * 8 / (i64::from(width) * 2)).expect("a grid index");
    let grid_y =
        usize::try_from(centre_twice_y * 8 / (i64::from(height) * 2)).expect("a grid index");
    (grid_x, grid_y)
}

fn ties_at_the_cut(grid: &SizeGrid) -> bool {
    let mut samples = Vec::new();
    for x in 0..8 {
        for y in 0..8 {
            let cell = grid.cell(x, y).expect("a grid cell");
            if cell.samples > 0 {
                samples.push(cell.samples);
            }
        }
    }
    if samples.len() < 8 {
        return false;
    }
    samples.sort_by_key(|&count| std::cmp::Reverse(count));
    let inside = samples.windows(2).any(|pair| pair[0] == pair[1]);
    let at_cut = samples.len() > 8 && samples[7] == samples[8];
    inside || at_cut
}

#[test]
fn region_from_grid_matches_frigate() {
    let grids = grids();
    let mut checked = 0;
    for line in fixture_lines("fromgrid ") {
        let p = parts(line);
        let grid = &grids[grid_id_of(&p)];
        assert_eq!(
            region_from_grid(frame(&p, 2), square(&p, 5), floor_of(&p, 4), grid),
            Some(square(&p, 9)),
            "fixture line: {line}"
        );
        checked += 1;
    }
    assert_eq!(checked, 600, "{checked} fromgrid lines checked");
}

#[test]
fn cluster_region_from_grid_matches_frigate() {
    let grids = grids();
    let mut checked = 0;
    for line in fixture_lines("clustergrid ") {
        let p = parts(line);
        let grid = &grids[grid_id_of(&p)];
        assert_eq!(
            cluster_region_from_grid(
                frame(&p, 2),
                floor_of(&p, 4),
                &cluster_of(p[6]),
                &boxes_of(p[5]),
                grid
            ),
            Some(square(&p, 7)),
            "fixture line: {line}"
        );
        checked += 1;
    }
    assert_eq!(checked, 150, "{checked} clustergrid lines checked");
}

#[test]
fn startup_regions_match_frigate() {
    let grids = grids();
    let mut checked = 0;
    for line in fixture_lines("startup ") {
        let p = parts(line);
        let grid = &grids[grid_id_of(&p)];
        assert_eq!(
            startup_regions(frame(&p, 2), floor_of(&p, 4), grid),
            Some(regions_of(p[5])),
            "fixture line: {line}"
        );
        checked += 1;
    }
    assert_eq!(checked, 50, "{checked} startup lines checked");
}

#[test]
fn grid_fixture_reaches_the_rules() {
    let grids = grids();
    let mut empty_cell = 0;
    let mut band = 0;
    let mut above = 0;
    let mut resized = 0;
    let mut resized_edge = 0;
    for line in fixture_lines("fromgrid ") {
        let p = parts(line);
        let grid = &grids[grid_id_of(&p)];
        let shape = frame(&p, 2);
        let floor = floor_of(&p, 4);
        let box_edges = square(&p, 5);
        let plain =
            calculate_region(shape, box_edges, floor, Margin::Motion).expect("a plain region");
        let result = region_from_grid(shape, box_edges, floor, grid).expect("a grid region");
        let (gx, gy) = cell_of(shape, plain);
        let cell = *grid.cell(gx, gy).expect("a grid cell");
        let calc_size = f64::from(plain.x2 - plain.x1)
            / f64::from(u32::try_from(shape.width()).expect("a small frame side"));
        let low = cell.mean - cell.std_dev;
        let high = cell.mean + cell.std_dev;
        if result == plain && cell.samples == 0 {
            empty_cell += 1;
        } else if result == plain && (low..=high).contains(&calc_size) {
            band += 1;
        } else if result == plain && calc_size > high {
            above += 1;
        } else if result != plain {
            resized += 1;
            let width = i64::try_from(shape.width()).expect("a small frame side");
            let height = i64::try_from(shape.height()).expect("a small frame side");
            if result.x1 == 0
                || result.y1 == 0
                || i64::from(result.x2) == width
                || i64::from(result.y2) == height
            {
                resized_edge += 1;
            }
        }
    }
    assert!(
        empty_cell >= 100,
        "{empty_cell} lines found an empty cell under the centre"
    );
    assert!(
        band >= 15,
        "{band} lines kept the plain region inside the band"
    );
    assert!(above >= 25, "{above} lines were above the band");
    assert!(resized >= 18, "{resized} lines were resized by their cell");
    assert!(
        resized_edge >= 20,
        "{resized_edge} resized lines touch a frame edge"
    );
    let mut startup_none = 0;
    let mut startup_few = 0;
    let mut startup_full = 0;
    let mut tie_lines = 0;
    for line in fixture_lines("startup ") {
        let p = parts(line);
        let grid = &grids[grid_id_of(&p)];
        match regions_of(p[5]).len() {
            0 => startup_none += 1,
            1..=7 => startup_few += 1,
            _ => startup_full += 1,
        }
        if ties_at_the_cut(grid) {
            tie_lines += 1;
        }
    }
    assert!(
        startup_full >= 10,
        "{startup_full} startup lines carry 8 regions"
    );
    assert!(
        startup_few >= 10,
        "{startup_few} startup lines carry 1 to 7 regions"
    );
    assert!(
        startup_none >= 1,
        "{startup_none} startup lines carry no region; only the fully empty grid yields none"
    );
    assert!(
        tie_lines >= 5,
        "{tie_lines} startup lines select samples that tie at or inside the cut"
    );
}

#[test]
fn empty_grid_gives_plain_regions() {
    let grid = SizeGrid::empty();
    assert_eq!(grid, SizeGrid::default());
    let shape = Shape::new(640, 480).expect("a frame shape");
    let boxes = [
        Region {
            x1: 0,
            y1: 0,
            x2: 50,
            y2: 40,
        },
        Region {
            x1: 900,
            y1: 500,
            x2: 960,
            y2: 560,
        },
        Region {
            x1: -20,
            y1: -20,
            x2: 700,
            y2: 700,
        },
        Region {
            x1: 600,
            y1: 400,
            x2: 640,
            y2: 480,
        },
    ];
    for box_edges in boxes {
        assert_eq!(
            region_from_grid(shape, box_edges, 320, &grid),
            calculate_region(shape, box_edges, 320, Margin::Motion)
        );
    }
    assert_eq!(startup_regions(shape, 320, &grid), Some(vec![]));
    let stored = SizeCell {
        samples: 2,
        mean: 0.5,
        std_dev: 0.1,
    };
    let mut grid = SizeGrid::empty();
    assert!(grid.set(7, 7, stored));
    assert!(!grid.set(8, 0, SizeCell::EMPTY));
    assert!(!grid.set(0, 8, SizeCell::EMPTY));
    assert_eq!(grid.cell(8, 0), None);
    assert_eq!(grid.cell(7, 7), Some(&stored));
}

#[test]
fn band_edges_keep_the_plain_region() {
    // The plain region (0, 0, 320, 320) has relative width exactly 0.5; each
    // pair below puts a band edge (mean +/- std_dev, exact in binary) on 0.5.
    let shape = Shape::new(640, 480).expect("a frame shape");
    let box_edges = Region {
        x1: 0,
        y1: 0,
        x2: 50,
        y2: 40,
    };
    let cases = [
        (0.375, 0.125, (0, 0, 320, 320)),
        (0.625, 0.125, (0, 0, 320, 320)),
        (0.7, 0.125, (0, 0, 768, 768)),
        (0.3, 0.125, (0, 0, 320, 320)),
    ];
    for (mean, std_dev, (x1, y1, x2, y2)) in cases {
        let mut grid = SizeGrid::empty();
        assert!(
            grid.set(
                2,
                2,
                SizeCell {
                    samples: 1,
                    mean,
                    std_dev,
                }
            ),
            "cell (2, 2) is inside the grid"
        );
        assert_eq!(
            region_from_grid(shape, box_edges, 320, &grid),
            Some(Region { x1, y1, x2, y2 }),
            "mean {mean} std_dev {std_dev}"
        );
    }
}

#[test]
fn out_of_domain_inputs_return_none() {
    let shape = Shape::new(640, 480).expect("a frame shape");
    let box_edges = Region {
        x1: 0,
        y1: 0,
        x2: 50,
        y2: 40,
    };
    let wide = Shape::new(1_048_577, 480).expect("a wide frame shape");
    let empty = SizeGrid::empty();
    assert_eq!(region_from_grid(wide, box_edges, 320, &empty), None);
    assert_eq!(region_from_grid(shape, box_edges, 1_048_577, &empty), None);
    let far = Region {
        x1: 1_048_577,
        y1: 0,
        x2: 1_048_627,
        y2: 40,
    };
    assert_eq!(region_from_grid(shape, far, 320, &empty), None);
    assert_eq!(
        cluster_region_from_grid(shape, 320, &[0], &[], &empty),
        None
    );
    assert_eq!(
        cluster_region_from_grid(shape, 320, &[1], &[box_edges], &empty),
        None
    );
    let mut huge = SizeGrid::empty();
    let giant = SizeCell {
        samples: 1,
        mean: 1.0e9,
        std_dev: 0.0,
    };
    assert!(huge.set(2, 2, giant));
    let clamped = calculate_region_f64(
        shape,
        Bounds {
            x1: 0.0,
            y1: 0.0,
            x2: 640.0,
            y2: 480.0,
        },
        320,
        Margin::Motion,
    )
    .expect("the whole-frame bounds fit");
    assert_eq!(
        region_from_grid(shape, box_edges, 320, &huge),
        Some(clamped)
    );
    assert_eq!(startup_regions(shape, 320, &huge), None);
    assert_eq!(startup_regions(shape, 1_048_577, &huge), None);
    assert_eq!(startup_regions(wide, 320, &huge), None);
}
