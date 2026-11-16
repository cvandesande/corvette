// Tests may unwrap, expect and index: `docs/standards/STYLE.md` states
// "Tests may unwrap, expect and index", and `clippy.toml` does not forbid them.
use corvette_motion::{
    Region, RegionBuilder, Shape, SizeCell, SizeGrid, box_inside, build_regions,
};

const FIXTURE: &str = include_str!("fixtures/builder_cases.txt");

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
        p[2].parse().expect("a frame width"),
        p[3].parse().expect("a frame height"),
    )
    .expect("a frame shape")
}

fn floor_of(p: &[&str]) -> usize {
    p[4].parse().expect("a floor")
}

fn flag(p: &[&str], at: usize) -> bool {
    p[at].parse::<u8>().expect("a flag") == 1
}

fn count(p: &[&str], at: usize) -> usize {
    p[at].parse().expect("a count")
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

fn squares(text: &str) -> Vec<Region> {
    if text == "-" {
        return Vec::new();
    }
    text.split(';').map(square_of_group).collect()
}

fn grids() -> Vec<SizeGrid> {
    let lines = fixture_lines("grid ");
    assert_eq!(lines.len(), 12, "fixture carries 12 grid lines");
    let mut grids = Vec::new();
    for line in lines {
        let p = parts(line);
        assert_eq!(count(&p, 1), grids.len(), "grid ids run in order");
        let mut grid = SizeGrid::empty();
        if p[2] != "-" {
            for entry in p[2].split(';') {
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
        grids.push(grid);
    }
    grids
}

fn sorted(regions: &[Region]) -> Vec<(i32, i32, i32, i32)> {
    let mut keys: Vec<(i32, i32, i32, i32)> =
        regions.iter().map(|r| (r.x1, r.y1, r.x2, r.y2)).collect();
    keys.sort_unstable();
    keys
}

fn boxen(rows: &[(i32, i32, i32, i32)]) -> Vec<Region> {
    rows.iter()
        .map(|&(x1, y1, x2, y2)| Region { x1, y1, x2, y2 })
        .collect()
}

fn case8_grid() -> SizeGrid {
    let mut grid = SizeGrid::empty();
    assert!(grid.set(
        3,
        4,
        SizeCell {
            samples: 3,
            mean: 0.423_333_333_333_333_34,
            std_dev: 0.020_548_046_676_563_25,
        }
    ));
    grid
}

#[test]
fn build_regions_matches_frigate() {
    let grids = grids();
    let mut checked = 0;
    for line in fixture_lines("build ") {
        let p = parts(line);
        let grid = &grids[count(&p, 1)];
        let object = squares(p[7]);
        let motion = squares(p[8]);
        let expected = squares(p[11]);
        let n_object = count(&p, 9);
        let tail = n_object + count(&p, 10);
        let result = build_regions(
            frame(&p),
            floor_of(&p),
            grid,
            &object,
            &motion,
            flag(&p, 6),
            flag(&p, 5),
        )
        .expect("a fixture line is in the domain");
        assert_eq!(result.len(), expected.len(), "total of: {line}");
        assert_eq!(
            sorted(&result[..n_object]),
            sorted(&expected[..n_object]),
            "object group of: {line}"
        );
        assert_eq!(
            sorted(&result[n_object..tail]),
            sorted(&expected[n_object..tail]),
            "motion group of: {line}"
        );
        assert_eq!(
            sorted(&result[tail..]),
            sorted(&expected[tail..]),
            "startup group of: {line}"
        );
        assert_eq!(
            &result[tail..],
            &expected[tail..],
            "startup order of: {line}"
        );
        checked += 1;
    }
    assert_eq!(checked, 400, "{checked} build lines checked");
}

#[test]
fn builder_fixture_reaches_the_rules() {
    let mut one_object_region = 0;
    let mut many_object_regions = 0;
    let mut one_motion_region = 0;
    let mut many_motion_regions = 0;
    let mut motion_inside = 0;
    let mut motion_equal_region = 0;
    let mut gate_closed_motion = 0;
    let mut startup_present = 0;
    let mut startup_without_region = 0;
    for line in fixture_lines("build ") {
        let p = parts(line);
        let startup = flag(&p, 5);
        let enabled = flag(&p, 6);
        let n_object = count(&p, 9);
        let n_motion = count(&p, 10);
        let motion = squares(p[8]);
        let expected = squares(p[11]);
        let object_regions = &expected[..n_object];
        let startup_regions = &expected[n_object + n_motion..];
        if !object_regions.is_empty() {
            one_object_region += 1;
        }
        if object_regions.len() >= 2 {
            many_object_regions += 1;
        }
        if n_motion >= 1 {
            one_motion_region += 1;
        }
        if n_motion >= 2 {
            many_motion_regions += 1;
        }
        if enabled
            && motion
                .iter()
                .any(|b| object_regions.iter().any(|r| box_inside(*r, *b)))
        {
            motion_inside += 1;
        }
        if motion.iter().any(|b| object_regions.contains(b)) {
            motion_equal_region += 1;
        }
        if !enabled && !motion.is_empty() {
            gate_closed_motion += 1;
        }
        if !startup_regions.is_empty() {
            startup_present += 1;
        }
        if startup && startup_regions.is_empty() {
            startup_without_region += 1;
        }
    }
    assert!(
        one_object_region >= 150,
        "{one_object_region} lines carry at least one object region"
    );
    assert!(
        many_object_regions >= 40,
        "{many_object_regions} lines carry two or more object regions"
    );
    assert!(
        one_motion_region >= 100,
        "{one_motion_region} lines carry at least one motion region"
    );
    assert!(
        many_motion_regions >= 20,
        "{many_motion_regions} lines carry two or more motion regions"
    );
    assert!(
        motion_inside >= 40,
        "{motion_inside} enabled lines have a motion box inside an object region"
    );
    assert!(
        motion_equal_region >= 10,
        "{motion_equal_region} lines have a motion box equal to an object region"
    );
    assert!(
        gate_closed_motion >= 30,
        "{gate_closed_motion} disabled lines carry motion boxes"
    );
    assert!(
        startup_present >= 40,
        "{startup_present} lines carry at least one startup region"
    );
    assert!(
        startup_without_region >= 8,
        "{startup_without_region} startup lines carry no startup region"
    );
}

#[test]
fn hand_cases_match_frigate() {
    let frame = Shape::new(1920, 1080).expect("a frame shape");
    let empty = SizeGrid::empty();
    let populated = case8_grid();
    let object = boxen(&[(100, 100, 180, 200)]);
    let none: Vec<Region> = Vec::new();
    let build = |grid: &SizeGrid,
                 object_boxes: &[Region],
                 motion_boxes: &[Region],
                 enabled: bool,
                 startup: bool|
     -> Vec<Region> {
        build_regions(
            frame,
            320,
            grid,
            object_boxes,
            motion_boxes,
            enabled,
            startup,
        )
        .expect("a hand case is in the domain")
    };
    assert_eq!(build(&empty, &none, &none, true, true), none);
    assert_eq!(
        build(&empty, &object, &none, true, false),
        boxen(&[(0, 0, 320, 320)])
    );
    assert_eq!(
        build(
            &empty,
            &object,
            &boxen(&[(110, 110, 150, 150), (1500, 800, 1560, 860)]),
            true,
            false
        ),
        boxen(&[(0, 0, 320, 320), (1370, 670, 1690, 990)])
    );
    assert_eq!(
        build(
            &empty,
            &object,
            &boxen(&[(110, 110, 150, 150), (1500, 800, 1560, 860)]),
            false,
            false
        ),
        boxen(&[(0, 0, 320, 320)])
    );
    assert_eq!(
        build(
            &empty,
            &none,
            &boxen(&[(1500, 800, 1560, 860)]),
            true,
            false
        ),
        boxen(&[(1370, 670, 1690, 990)])
    );
    assert_eq!(
        build(&empty, &object, &boxen(&[(0, 0, 320, 320)]), true, false),
        boxen(&[(0, 0, 320, 320)])
    );
    assert_eq!(
        build(&empty, &object, &boxen(&[(0, 0, 321, 320)]), true, false),
        boxen(&[(0, 0, 320, 320), (0, 0, 640, 640)])
    );
    assert_eq!(
        build(
            &populated,
            &object,
            &boxen(&[(900, 500, 960, 560)]),
            true,
            true
        ),
        boxen(&[
            (0, 0, 320, 320),
            (770, 370, 1090, 690),
            (434, 201, 1246, 1013)
        ])
    );
    assert_eq!(
        build(
            &populated,
            &object,
            &boxen(&[(900, 500, 960, 560)]),
            false,
            true
        ),
        boxen(&[(0, 0, 320, 320), (434, 201, 1246, 1013)])
    );
}

#[test]
fn region_builder_runs_the_startup_scan_once() {
    let frame = Shape::new(1920, 1080).expect("a frame shape");
    let wide = Shape::new(1_048_577, 1080).expect("a wide frame shape");
    let grid = case8_grid();
    let object = boxen(&[(100, 100, 180, 200)]);
    let motion = boxen(&[(900, 500, 960, 560)]);
    let three = boxen(&[
        (0, 0, 320, 320),
        (770, 370, 1090, 690),
        (434, 201, 1246, 1013),
    ]);
    let two = boxen(&[(0, 0, 320, 320), (770, 370, 1090, 690)]);
    let mut builder = RegionBuilder::new(320, grid.clone());
    assert_eq!(
        builder.build(frame, &object, &motion, true),
        Some(three.clone())
    );
    assert_eq!(builder.build(frame, &object, &motion, true), Some(two));
    let mut fresh = RegionBuilder::new(320, grid);
    assert_eq!(fresh.build(wide, &object, &motion, true), None);
    assert_eq!(fresh.build(frame, &object, &motion, true), Some(three));
}

#[test]
fn out_of_domain_inputs_return_none() {
    let frame = Shape::new(1920, 1080).expect("a frame shape");
    let wide = Shape::new(1_048_577, 1080).expect("a wide frame shape");
    let grid = SizeGrid::empty();
    let none: &[Region] = &[];
    let far_object = boxen(&[(1_048_577, 0, 1_048_627, 40)]);
    let far_motion = boxen(&[(1_048_577, 0, 1_048_627, 40)]);
    assert_eq!(
        build_regions(wide, 320, &grid, none, none, true, false),
        None
    );
    assert_eq!(
        build_regions(frame, 1_048_577, &grid, none, none, true, false),
        None
    );
    assert_eq!(
        build_regions(frame, 320, &grid, &far_object, none, true, false),
        None
    );
    assert_eq!(
        build_regions(frame, 320, &grid, none, &far_motion, true, false),
        None
    );
    assert_eq!(
        build_regions(frame, 320, &grid, none, &far_motion, false, false),
        Some(vec![])
    );
}
