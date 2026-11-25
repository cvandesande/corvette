//! Addresses, plan and answers of the playlist route directory for one camera range.

use crate::activity::{MAX_RANGE_SECONDS, parse_timestamp, percent_decode};
use crate::config::ConfigSource;
use crate::rebase::{self, FragmentPart, PieceParts, Timeline};
use crate::recordings::to_millis;
use crate::{
    Request, Response, bad_request, config_unavailable, error_response, log_event, no_footage,
    recording_unavailable,
};
use rusqlite::{Connection, Row, params};
use std::borrow::Cow;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

const MAX_INIT_BYTES: u64 = 1024 * 1024;
const MAX_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PLAN_PIECES: i64 = 25_000;
const GAP_AFTER_MS: i64 = 1_000;
const DAY_MS: i64 = 86_400_000;
const MEDIA_TYPE_PLAYLIST: &str = "application/vnd.apple.mpegurl";
const MEDIA_TYPE_MP4: &str = "video/mp4";

const PLAN_SQL: &str = "SELECT seq, start_ts, end_ts, duration_s, path, size_bytes \
     FROM pieces WHERE camera = ?1 AND start_ts < ?2 AND end_ts > ?3 ORDER BY start_ts, seq";

const PLAN_COUNT_SQL: &str = "SELECT count(*) FROM pieces \
     WHERE camera = ?1 AND start_ts < ?2 AND end_ts > ?3";

/// One accepted file name of a playlist directory, with the parsed number the
/// `init-` and `seg-` files carry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MediaFile {
    Master,
    Index,
    Init(u64),
    Segment(u64),
}

/// The raw parts of one playlist-route address; the values are validated by `handle`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MediaPath<'a> {
    pub(crate) camera: &'a str,
    pub(crate) start: &'a str,
    pub(crate) end: &'a str,
    pub(crate) file: MediaFile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PieceRead {
    Unreadable,
    Malformed,
}

#[derive(Debug)]
enum PlanError {
    IndexUnreadable,
    PieceUnreadable { seq: i64 },
    PieceMalformed { seq: i64 },
    TooManyPieces,
}

#[derive(Debug)]
enum Answer {
    Playlist(Vec<u8>),
    Mp4(Vec<u8>),
    Mp4Partial {
        body: Vec<u8>,
        content_range: String,
    },
    Mp4Unsatisfiable {
        total: usize,
    },
    PieceUnreadable {
        seq: i64,
    },
    PieceMalformed {
        seq: i64,
    },
    IndexUnreadable,
    NotFound,
}

enum RangeAnswer {
    Whole,
    Partial { start: usize, end: usize },
    Unsatisfiable,
}

struct PieceRow {
    seq: i64,
    start_ts: i64,
    end_ts: i64,
    duration_s: f64,
    path: String,
    size_bytes: i64,
}

struct PlannedPiece {
    seq: i64,
    start_ts: i64,
    duration_s: f64,
    size_bytes: i64,
    path: PathBuf,
}

struct Plan {
    pieces: Vec<PlannedPiece>,
    timeline: Timeline,
}

/// Splits a playlist-route address into its camera, bounds and file, or `None`
/// when the path is not one address of one playlist directory.
pub(crate) fn media_parts(path: &str) -> Option<MediaPath<'_>> {
    let prefix = format!("{}media/", crate::API_PREFIX);
    let rest = path.strip_prefix(prefix.as_str())?;
    let parts: Vec<&str> = rest.split('/').collect();
    let [camera, "start", start, "end", end, file] = parts.as_slice() else {
        return None;
    };
    if camera.is_empty() || start.is_empty() || end.is_empty() {
        return None;
    }
    let file = match *file {
        "master.m3u8" => MediaFile::Master,
        "index.m3u8" => MediaFile::Index,
        name => match (
            name.strip_prefix("init-")
                .and_then(|rest| rest.strip_suffix(".mp4")),
            name.strip_prefix("seg-")
                .and_then(|rest| rest.strip_suffix(".m4s")),
        ) {
            (Some(run), _) => MediaFile::Init(plain_u64(run)?),
            (None, Some(number)) => MediaFile::Segment(plain_u64(number)?),
            (None, None) => return None,
        },
    };
    Some(MediaPath {
        camera,
        start,
        end,
        file,
    })
}

/// The answer for one validated playlist-route request.
pub(crate) async fn handle(
    request: &Request,
    config: &ConfigSource,
    index_path: &Path,
    media: MediaPath<'_>,
) -> Response {
    let camera = match percent_decode(media.camera) {
        Ok(camera) => camera,
        Err(reason) => return bad_request(&reason),
    };
    let start = match parse_timestamp("start", media.start) {
        Ok(start) => start,
        Err(reason) => return bad_request(&reason),
    };
    let end = match parse_timestamp("end", media.end) {
        Ok(end) => end,
        Err(reason) => return bad_request(&reason),
    };
    if to_millis(start) > to_millis(end) {
        return no_footage();
    }
    if end - start > MAX_RANGE_SECONDS {
        return no_footage();
    }
    let allowed = match config.allowed_camera_names(&request.remote_role).await {
        Ok(cameras) => cameras,
        Err(err) => return config_unavailable(&err),
    };
    if !allowed.contains(&camera) {
        return no_footage();
    }
    let range = request.range.clone();
    let camera_for_plan = camera.clone();
    let file = media.file;
    let index_path = index_path.to_path_buf();
    let root = index_path
        .parent()
        .map_or_else(PathBuf::new, Path::to_path_buf);
    // rusqlite and the piece reads are synchronous: the plan moves off the async worker.
    let planned = tokio::task::spawn_blocking(move || {
        let start_ms = to_millis(start);
        let end_ms = to_millis(end);
        let Ok(connection) = crate::database::open_read_only(&index_path) else {
            return Answer::IndexUnreadable;
        };
        match plan_range(&connection, &root, &camera_for_plan, start_ms, end_ms) {
            Err(PlanError::TooManyPieces) => Answer::NotFound,
            Err(PlanError::IndexUnreadable) => Answer::IndexUnreadable,
            Err(PlanError::PieceUnreadable { seq }) => Answer::PieceUnreadable { seq },
            Err(PlanError::PieceMalformed { seq }) => Answer::PieceMalformed { seq },
            Ok(plan) => serve(&plan, file, range.as_deref()),
        }
    })
    .await;
    let Ok(answer) = planned else {
        return recording_unavailable();
    };
    answer_response(answer, &camera)
}

fn answer_response(answer: Answer, camera: &str) -> Response {
    match answer {
        Answer::Playlist(body) => Response {
            status: Cow::Borrowed("200 OK"),
            body,
            content_type: Cow::Borrowed(MEDIA_TYPE_PLAYLIST),
            cache_control: Cow::Borrowed("no-store"),
            extra: Vec::new(),
        },
        Answer::Mp4(body) => Response {
            status: Cow::Borrowed("200 OK"),
            body,
            content_type: Cow::Borrowed(MEDIA_TYPE_MP4),
            cache_control: Cow::Borrowed("no-store"),
            extra: vec![(Cow::Borrowed("Accept-Ranges"), "bytes".to_owned())],
        },
        Answer::Mp4Partial {
            body,
            content_range,
        } => Response {
            status: Cow::Borrowed("206 Partial Content"),
            body,
            content_type: Cow::Borrowed(MEDIA_TYPE_MP4),
            cache_control: Cow::Borrowed("no-store"),
            extra: vec![(Cow::Borrowed("Content-Range"), content_range)],
        },
        Answer::Mp4Unsatisfiable { total } => {
            let mut response = error_response(
                "416 Range Not Satisfiable",
                "range_unsatisfiable",
                "The requested byte range is not available.",
            );
            response
                .extra
                .push((Cow::Borrowed("Content-Range"), format!("bytes */{total}")));
            response
        }
        Answer::PieceUnreadable { seq } => {
            log_event(
                "playlist",
                "recording-piece-unreadable",
                format!("camera={camera} seq={seq}"),
            );
            recording_unavailable()
        }
        Answer::PieceMalformed { seq } => {
            log_event(
                "playlist",
                "recording-piece-malformed",
                format!("camera={camera} seq={seq}"),
            );
            recording_unavailable()
        }
        Answer::IndexUnreadable => recording_unavailable(),
        Answer::NotFound => no_footage(),
    }
}

fn plan_range(
    connection: &Connection,
    root: &Path,
    camera: &str,
    start_ms: i64,
    end_ms: i64,
) -> Result<Plan, PlanError> {
    let count: i64 = connection
        .query_row(PLAN_COUNT_SQL, params![camera, end_ms, start_ms], |row| {
            row.get(0)
        })
        .map_err(|_| PlanError::IndexUnreadable)?;
    if count > MAX_PLAN_PIECES {
        return Err(PlanError::TooManyPieces);
    }
    let mut statement = connection
        .prepare(PLAN_SQL)
        .map_err(|_| PlanError::IndexUnreadable)?;
    let rows = statement
        .query_map(params![camera, end_ms, start_ms], read_row)
        .map_err(|_| PlanError::IndexUnreadable)?;
    let mut plan = Plan {
        pieces: Vec::new(),
        timeline: Timeline::new(),
    };
    let mut previous_end: Option<i64> = None;
    for row in rows {
        let row = row.map_err(|_| PlanError::IndexUnreadable)?;
        let relative = piece_relative_path(&row.path, camera)
            .ok_or(PlanError::PieceUnreadable { seq: row.seq })?;
        let path = root.join(relative);
        let init = read_init_prefix(&path).map_err(|err| match err {
            PieceRead::Unreadable => PlanError::PieceUnreadable { seq: row.seq },
            PieceRead::Malformed => PlanError::PieceMalformed { seq: row.seq },
        })?;
        let fingerprint =
            rebase::fingerprint(&init).map_err(|_| PlanError::PieceMalformed { seq: row.seq })?;
        // `Timeline::push` sums fragment durations; one synthetic fragment carries
        // the row's rounded duration, so no fragment is read while planning.
        let parts = PieceParts {
            init_len: 0,
            fragments: vec![FragmentPart {
                start: 0,
                end: 0,
                duration_ticks: duration_ticks(row.duration_s, fingerprint.timescale),
                base_decode_ticks: 0,
            }],
        };
        let gap_before =
            previous_end.is_some_and(|end| row.start_ts > end.saturating_add(GAP_AFTER_MS));
        plan.timeline
            .push(&parts, &fingerprint, gap_before)
            .map_err(|_| PlanError::PieceMalformed { seq: row.seq })?;
        previous_end = Some(row.end_ts);
        plan.pieces.push(PlannedPiece {
            seq: row.seq,
            start_ts: row.start_ts,
            duration_s: row.duration_s,
            size_bytes: row.size_bytes,
            path,
        });
    }
    Ok(plan)
}

fn read_row(row: &Row<'_>) -> rusqlite::Result<PieceRow> {
    Ok(PieceRow {
        seq: row.get(0)?,
        start_ts: row.get(1)?,
        end_ts: row.get(2)?,
        duration_s: row.get(3)?,
        path: row.get(4)?,
        size_bytes: row.get(5)?,
    })
}

/// The `pieces.path` layout the bridge writes — `recordings/<camera>/<date>/<hour>/<name>.mp4`
/// — as a path safe to join onto the storage root, mirroring the bridge's own check.
fn piece_relative_path<'a>(stored: &'a str, camera: &str) -> Option<&'a str> {
    let parts = Path::new(stored).components().collect::<Vec<_>>();
    let [
        Component::Normal(recordings),
        Component::Normal(second),
        Component::Normal(_),
        Component::Normal(_),
        Component::Normal(name),
    ] = parts.as_slice()
    else {
        return None;
    };
    (*recordings == "recordings" && *second == camera && name.to_string_lossy().ends_with(".mp4"))
        .then_some(stored)
}

/// Reads a piece's init prefix by walking top-level box headers only, bounded to
/// [`MAX_INIT_BYTES`]: the file body beyond the init prefix is never read.
fn read_init_prefix(path: &Path) -> Result<Vec<u8>, PieceRead> {
    let mut file = File::open(path).map_err(|_| PieceRead::Unreadable)?;
    let mut header = [0_u8; 16];
    let mut start: u64 = 0;
    let init_len = loop {
        read_exact_at(&mut file, start, &mut header[..8])?;
        let size = u32::from_be_bytes(header[..4].try_into().map_err(|_| PieceRead::Malformed)?);
        let kind: [u8; 4] = header[4..8].try_into().map_err(|_| PieceRead::Malformed)?;
        let total = match size {
            0 => return Err(PieceRead::Malformed),
            1 => {
                read_exact_at(&mut file, start + 8, &mut header[8..])?;
                let large =
                    u64::from_be_bytes(header[8..16].try_into().map_err(|_| PieceRead::Malformed)?);
                if large < 16 {
                    return Err(PieceRead::Malformed);
                }
                large
            }
            other => {
                if other < 8 {
                    return Err(PieceRead::Malformed);
                }
                u64::from(other)
            }
        };
        if kind == *b"moof" {
            if start > MAX_INIT_BYTES {
                return Err(PieceRead::Malformed);
            }
            break start;
        }
        start = start.saturating_add(total);
        if start > MAX_INIT_BYTES {
            return Err(PieceRead::Malformed);
        }
    };
    let mut init = vec![0_u8; usize::try_from(init_len).map_err(|_| PieceRead::Malformed)?];
    read_exact_at(&mut file, 0, &mut init)?;
    Ok(init)
}

fn read_exact_at(file: &mut File, at: u64, buffer: &mut [u8]) -> Result<(), PieceRead> {
    file.seek(SeekFrom::Start(at))
        .map_err(|_| PieceRead::Unreadable)?;
    match file.read_exact(buffer) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::UnexpectedEof => Err(PieceRead::Malformed),
        Err(_) => Err(PieceRead::Unreadable),
    }
}

fn segment_body(path: &Path, offset_ticks: u64) -> Result<Vec<u8>, PieceRead> {
    let length = std::fs::metadata(path)
        .map_err(|_| PieceRead::Unreadable)?
        .len();
    if length > MAX_SEGMENT_BYTES {
        return Err(PieceRead::Unreadable);
    }
    let bytes = std::fs::read(path).map_err(|_| PieceRead::Unreadable)?;
    let parts = rebase::split_piece(&bytes).map_err(|_| PieceRead::Malformed)?;
    let mut out = Vec::with_capacity(bytes.len());
    for fragment in &parts.fragments {
        let slice = bytes
            .get(fragment.start..fragment.end)
            .ok_or(PieceRead::Malformed)?;
        out.extend_from_slice(
            &rebase::rebase_fragment(slice, offset_ticks).map_err(|_| PieceRead::Malformed)?,
        );
    }
    Ok(out)
}

fn serve(plan: &Plan, file: MediaFile, range: Option<&str>) -> Answer {
    if plan.pieces.is_empty() {
        return Answer::NotFound;
    }
    match file {
        MediaFile::Master => Answer::Playlist(master_text(plan)),
        MediaFile::Index => Answer::Playlist(index_text(plan)),
        MediaFile::Init(run) => {
            let piece = usize::try_from(run)
                .ok()
                .and_then(|run| plan.timeline.run_first_pieces().get(run).copied())
                .and_then(|first| plan.pieces.get(first));
            let Some(piece) = piece else {
                return Answer::NotFound;
            };
            match read_init_prefix(&piece.path) {
                Ok(bytes) => Answer::Mp4(bytes),
                Err(PieceRead::Unreadable) => Answer::PieceUnreadable { seq: piece.seq },
                Err(PieceRead::Malformed) => Answer::PieceMalformed { seq: piece.seq },
            }
        }
        MediaFile::Segment(number) => {
            let placed_pair = usize::try_from(number).ok().and_then(|number| {
                Some((
                    plan.pieces.get(number)?,
                    plan.timeline.pieces().get(number)?,
                ))
            });
            let Some((piece, placed)) = placed_pair else {
                return Answer::NotFound;
            };
            match segment_body(&piece.path, placed.offset_ticks) {
                Ok(body) => match parse_range(range, body.len()) {
                    RangeAnswer::Whole => Answer::Mp4(body),
                    RangeAnswer::Partial { start, end } => Answer::Mp4Partial {
                        content_range: format!("bytes {start}-{end}/{}", body.len()),
                        body: body[start..=end].to_vec(),
                    },
                    RangeAnswer::Unsatisfiable => Answer::Mp4Unsatisfiable { total: body.len() },
                },
                Err(PieceRead::Unreadable) => Answer::PieceUnreadable { seq: piece.seq },
                Err(PieceRead::Malformed) => Answer::PieceMalformed { seq: piece.seq },
            }
        }
    }
}

/// The recorder truncates each piece's media ticks to a whole millisecond for
/// `duration_s`; rounding back to the media timescale places the next piece so
/// each junction overlaps the following piece by less than one millisecond.
fn duration_ticks(duration_s: f64, timescale: u32) -> u64 {
    rounded_u64(duration_s.max(0.0) * f64::from(timescale), u64::MAX)
}

fn rounded_u64(value: f64, fallback: u64) -> u64 {
    format!("{:.0}", value.round())
        .parse::<u64>()
        .unwrap_or(fallback)
}

fn master_text(plan: &Plan) -> Vec<u8> {
    let total_bytes = plan
        .pieces
        .iter()
        .map(|piece| u64::try_from(piece.size_bytes.max(0)).unwrap_or(0))
        .fold(0_u64, u64::saturating_add);
    let total_ms = rounded_u64(
        plan.pieces
            .iter()
            .map(|piece| piece.duration_s.max(0.0) * 1_000.0)
            .fold(0_f64, |sum, value| sum + value),
        0,
    );
    let bandwidth = total_bytes
        .saturating_mul(8)
        .saturating_mul(1_000)
        .checked_div(total_ms)
        .unwrap_or(0);
    format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH={bandwidth}\nindex.m3u8\n").into_bytes()
}

fn index_text(plan: &Plan) -> Vec<u8> {
    let longest = plan
        .pieces
        .iter()
        .map(|piece| piece.duration_s.max(0.0))
        .fold(0_f64, f64::max);
    let target_duration = rounded_u64(longest.ceil(), u64::MAX);
    let mut out = String::from(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-INDEPENDENT-SEGMENTS\n",
    );
    let _ = writeln!(out, "#EXT-X-TARGETDURATION:{target_duration}");
    let mut previous_run: Option<usize> = None;
    for (number, (piece, placed)) in plan.pieces.iter().zip(plan.timeline.pieces()).enumerate() {
        let boundary = number == 0 || placed.has_discontinuity_before;
        if placed.has_discontinuity_before {
            out.push_str("#EXT-X-DISCONTINUITY\n");
        }
        if boundary || previous_run != Some(placed.run_index) {
            let _ = writeln!(out, "#EXT-X-MAP:URI=\"init-{}.mp4\"", placed.run_index);
        }
        if boundary {
            let _ = writeln!(
                out,
                "#EXT-X-PROGRAM-DATE-TIME:{}",
                program_date_time(piece.start_ts)
            );
        }
        previous_run = Some(placed.run_index);
        let _ = writeln!(out, "#EXTINF:{:.3},", piece.duration_s.max(0.0));
        let _ = writeln!(out, "seg-{number}.m4s");
    }
    out.push_str("#EXT-X-ENDLIST\n");
    out.into_bytes()
}

fn program_date_time(start_ms: i64) -> String {
    let days = start_ms.div_euclid(DAY_MS);
    let rest = start_ms.rem_euclid(DAY_MS);
    let hours = rest / 3_600_000;
    let minutes = rest / 60_000 % 60;
    let seconds = rest / 1_000 % 60;
    let millis = rest % 1_000;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
}

/// Days since the Unix epoch to the proleptic Gregorian date (Howard Hinnant's algorithm).
const fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = (if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    }) / 146_097;
    let doe = shifted - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

fn parse_range(header: Option<&str>, length: usize) -> RangeAnswer {
    let Some(header) = header else {
        return RangeAnswer::Whole;
    };
    let Some(spec) = header.get(6..) else {
        return RangeAnswer::Whole;
    };
    if !header[..6].eq_ignore_ascii_case("bytes=") {
        return RangeAnswer::Whole;
    }
    if spec.is_empty() || spec.contains(',') {
        return RangeAnswer::Whole;
    }
    let Some((first, last)) = spec.split_once('-') else {
        return RangeAnswer::Whole;
    };
    match (first.is_empty(), last.is_empty()) {
        (true, true) => RangeAnswer::Whole,
        (true, false) => {
            let Some(suffix) = plain_usize(last) else {
                return RangeAnswer::Whole;
            };
            if suffix == 0 {
                return RangeAnswer::Whole;
            }
            let Some(last_byte) = length.checked_sub(1) else {
                return RangeAnswer::Unsatisfiable;
            };
            RangeAnswer::Partial {
                start: length.saturating_sub(suffix),
                end: last_byte,
            }
        }
        (false, true) => match plain_usize(first) {
            Some(start) if start < length => RangeAnswer::Partial {
                start,
                end: length - 1,
            },
            Some(_) => RangeAnswer::Unsatisfiable,
            None => RangeAnswer::Whole,
        },
        (false, false) => match (plain_usize(first), plain_usize(last)) {
            (Some(start), Some(end)) if start > end => RangeAnswer::Whole,
            (Some(start), Some(_)) if start >= length => RangeAnswer::Unsatisfiable,
            (Some(start), Some(end)) => RangeAnswer::Partial {
                start,
                end: end.min(length - 1),
            },
            _ => RangeAnswer::Whole,
        },
    }
}

fn plain_usize(text: &str) -> Option<usize> {
    (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse::<usize>().ok())
        .flatten()
}

fn plain_u64(text: &str) -> Option<u64> {
    (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse::<u64>().ok())
        .flatten()
}
