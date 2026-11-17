use corvette_track::{IdSource, Observation, Track, TrackConfig};
use std::collections::BTreeMap;

const FIXTURE: &str = include_str!("fixtures/object_cases.txt");

const HAND_ROWS: &str = r"### hit-every-frame   cfg max=25 delay=2 past=4 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 s:0 h:0:1:car:101,100,141,180 s:0 h:0:2:car:102,100,142,180 s:0 h:0:3:car:103,100,143,180 s:0 h:0:4:car:104,100,144,180 s:0 h:0:5:car:105,100,145,180 s:0 h:0:6:car:106,100,146,180
after : 0:1,0,1,1,-,0,0,0@0 | 0:0,1,1,1,-,0,0,0@0 | 0:2,1,1,1,-,0,1,0@0/1@1 | 0:1,2,1,1,-,0,1,0@0/1@1 | 0:3,2,0,1,1,1,2,0@0/1@1/2@2 | 0:2,3,0,1,1,1,2,0@0/1@1/2@2 | 0:4,3,0,1,1,1,3,0@0/1@1/2@2/3@3 | 0:3,4,0,1,1,1,3,0@0/1@1/2@2/3@3 | 0:5,4,0,1,1,1,4,1@1/2@2/3@3/4@4 | 0:4,5,0,1,1,1,4,1@1/2@2/3@3/4@4 | 0:6,5,0,1,1,1,5,2@2/3@3/4@4/5@5 | 0:5,6,0,1,1,1,5,2@2/3@3/4@4/5@5 | 0:7,6,0,1,1,1,6,2@2/3@3/4@4/5@5
final : 0: label=car est=405a6afedfae92d2.4059000000000000.4062357f6fd74969.4066800000000000 vel=3fec4c5f4aabf305.0000000000000000.3fec4c5f4aabf37f.0000000000000000

### delay-zero   cfg max=25 delay=0 past=4 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 s:0 h:0:1:car:101,100,141,180
after : 0:1,0,0,1,1,1,0,0@0 | 0:0,1,0,1,1,1,0,0@0 | 0:2,1,0,1,1,1,1,0@0/1@1
final : 0: label=car est=405930eb991a6f0f.4059000000000000.40619875cc8d3787.4066800000000000 vel=3fb1bda63b411bda.0000000000000000.3fb1bda63b411bda.0000000000000000

### miss-then-recover   cfg max=25 delay=2 past=4 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 s:0 h:0:1:car:101,100,141,180 s:0 h:0:2:car:102,100,142,180 s:0 s:0 s:0 s:0 s:0 h:0:3:car:108,100,148,180
after : 0:1,0,1,1,-,0,0,0@0 | 0:0,1,1,1,-,0,0,0@0 | 0:2,1,1,1,-,0,1,0@0/1@1 | 0:1,2,1,1,-,0,1,0@0/1@1 | 0:3,2,0,1,1,1,2,0@0/1@1/2@2 | 0:2,3,0,1,1,1,2,0@0/1@1/2@2 | 0:1,4,0,1,1,1,2,0@0/1@1/2@2 | 0:0,5,0,1,1,1,2,0@0/1@1/2@2 | 0:-1,6,0,1,1,0,2,0@0/1@1/2@2 | 0:-2,7,0,1,1,0,2,0@0/1@1/2@2 | 0:0,7,0,1,1,1,3,0@0/1@1/2@2/3@7
final : 0: label=car est=405a60aad54887dd.4059000000000000.406230556aa443ee.4066800000000000 vel=3ff3d8f5086c7050.0000000000000000.3ff3d8f5086c7071.0000000000000000

### cap-at-max   cfg max=5 delay=2 past=4 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 s:0 h:0:1:car:101,100,141,180 s:0 h:0:2:car:102,100,142,180 s:0 h:0:3:car:103,100,143,180 s:0 h:0:4:car:104,100,144,180 s:0 h:0:5:car:105,100,145,180
after : 0:1,0,1,1,-,0,0,0@0 | 0:0,1,1,1,-,0,0,0@0 | 0:2,1,1,1,-,0,1,0@0/1@1 | 0:1,2,1,1,-,0,1,0@0/1@1 | 0:3,2,0,1,1,1,2,0@0/1@1/2@2 | 0:2,3,0,1,1,1,2,0@0/1@1/2@2 | 0:4,3,0,1,1,1,3,0@0/1@1/2@2/3@3 | 0:3,4,0,1,1,1,3,0@0/1@1/2@2/3@3 | 0:5,4,0,1,1,1,4,1@1/2@2/3@3/4@4 | 0:4,5,0,1,1,1,4,1@1/2@2/3@3/4@4 | 0:5,5,0,1,1,1,5,2@2/3@3/4@4/5@5
final : 0: label=car est=405a24b84fe758c1.4059000000000000.4062125c27f3ac60.4066800000000000 vel=3fe9e3e3d5979ce9.0000000000000000.3fe9e3e3d5979d53.0000000000000000

### steps-without-hits   cfg max=25 delay=2 past=4 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 s:0 s:0 s:0 s:0
after : 0:1,0,1,1,-,0,0,0@0 | 0:0,1,1,1,-,0,0,0@0 | 0:-1,2,1,1,-,0,0,0@0 | 0:-2,3,1,1,-,0,0,0@0 | 0:-3,4,1,1,-,0,0,0@0
final : 0: label=car est=4059000000000000.4059000000000000.4061800000000000.4066800000000000 vel=0000000000000000.0000000000000000.0000000000000000.0000000000000000

### two-objects-share-counters   cfg max=25 delay=2 past=4 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 n:1:car:150,100,190,180 s:0 s:1 h:1:2:car:151,100,191,180 h:0:3:car:101,100,141,180 s:0 s:1 h:1:4:car:152,100,192,180 h:0:5:car:102,100,142,180 s:1 h:1:6:car:153,100,193,180 s:0 h:0:7:car:103,100,143,180
after : 0:1,0,1,1,-,0,0,0@0 | 1:1,0,1,2,-,0,1,1@0 | 0:0,1,1,1,-,0,0,0@0 | 1:0,1,1,2,-,0,1,1@0 | 1:2,1,1,2,-,0,2,1@0/2@1 | 0:2,1,1,1,-,0,3,0@0/3@1 | 0:1,2,1,1,-,0,3,0@0/3@1 | 1:1,2,1,2,-,0,2,1@0/2@1 | 1:3,2,0,2,1,1,4,1@0/2@1/4@2 | 0:3,2,0,1,2,1,5,0@0/3@1/5@2 | 1:2,3,0,2,1,1,4,1@0/2@1/4@2 | 1:4,3,0,2,1,1,6,1@0/2@1/4@2/6@3 | 0:2,3,0,1,2,1,5,0@0/3@1/5@2 | 0:4,3,0,1,2,1,7,0@0/3@1/5@2/7@3
final : 0: label=car est=4059994c6f4ea987.4059000000000000.4061cca637a754c3.4066800000000000 vel=3fdfb4087b9097fe.0000000000000000.3fdfb4087b909880.0000000000000000
final : 1: label=car est=40630ca637a754c3.4059000000000000.40680ca637a754c3.4066800000000000 vel=3fdfb4087b909880.0000000000000000.3fdfb4087b909880.0000000000000000

### past-detections-spread   cfg max=25 delay=2 past=4 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 s:0 h:0:1:car:101,100,141,180 s:0 h:0:2:car:102,100,142,180 s:0 h:0:3:car:103,100,143,180 s:0 h:0:4:car:104,100,144,180 s:0 h:0:5:car:105,100,145,180 s:0 h:0:6:car:106,100,146,180 s:0 h:0:7:car:107,100,147,180 s:0 h:0:8:car:108,100,148,180 s:0 h:0:9:car:109,100,149,180 s:0 h:0:10:car:110,100,150,180 s:0 h:0:11:car:111,100,151,180 s:0 h:0:12:car:112,100,152,180
after : 0:1,0,1,1,-,0,0,0@0 | 0:0,1,1,1,-,0,0,0@0 | 0:2,1,1,1,-,0,1,0@0/1@1 | 0:1,2,1,1,-,0,1,0@0/1@1 | 0:3,2,0,1,1,1,2,0@0/1@1/2@2 | 0:2,3,0,1,1,1,2,0@0/1@1/2@2 | 0:4,3,0,1,1,1,3,0@0/1@1/2@2/3@3 | 0:3,4,0,1,1,1,3,0@0/1@1/2@2/3@3 | 0:5,4,0,1,1,1,4,1@1/2@2/3@3/4@4 | 0:4,5,0,1,1,1,4,1@1/2@2/3@3/4@4 | 0:6,5,0,1,1,1,5,2@2/3@3/4@4/5@5 | 0:5,6,0,1,1,1,5,2@2/3@3/4@4/5@5 | 0:7,6,0,1,1,1,6,2@2/3@3/4@4/5@5 | 0:6,7,0,1,1,1,6,2@2/3@3/4@4/5@5 | 0:8,7,0,1,1,1,7,2@2/3@3/4@4/5@5 | 0:7,8,0,1,1,1,7,2@2/3@3/4@4/5@5 | 0:9,8,0,1,1,1,8,3@3/4@4/5@5/8@8 | 0:8,9,0,1,1,1,8,3@3/4@4/5@5/8@8 | 0:10,9,0,1,1,1,9,3@3/4@4/5@5/8@8 | 0:9,10,0,1,1,1,9,3@3/4@4/5@5/8@8 | 0:11,10,0,1,1,1,10,3@3/4@4/5@5/8@8 | 0:10,11,0,1,1,1,10,3@3/4@4/5@5/8@8 | 0:12,11,0,1,1,1,11,3@3/4@4/5@5/8@8 | 0:11,12,0,1,1,1,11,3@3/4@4/5@5/8@8 | 0:13,12,0,1,1,1,12,4@4/5@5/8@8/12@12
final : 0: label=car est=405bfd58c399fc2a.4059000000000000.4062feac61ccfe15.4066800000000000 vel=3ff014a2c6e62895.0000000000000000.3ff014a2c6e62888.0000000000000000

### past-length-zero   cfg max=25 delay=2 past=0 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 s:0 h:0:1:car:101,100,141,180 s:0 h:0:2:car:102,100,142,180
after : 0:1,0,1,1,-,0,0,- | 0:0,1,1,1,-,0,0,- | 0:2,1,1,1,-,0,1,- | 0:1,2,1,1,-,0,1,- | 0:3,2,0,1,1,1,2,-
final : 0: label=car est=40595dfbc79f333d.4059000000000000.4061aefde3cf999e.4066800000000000 vel=3fd0677a152b6c02.0000000000000000.3fd0677a152b6c54.0000000000000000

### hit-without-step   cfg max=25 delay=2 past=4 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 h:0:1:car:101,100,141,180 h:0:2:car:102,100,142,180 h:0:3:car:103,100,143,180
after : 0:1,0,1,1,-,0,0,0@0 | 0:3,0,0,1,1,1,1,0@0/1@0 | 0:5,0,0,1,1,1,2,0@0/1@0/2@0 | 0:7,0,0,1,1,1,3,0@0/1@0/2@0/3@0
final : 0: label=car est=4059908fa1493e83.4059000000000000.4061c847d0a49f41.4066800000000000 vel=3fe1ef2963e0a2b8.0000000000000000.3fe1ef2963e0a2cd.0000000000000000

### past-length-one   cfg max=25 delay=2 past=1 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 s:0 h:0:1:car:101,100,141,180 s:0 h:0:2:car:102,100,142,180 s:0 h:0:3:car:103,100,143,180 s:0 h:0:4:car:104,100,144,180 s:0 h:0:5:car:105,100,145,180
after : 0:1,0,1,1,-,0,0,0@0 | 0:0,1,1,1,-,0,0,0@0 | 0:2,1,1,1,-,0,1,1@1 | 0:1,2,1,1,-,0,1,1@1 | 0:3,2,0,1,1,1,2,2@2 | 0:2,3,0,1,1,1,2,2@2 | 0:4,3,0,1,1,1,3,3@3 | 0:3,4,0,1,1,1,3,3@3 | 0:5,4,0,1,1,1,4,4@4 | 0:4,5,0,1,1,1,4,4@4 | 0:6,5,0,1,1,1,5,5@5
final : 0: label=car est=405a24b84fe758c1.4059000000000000.4062125c27f3ac60.4066800000000000 vel=3fe9e3e3d5979ce9.0000000000000000.3fe9e3e3d5979d53.0000000000000000

### delay-boundary   cfg max=25 delay=3 past=4 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 h:0:1:car:101,100,141,180 h:0:2:car:102,100,142,180 h:0:3:car:103,100,143,180
after : 0:1,0,1,1,-,0,0,0@0 | 0:3,0,1,1,-,0,1,0@0/1@0 | 0:5,0,0,1,1,1,2,0@0/1@0/2@0 | 0:7,0,0,1,1,1,3,0@0/1@0/2@0/3@0
final : 0: label=car est=4059908fa1493e83.4059000000000000.4061c847d0a49f41.4066800000000000 vel=3fe1ef2963e0a2b8.0000000000000000.3fe1ef2963e0a2cd.0000000000000000

### delay-one   cfg max=25 delay=1 past=4 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 s:0 h:0:1:car:101,100,141,180 s:0 h:0:2:car:102,100,142,180
after : 0:1,0,1,1,-,0,0,0@0 | 0:0,1,1,1,-,0,0,0@0 | 0:2,1,0,1,1,1,1,0@0/1@1 | 0:1,2,0,1,1,1,1,0@0/1@1 | 0:3,2,0,1,1,1,2,0@0/1@1/2@2
final : 0: label=car est=40595dfbc79f333d.4059000000000000.4061aefde3cf999e.4066800000000000 vel=3fd0677a152b6c02.0000000000000000.3fd0677a152b6c54.0000000000000000

### label-stays-creating   cfg max=25 delay=2 past=4 r=3.4 q=0.03
ops   : n:0:car:100,100,140,180 s:0 h:0:1:person:101,100,141,180 s:0 h:0:2:person:102,100,142,180
after : 0:1,0,1,1,-,0,0,0@0 | 0:0,1,1,1,-,0,0,0@0 | 0:2,1,1,1,-,0,1,0@0/1@1 | 0:1,2,1,1,-,0,1,0@0/1@1 | 0:3,2,0,1,1,1,2,0@0/1@1/2@2
final : 0: label=car est=40595dfbc79f333d.4059000000000000.4061aefde3cf999e.4066800000000000 vel=3fd0677a152b6c02.0000000000000000.3fd0677a152b6c54.0000000000000000

### negative-corners   cfg max=25 delay=2 past=4 r=3.4 q=0.03
ops   : n:0:car:-30,-20,-5,10 s:0 h:0:1:car:-28,-19,-4,12
after : 0:1,0,1,1,-,0,0,0@0 | 0:0,1,1,1,-,0,0,0@0 | 0:2,1,1,1,-,0,1,0@0/1@1
final : 0: label=car est=c03c78a3372c878a.c0333c519b9643c5.c010f1466e590f14.40270eb991a6f0ec vel=3fc1bda63b411bda.3fb1bda63b411bda.3fb1bda63b411bda.3fc1bda63b411bda

";

fn hex_bits(value: f64) -> String {
    format!("{:016x}", value.to_bits())
}

fn parse_edges(text: &str) -> [i32; 4] {
    let values: Vec<i32> = text
        .split(',')
        .map(|part| part.parse().expect("edge"))
        .collect();
    [values[0], values[1], values[2], values[3]]
}

const fn flag(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

fn past_joined(track: &Track<u64>) -> String {
    if track.past_detections().is_empty() {
        return "-".to_string();
    }
    track
        .past_detections()
        .iter()
        .map(|(kept_age, observation)| format!("{}@{}", observation.data, kept_age))
        .collect::<Vec<String>>()
        .join("/")
}

fn hand_config(header: &str) -> (String, TrackConfig) {
    let name = header.strip_prefix("### ").unwrap_or(header).to_string();
    let (_, config_text) = header
        .split_once("   cfg ")
        .unwrap_or_else(|| panic!("hand header without config: {header}"));
    let mut hit_counter_max = 0u32;
    let mut initialization_delay = 0u32;
    let mut past_length = 0u32;
    let mut r = 0f64;
    let mut q = 0f64;
    for token in config_text.split_whitespace() {
        let (key, value) = token.split_once('=').expect("config setting");
        match key {
            "max" => hit_counter_max = value.parse().expect("max"),
            "delay" => initialization_delay = value.parse().expect("delay"),
            "past" => past_length = value.parse().expect("past"),
            "r" => r = value.parse().expect("r"),
            "q" => q = value.parse().expect("q"),
            other => panic!("unknown config setting {other} in {name}"),
        }
    }
    let config = TrackConfig::new(hit_counter_max, initialization_delay, past_length, r, q)
        .unwrap_or_else(|| panic!("hand config of {name} refused"));
    (name, config)
}

fn run_hand_op(
    op: &str,
    name: &str,
    position: usize,
    config: &TrackConfig,
    tracks: &mut Vec<Track<u64>>,
    ids: &mut IdSource,
) -> usize {
    let parts: Vec<&str> = op.split(':').collect();
    match parts[0] {
        "n" => {
            let tag = parts[1].parse().expect("new tag");
            let edges = parse_edges(parts[3]);
            tracks.push(Track::new(
                config,
                ids,
                parts[2],
                Observation { edges, data: tag },
            ));
            tracks.len() - 1
        }
        "s" => {
            let index = parts[1].parse::<usize>().expect("step object index");
            tracks[index].step();
            index
        }
        "h" => {
            let index = parts[1].parse::<usize>().expect("hit object index");
            let tag = parts[2].parse().expect("hit tag");
            let edges = parse_edges(parts[4]);
            tracks[index].hit(ids, Observation { edges, data: tag });
            index
        }
        other => panic!("hand {name} op {position}: unknown kind {other}"),
    }
}

fn check_hand_after(
    track: &Track<u64>,
    expect: &str,
    name: &str,
    position: usize,
    track_index: usize,
) {
    let (expected_index, rest) = expect.split_once(':').expect("hand after entry");
    assert_eq!(
        expected_index.parse::<usize>().expect("hand after index"),
        track_index,
        "hand {name} op {position}: object index"
    );
    let columns: Vec<&str> = rest.split(',').collect();
    assert_eq!(columns.len(), 8, "hand {name} op {position}: after fields");
    let hit_counter: i64 = columns[0].parse().expect("expected counter");
    let age: u64 = columns[1].parse().expect("expected age");
    let initializing_id: u64 = columns[3].parse().expect("expected initializing id");
    let last_tag: u64 = columns[6].parse().expect("expected last tag");
    let id = if columns[4] == "-" {
        None
    } else {
        Some(columns[4].parse().expect("expected id"))
    };
    assert_eq!(
        track.hit_counter(),
        hit_counter,
        "hand {name} op {position}: counter"
    );
    assert_eq!(track.age(), age, "hand {name} op {position}: age");
    assert_eq!(
        track.is_initializing(),
        columns[2] == "1",
        "hand {name} op {position}: initializing"
    );
    assert_eq!(
        track.initializing_id(),
        initializing_id,
        "hand {name} op {position}: initializing id"
    );
    assert_eq!(track.id(), id, "hand {name} op {position}: id");
    assert_eq!(
        track.is_active(),
        columns[5] == "1",
        "hand {name} op {position}: active"
    );
    assert_eq!(
        track.hit_counter_is_positive(),
        hit_counter >= 0,
        "hand {name} op {position}: positive counter"
    );
    assert_eq!(
        track.last_detection().data,
        last_tag,
        "hand {name} op {position}: last tag"
    );
    assert_eq!(
        past_joined(track),
        columns[7],
        "hand {name} op {position}: past list"
    );
}

fn check_hand_final(track: &Track<u64>, text: &str, name: &str) {
    for token in text.split_whitespace() {
        let (key, value) = token.split_once('=').expect("hand final field");
        match key {
            "label" => assert_eq!(track.label(), value, "hand {name}: label"),
            "est" => {
                let estimate = track.estimate();
                for (number, expected) in value.split('.').enumerate() {
                    assert_eq!(
                        expected,
                        hex_bits(estimate[number]),
                        "hand {name}: estimate {number}"
                    );
                }
            }
            "vel" => {
                let velocity = track.velocity();
                for (number, expected) in value.split('.').enumerate() {
                    assert_eq!(
                        expected,
                        hex_bits(velocity[number]),
                        "hand {name}: velocity {number}"
                    );
                }
            }
            other => panic!("hand {name}: unknown final field {other}"),
        }
    }
}

#[test]
fn hand_scenarios_match_the_real_library() {
    let mut seen = 0;
    for block in HAND_ROWS.split("\n\n") {
        if block.trim().is_empty() {
            continue;
        }
        let mut lines = block.lines();
        let header = lines.next().expect("hand block header line");
        let (name, config) = hand_config(header);
        let ops_text = lines
            .next()
            .and_then(|line| line.strip_prefix("ops   : "))
            .expect("hand ops line");
        let after_text = lines
            .next()
            .and_then(|line| line.strip_prefix("after : "))
            .expect("hand after line");
        let ops: Vec<&str> = ops_text
            .split(' ')
            .filter(|token| !token.is_empty())
            .collect();
        let expects: Vec<&str> = after_text.split(" | ").collect();
        assert_eq!(ops.len(), expects.len(), "hand {name}: op and after counts");
        let mut ids = IdSource::default();
        let mut tracks: Vec<Track<u64>> = Vec::new();
        for (position, (op, expect)) in ops.iter().zip(&expects).enumerate() {
            let track_index = run_hand_op(op, &name, position, &config, &mut tracks, &mut ids);
            check_hand_after(&tracks[track_index], expect, &name, position, track_index);
        }
        for line in lines {
            let text = line
                .strip_prefix("final : ")
                .unwrap_or_else(|| panic!("hand {name}: not a final line: {line}"));
            let (index_text, rest) = text.split_once(':').expect("hand final entry");
            check_hand_final(
                &tracks[index_text.parse::<usize>().expect("hand final index")],
                rest,
                &name,
            );
        }
        seen += 1;
    }
    assert_eq!(seen, 14, "hand scenario count");
}

#[test]
fn ids_count_per_source() {
    let config = TrackConfig::new(25, 2, 4, 3.4, 0.03).expect("config");
    let zero_delay = TrackConfig::new(25, 0, 4, 3.4, 0.03).expect("zero delay config");
    let mut source_a = IdSource::default();
    let mut source_b = IdSource::default();
    let observation = |tag| Observation {
        edges: [0, 0, 10, 10],
        data: tag,
    };
    let mut first_a = Track::new(&config, &mut source_a, "car", observation(0));
    let mut first_b = Track::new(&config, &mut source_b, "car", observation(1));
    let mut second_a = Track::new(&config, &mut source_a, "car", observation(2));
    let mut second_b = Track::new(&config, &mut source_b, "car", observation(3));
    assert_eq!(first_a.initializing_id(), 1);
    assert_eq!(first_b.initializing_id(), 1);
    assert_eq!(second_a.initializing_id(), 2);
    assert_eq!(second_b.initializing_id(), 2);
    assert_eq!(first_a.id(), None);
    assert_eq!(first_b.id(), None);
    assert_eq!(second_a.id(), None);
    assert_eq!(second_b.id(), None);
    first_a.hit(&mut source_a, observation(4));
    first_a.hit(&mut source_a, observation(5));
    assert_eq!(
        first_a.id(),
        Some(1),
        "first track on source A takes the first id"
    );
    second_b.hit(&mut source_b, observation(6));
    second_b.hit(&mut source_b, observation(7));
    assert_eq!(
        second_b.id(),
        Some(1),
        "later-created track on source B initializes first and takes id 1"
    );
    second_a.hit(&mut source_a, observation(8));
    second_a.hit(&mut source_a, observation(9));
    assert_eq!(
        second_a.id(),
        Some(2),
        "second track on source A becomes real after the first"
    );
    first_b.hit(&mut source_b, observation(10));
    first_b.hit(&mut source_b, observation(11));
    assert_eq!(
        first_b.id(),
        Some(2),
        "first track on source B becomes real after the second"
    );
    let real_a = Track::new(&zero_delay, &mut source_a, "car", observation(12));
    let real_b = Track::new(&zero_delay, &mut source_b, "car", observation(13));
    assert_eq!(
        real_a.id(),
        Some(3),
        "source A keeps counting after both earlier ids"
    );
    assert_eq!(
        real_b.id(),
        Some(3),
        "source B keeps counting after both earlier ids"
    );
    assert_eq!(real_a.initializing_id(), 3);
    assert_eq!(real_b.initializing_id(), 3);
    assert_eq!(
        first_a.initializing_id(),
        1,
        "source A creations did not shift source A ids"
    );
    assert_eq!(
        second_a.id(),
        Some(2),
        "source A ids stayed after source B creations"
    );
    assert_eq!(
        first_b.id(),
        Some(2),
        "source B ids stayed after source A creations"
    );
}

#[test]
fn config_is_refused_when_delay_reaches_the_cap() {
    assert!(TrackConfig::new(25, 25, 4, 3.4, 0.03).is_none());
    assert!(TrackConfig::new(0, 0, 4, 3.4, 0.03).is_none());
    assert!(TrackConfig::new(3, 5, 4, 3.4, 0.03).is_none());
    assert!(TrackConfig::new(2, 2, 4, 3.4, 0.03).is_none());
    assert!(TrackConfig::new(25, 24, 4, 3.4, 0.03).is_some());
    assert!(TrackConfig::new(1, 0, 4, 3.4, 0.03).is_some());
    assert!(TrackConfig::new(2, 1, 4, 3.4, 0.03).is_some());
    assert!(TrackConfig::new(26, 0, 4, 3.4, 0.03).is_some());
    assert!(
        TrackConfig::new(25, 2, 0, 3.4, 0.03).is_some(),
        "a past length of zero is accepted"
    );
}

#[test]
fn long_runs_keep_counting_without_overflow() {
    let config = TrackConfig::new(25, 2, 4, 3.4, 0.03).expect("config");
    let mut ids = IdSource::default();
    let mut track = Track::new(
        &config,
        &mut ids,
        "car",
        Observation {
            edges: [0, 0, 10, 10],
            data: 0,
        },
    );
    for _ in 0..100_000 {
        track.step();
    }
    assert_eq!(track.hit_counter(), -99_999);
    assert_eq!(track.age(), 100_000);
    track.hit(
        &mut ids,
        Observation {
            edges: [1, 1, 11, 11],
            data: 1,
        },
    );
    assert_eq!(track.hit_counter(), -99_997);
    for _ in 0..100_000 {
        track.hit(
            &mut ids,
            Observation {
                edges: [2, 2, 12, 12],
                data: 2,
            },
        );
    }
    assert_eq!(track.hit_counter(), 25);
}

#[test]
fn extreme_boxes_never_panic() {
    let config = TrackConfig::new(25, 2, 4, 3.4, 0.03).expect("config");
    let mut ids = IdSource::default();
    let lowest = [i32::MIN, i32::MIN, i32::MAX, i32::MAX];
    let highest = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
    for edges in [lowest, highest, [i32::MIN, i32::MAX, i32::MIN, i32::MAX]] {
        let mut track = Track::new(&config, &mut ids, "car", Observation { edges, data: 0 });
        track.step();
        track.hit(&mut ids, Observation { edges, data: 1 });
    }
}

fn actual_state(track: &Track<u64>) -> Vec<String> {
    let mut state: Vec<String> = vec![
        track.label().to_string(),
        track.hit_counter().to_string(),
        track.age().to_string(),
        flag(track.is_initializing()).to_string(),
        track.initializing_id().to_string(),
        track
            .id()
            .map_or_else(|| "-".to_string(), |id| id.to_string()),
        flag(track.hit_counter_is_positive()).to_string(),
        flag(track.is_active()).to_string(),
        track.last_detection().data.to_string(),
        past_joined(track),
    ];
    state.extend(track.estimate().iter().map(|value| hex_bits(*value)));
    state.extend(track.velocity().iter().map(|value| hex_bits(*value)));
    state
}

fn fixture_scenario(line: &str, line_number: usize) -> (u64, i64, u32, u32, TrackConfig) {
    let fields: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(
        fields.len(),
        12,
        "line {line_number}: scenario header: {line}"
    );
    assert_eq!(fields[0], "scenario", "line {line_number}: {line}");
    let number: u64 = fields[1].parse().expect("scenario number");
    let hit_counter_max: u32 = fields[3].parse().expect("scenario cap");
    let delay: u32 = fields[5].parse().expect("scenario delay");
    let past_length: u32 = fields[7].parse().expect("scenario past length");
    let r_bits = u64::from_str_radix(fields[9], 16).expect("scenario r bits");
    let q_bits = u64::from_str_radix(fields[11], 16).expect("scenario q bits");
    let config = TrackConfig::new(
        hit_counter_max,
        delay,
        past_length,
        f64::from_bits(r_bits),
        f64::from_bits(q_bits),
    )
    .unwrap_or_else(|| panic!("line {line_number}: config refused: {line}"));
    (
        number,
        i64::from(hit_counter_max),
        delay,
        past_length,
        config,
    )
}

struct ReplayContext<'a> {
    config: &'a TrackConfig,
    cap: i64,
    past_length: u32,
    stats: &'a mut BTreeMap<String, u64>,
    tracks: &'a mut Vec<Track<u64>>,
    ids: &'a mut IdSource,
    created: usize,
}

fn replay_fixture_op(
    ctx: &mut ReplayContext,
    left: &str,
    expected_label: &str,
    line_number: usize,
) -> usize {
    let fields: Vec<&str> = left.split_whitespace().collect();
    match fields.first().copied().unwrap_or_default() {
        "new" => {
            assert_eq!(fields.len(), 8, "line {line_number}: new fields: {left}");
            let object: usize = fields[1].parse().expect("new object index");
            assert_eq!(
                object, ctx.created,
                "line {line_number}: new object order: {left}"
            );
            ctx.created += 1;
            let tag: u64 = fields[2].parse().expect("new tag");
            let edges = [
                fields[4].parse::<i32>().expect("new x1"),
                fields[5].parse::<i32>().expect("new y1"),
                fields[6].parse::<i32>().expect("new x2"),
                fields[7].parse::<i32>().expect("new y2"),
            ];
            ctx.tracks.push(Track::new(
                ctx.config,
                ctx.ids,
                fields[3],
                Observation { edges, data: tag },
            ));
            ctx.tracks.len() - 1
        }
        "step" => {
            assert_eq!(fields.len(), 2, "line {line_number}: step fields: {left}");
            let object: usize = fields[1].parse().expect("step object index");
            ctx.tracks[object].step();
            object
        }
        "hit" => {
            assert_eq!(fields.len(), 8, "line {line_number}: hit fields: {left}");
            let object: usize = fields[1].parse().expect("hit object index");
            let tag: u64 = fields[2].parse().expect("hit tag");
            let label = fields[3];
            let before_counter = ctx.tracks[object].hit_counter();
            let before_initializing = ctx.tracks[object].is_initializing();
            let full_before = ctx.past_length > 0
                && ctx.tracks[object].past_detections().len()
                    == usize::try_from(ctx.past_length).unwrap_or(usize::MAX);
            let edges = [
                fields[4].parse::<i32>().expect("hit x1"),
                fields[5].parse::<i32>().expect("hit y1"),
                fields[6].parse::<i32>().expect("hit x2"),
                fields[7].parse::<i32>().expect("hit y2"),
            ];
            ctx.tracks[object].hit(ctx.ids, Observation { edges, data: tag });
            if before_initializing && !ctx.tracks[object].is_initializing() {
                bump(ctx.stats, "uninit");
            }
            if before_counter.saturating_add(2) > ctx.cap
                && ctx.tracks[object].hit_counter() == ctx.cap
            {
                bump(ctx.stats, "capfull");
            }
            if before_counter < 0 {
                bump(ctx.stats, "hitneg");
                if before_counter == -1 {
                    bump(ctx.stats, "hitneg1");
                }
            }
            if full_before {
                let kept = ctx.tracks[object]
                    .past_detections()
                    .last()
                    .is_some_and(|(_, observation)| observation.data == tag);
                bump(ctx.stats, if kept { "pastreplace" } else { "pastsame" });
            }
            if label != expected_label {
                bump(ctx.stats, "labeldiff");
            }
            object
        }
        other => panic!("line {line_number}: unknown operation {other}: {left}"),
    }
}

fn ids_out_of_order(tracks: &[Track<u64>]) -> bool {
    let mut out = false;
    for early in 0..tracks.len() {
        for late in early + 1..tracks.len() {
            let (Some(first_id), Some(second_id)) = (tracks[early].id(), tracks[late].id()) else {
                continue;
            };
            if second_id < first_id {
                out = true;
            }
        }
    }
    out
}

#[test]
fn fixture_matches_the_real_library() {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    let mut header_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut floors: BTreeMap<String, u64> = BTreeMap::new();
    let mut stats: BTreeMap<String, u64> = BTreeMap::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        let line_number = index;
        assert!(
            !line.is_empty(),
            "line {line_number}: empty line in the fixture"
        );
        if let Some(rest) = line.strip_prefix("# counts ") {
            header_counts = parse_pairs(rest, line_number);
            continue;
        }
        if let Some(rest) = line.strip_prefix("# floors ") {
            floors = parse_pairs(rest, line_number);
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let (scenario_number, cap, delay, past_length, config) =
            fixture_scenario(line, line_number);
        bump(&mut stats, "scenarios");
        if delay <= 3 {
            bump(&mut stats, &format!("delay{delay}"));
        }
        if past_length == 0 {
            bump(&mut stats, "past0");
        }
        if past_length == 1 {
            bump(&mut stats, "past1");
        }
        let mut ids = IdSource::default();
        let mut tracks: Vec<Track<u64>> = Vec::new();
        let mut operations = 0usize;
        let mut ctx = ReplayContext {
            config: &config,
            cap,
            past_length,
            stats: &mut stats,
            tracks: &mut tracks,
            ids: &mut ids,
            created: 0,
        };
        loop {
            let op_line = lines[index];
            index += 1;
            let line_number = index;
            if op_line == "end" {
                break;
            }
            let (left, state_text) = op_line.split_once(" => ").unwrap_or_else(|| {
                panic!("line {line_number}: operation without state: {op_line}")
            });
            let expected: Vec<String> = state_text.split(' ').map(str::to_string).collect();
            assert_eq!(
                expected.len(),
                18,
                "line {line_number}: state fields: {op_line}"
            );
            let expected_label = expected[0].clone();
            bump(ctx.stats, "operations");
            operations += 1;
            let track_index = replay_fixture_op(&mut ctx, left, &expected_label, line_number);
            if ctx.tracks[track_index].hit_counter() < 0 {
                bump(ctx.stats, "negops");
            }
            assert_eq!(
                actual_state(&ctx.tracks[track_index]),
                expected,
                "scenario {scenario_number} operation {operations} at line {line_number}: mismatch of full state"
            );
        }
        let created = ctx.created;
        let out_of_order = ids_out_of_order(ctx.tracks);
        bump(&mut stats, &format!("obj{created}"));
        if out_of_order {
            bump(&mut stats, "idoutoforder");
        }
        assert!(
            operations >= 6,
            "scenario {scenario_number}: {operations} operations"
        );
    }
    check_fixture_stats_and_floors(&header_counts, &floors, &stats);
}

fn check_fixture_stats_and_floors(
    header_counts: &BTreeMap<String, u64>,
    floors: &BTreeMap<String, u64>,
    stats: &BTreeMap<String, u64>,
) {
    assert_eq!(stats.get("scenarios"), Some(&300), "scenario count");
    assert_eq!(
        header_counts, stats,
        "fixture header counts differ from the recomputed ones"
    );
    for (name, floor) in floors {
        let reached = stats.get(name).copied().unwrap_or_default();
        assert!(
            reached >= *floor,
            "floor {name}: recomputed {reached} < stated {floor}"
        );
    }
    assert_eq!(floors.len(), 20, "fixture floor lines");
}

fn parse_pairs(text: &str, line_number: usize) -> BTreeMap<String, u64> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    assert!(
        tokens.len().is_multiple_of(2),
        "line {line_number}: pairs expected"
    );
    let mut pairs = BTreeMap::new();
    let mut position = 0;
    while position < tokens.len() {
        let value = tokens[position + 1].parse().expect("pair value");
        pairs.insert(tokens[position].to_string(), value);
        position += 2;
    }
    pairs
}

fn bump(stats: &mut BTreeMap<String, u64>, name: &str) {
    *stats.entry(name.to_string()).or_insert(0) += 1;
}
