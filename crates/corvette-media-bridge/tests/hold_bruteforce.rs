use corvette_media_bridge::index::Index;
use rusqlite::{Connection, params};
use std::path::PathBuf;

const DAY: i64 = 86_400_000;
const NOW: i64 = 1_790_000_000_000;
const START: i64 = NOW - 30 * DAY;
const SEED: i64 = 0x2545_F491_4F6C_DD1D;
const PIECE_MAX_MS: i64 = 60_000;
const CAMERAS: [&str; 3] = ["front", "back", "yard"];
const PIECES_PER_CAMERA: u64 = 1_200;
const REVIEWS_PER_CAMERA: i64 = 150;

#[derive(Clone)]
struct FixturePiece {
    camera: String,
    seq: u64,
    start: i64,
    end: i64,
}

#[derive(Clone)]
struct FixtureReview {
    camera: String,
    start: i64,
    end: Option<i64>,
    last_activity: i64,
}

const fn roll(rng: &mut i64, bound: i64) -> i64 {
    let mut x = *rng;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *rng = x;
    (x & i64::MAX) % bound
}

fn fixture() -> (Vec<FixturePiece>, Vec<FixtureReview>) {
    let mut rng = SEED;
    let mut pieces: Vec<FixturePiece> = Vec::new();
    let mut reviews: Vec<FixtureReview> = Vec::new();
    for camera in CAMERAS {
        let camera_start = pieces.len();
        let mut cursor = START;
        for seq in 0..PIECES_PER_CAMERA {
            let duration = 1 + roll(&mut rng, PIECE_MAX_MS);
            pieces.push(FixturePiece {
                camera: camera.to_owned(),
                seq,
                start: cursor,
                end: cursor + duration,
            });
            cursor += duration;
        }
        for _ in 0..REVIEWS_PER_CAMERA {
            let start = START + roll(&mut rng, 28 * DAY);
            let open = roll(&mut rng, 4) == 0;
            let end = if open {
                None
            } else {
                Some(start + roll(&mut rng, 3 * DAY))
            };
            let last_activity = end.unwrap_or_else(|| start + roll(&mut rng, 3 * DAY));
            reviews.push(FixtureReview {
                camera: camera.to_owned(),
                start,
                end,
                last_activity,
            });
        }
        for piece in pieces[camera_start..].iter().take(40) {
            for (start, end) in [
                (piece.end, piece.end + 5000),
                (piece.end - 1, piece.end + 5000),
                (piece.start, piece.start + 1),
                (piece.start - 5000, piece.start),
                (piece.start - 5000, piece.start + 1),
            ] {
                reviews.push(FixtureReview {
                    camera: camera.to_owned(),
                    start,
                    end: Some(end),
                    last_activity: end,
                });
            }
        }
    }
    (pieces, reviews)
}

fn base(review: &FixtureReview) -> i64 {
    review.end.unwrap_or(review.last_activity)
}

fn expected_held(
    pieces: &[FixturePiece],
    reviews: &[FixtureReview],
    camera: &str,
    floor: i64,
) -> Vec<u64> {
    let mut held: Vec<u64> = pieces
        .iter()
        .filter(|piece| piece.camera == camera)
        .filter(|piece| {
            reviews.iter().any(|review| {
                review.camera == camera
                    && floor <= base(review)
                    && piece.end > review.start
                    && piece.start < base(review)
            })
        })
        .map(|piece| piece.seq)
        .collect();
    held.sort_unstable();
    held
}

fn fresh_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("corvette-hold-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("test root created");
    dir
}

#[test]
fn held_seqs_equals_a_brute_force_overlap_and_age_scan_for_every_camera_and_age_floor() {
    let (pieces, reviews) = fixture();
    let root = fresh_root("bruteforce");
    let cameras = CAMERAS
        .iter()
        .map(|camera| ((*camera).to_owned(), 4))
        .collect::<Vec<_>>();
    let index = Index::open(&root.join("index.sqlite"), &cameras).expect("index opens");
    {
        let conn = Connection::open(root.join("index.sqlite")).expect("raw connection opens");
        conn.execute_batch("BEGIN").expect("transaction begins");
        for piece in &pieces {
            conn.execute(
                "INSERT INTO pieces (camera, seq, start_ts, end_ts, duration_s, path, size_bytes, codec)
                 VALUES (?1, ?2, ?3, ?4, 4.0, '', 1000, 'h264')",
                params![
                    piece.camera,
                    i64::try_from(piece.seq).expect("test seqs fit"),
                    piece.start,
                    piece.end
                ],
            )
            .expect("piece inserted");
        }
        for review in &reviews {
            conn.execute(
                "INSERT INTO reviews (camera, severity, start_ts, end_ts, last_activity_ts)
                 VALUES (?1, 'alert', ?2, ?3, ?4)",
                params![
                    review.camera,
                    review.start,
                    review.end,
                    review.last_activity
                ],
            )
            .expect("review inserted");
        }
        conn.execute_batch("COMMIT").expect("transaction commits");
    }
    for floor in [i64::MIN, START, NOW - 10 * DAY, NOW + DAY] {
        for camera in CAMERAS {
            let want = expected_held(&pieces, &reviews, camera, floor);
            let mut got = index.held_seqs(camera, floor).expect("held seqs read");
            got.sort_unstable();
            assert_eq!(got, want, "camera {camera} floor {floor}");
        }
    }
    std::fs::remove_dir_all(&root).expect("temp root removed");
}
