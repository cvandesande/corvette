use corvette_media_bridge::index::{Index, REVIEWS_HOLD_SQL};
use rusqlite::{Connection, params};
use std::time::Instant;

const DAY_MS: i64 = 86_400_000;
const NOW: i64 = 1_790_000_000_000;
const PIECE_MS: i64 = 4_000;
const REVIEW_MS: i64 = 5_000;

fn arg(index: usize, fallback: i64) -> i64 {
    std::env::args()
        .nth(index)
        .and_then(|value| value.parse().ok())
        .unwrap_or(fallback)
}

fn main() {
    let pieces = arg(1, 100_000);
    let reviews = arg(2, 1_000);
    let dir = std::env::temp_dir().join(format!("corvette-hold-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("bench root created");
    let path = dir.join("index.sqlite");
    let index = Index::open(&path, &[("front".to_owned(), 4)]).expect("index opens");
    let window_ms = pieces * PIECE_MS;
    {
        let conn = Connection::open(&path).expect("raw connection opens");
        conn.execute_batch("BEGIN").expect("transaction begins");
        {
            let mut insert = conn
                .prepare(
                    "INSERT INTO pieces (camera, seq, start_ts, end_ts, duration_s, path, size_bytes, codec)
                     VALUES ('front', ?1, ?2, ?3, 4.0, '', 1000, 'h264')",
                )
                .expect("piece insert prepares");
            for seq in 0..pieces {
                let start = NOW - window_ms + seq * PIECE_MS;
                insert
                    .execute(params![seq, start, start + PIECE_MS])
                    .expect("piece inserted");
            }
        }
        {
            let mut insert = conn
                .prepare(
                    "INSERT INTO reviews (camera, severity, start_ts, end_ts, last_activity_ts)
                     VALUES ('front', 'alert', ?1, ?2, ?2)",
                )
                .expect("review insert prepares");
            for review in 0..reviews {
                let start = NOW - window_ms + (review + 1) * (window_ms / (reviews + 1));
                insert
                    .execute(params![start, start + REVIEW_MS])
                    .expect("review inserted");
            }
        }
        conn.execute_batch("COMMIT").expect("transaction commits");
        let mut explain = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {REVIEWS_HOLD_SQL}"))
            .expect("plan prepares");
        let camera = "front".to_owned();
        let mut bind: Vec<&dyn rusqlite::ToSql> = vec![&camera, &0i64];
        if explain.parameter_count() > 2 {
            bind.push(&4_000i64);
        }
        let details = explain
            .query_map(bind.as_slice(), |row| row.get::<_, String>(3))
            .expect("plan runs")
            .collect::<Result<Vec<String>, _>>()
            .expect("plan rows read");
        for detail in &details {
            println!("plan: {detail}");
        }
    }
    let floor = NOW - 30 * DAY_MS;
    let started = Instant::now();
    let held = index.held_seqs("front", floor).expect("hold query runs");
    println!(
        "pieces={} reviews={} held={} hold_millis={}",
        pieces,
        reviews,
        held.len(),
        started.elapsed().as_millis()
    );
    std::fs::remove_dir_all(&dir).expect("bench root removed");
}
