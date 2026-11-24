//! Reconciles the visible recording tree with the index after a crash, before
//! any piece producer runs: each of the three states the design names has one.

use crate::index::{Index, IndexError, PieceRow};
use crate::piece_walk;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub inserted: u32,
    pub rows_dropped: u32,
    pub part_removed: u32,
    pub trash_removed: u32,
    pub quarantined: u32,
    pub skipped_dirs: u32,
    pub rows_refused: u32,
}

#[derive(Debug)]
pub enum RecoveryError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    Index {
        source: IndexError,
    },
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                f,
                "recording recovery {operation} on {} failed: {source}",
                path.display()
            ),
            Self::Index { source } => {
                write!(f, "recording recovery index write failed: {source}")
            }
        }
    }
}

impl std::error::Error for RecoveryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Index { source } => Some(source),
        }
    }
}

const fn io_error(operation: &'static str, path: PathBuf, source: std::io::Error) -> RecoveryError {
    RecoveryError::Io {
        operation,
        path,
        source,
    }
}

const fn index_error(source: IndexError) -> RecoveryError {
    RecoveryError::Index { source }
}

fn missing_directory(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}

const MAX_PIECE_BYTES: u64 = 256 * 1024 * 1024;

fn valid_camera_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn stored_path_is_safe(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

/// Where one piece file sits inside the fixed layout, with the root for
/// building the index row's root-relative path.
#[derive(Clone, Copy)]
struct PiecePlace<'a> {
    root: &'a Path,
    camera: &'a str,
    date: &'a str,
    hour: &'a str,
    name: &'a str,
    path: &'a Path,
}

/// Reconciles the recording tree with the index in one startup pass.
///
/// # Errors
/// Returns a [`RecoveryError`] when a directory, a quarantine rename, or an
/// index write fails; a file that cannot be read or parsed is quarantined
/// rather than an error.
pub fn scan(root: &Path, index: &mut Index) -> Result<ScanReport, RecoveryError> {
    let mut report = ScanReport::default();
    sweep(
        root.join("recordings/.part"),
        "part-removed",
        &mut report.part_removed,
    )?;
    sweep(
        root.join(".trash"),
        "trash-removed",
        &mut report.trash_removed,
    )?;
    let rows = index.all_pieces().map_err(index_error)?;
    let indexed: HashSet<(String, u64)> = rows
        .iter()
        .map(|row| (row.camera.clone(), row.seq))
        .collect();
    scan_camera_tree(root, &indexed, index, &mut report)?;
    drop_dangling_rows(root, index, &mut report)?;
    Ok(report)
}

fn sweep(dir: PathBuf, outcome: &str, count: &mut u32) -> Result<(), RecoveryError> {
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(source) if missing_directory(&source) => return Ok(()),
        Err(source) => return Err(io_error("list", dir, source)),
    };
    for entry in entries {
        let entry = entry.map_err(|source| io_error("list", dir.clone(), source))?;
        if !entry
            .file_type()
            .map_err(|source| io_error("inspect", entry.path(), source))?
            .is_file()
        {
            continue;
        }
        std::fs::remove_file(entry.path())
            .map_err(|source| io_error("remove swept file", entry.path(), source))?;
        eprintln!("corvette-media-bridge: recovery {outcome}");
        *count += 1;
    }
    Ok(())
}

fn sub_entries(
    dir: &Path,
    want_dir: bool,
    operation: &'static str,
) -> Result<Vec<(String, PathBuf)>, RecoveryError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(source) if missing_directory(&source) => return Ok(Vec::new()),
        Err(source) => return Err(io_error(operation, dir.to_path_buf(), source)),
    };
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| io_error(operation, dir.to_path_buf(), source))?;
        let is_dir = entry
            .file_type()
            .map_err(|source| io_error(operation, entry.path(), source))?
            .is_dir();
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if is_dir == want_dir {
            found.push((name, entry.path()));
        }
    }
    found.sort();
    Ok(found)
}

fn scan_camera_tree(
    root: &Path,
    indexed: &HashSet<(String, u64)>,
    index: &mut Index,
    report: &mut ScanReport,
) -> Result<(), RecoveryError> {
    for (camera, camera_dir) in sub_entries(&root.join("recordings"), true, "list cameras")? {
        if camera == ".part" {
            continue;
        }
        if !valid_camera_name(&camera) {
            eprintln!("corvette-media-bridge: recovery skipped-directory");
            report.skipped_dirs += 1;
            continue;
        }
        let place_root = PiecePlaceRoot {
            root,
            camera: &camera,
            camera_dir: &camera_dir,
        };
        scan_camera(place_root, indexed, index, report)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct PiecePlaceRoot<'a> {
    root: &'a Path,
    camera: &'a str,
    camera_dir: &'a Path,
}

fn scan_camera(
    scope: PiecePlaceRoot<'_>,
    indexed: &HashSet<(String, u64)>,
    index: &mut Index,
    report: &mut ScanReport,
) -> Result<(), RecoveryError> {
    for (date, date_dir) in sub_entries(scope.camera_dir, true, "list dates")? {
        for (hour, hour_dir) in sub_entries(&date_dir, true, "list hours")? {
            for (name, path) in sub_entries(&hour_dir, false, "list pieces")? {
                let place = PiecePlace {
                    root: scope.root,
                    camera: scope.camera,
                    date: &date,
                    hour: &hour,
                    name: &name,
                    path: &path,
                };
                scan_piece_file(place, indexed, index, report)?;
            }
        }
    }
    Ok(())
}

fn scan_piece_file(
    place: PiecePlace<'_>,
    indexed: &HashSet<(String, u64)>,
    index: &mut Index,
    report: &mut ScanReport,
) -> Result<(), RecoveryError> {
    let Some(stem) = place.name.strip_suffix(".mp4") else {
        return quarantine(&place, Quarantined::NameUnparseable, report);
    };
    let Some((start_text, seq_text)) = stem.split_once('-') else {
        return quarantine(&place, Quarantined::NameUnparseable, report);
    };
    let Ok(start_ms) = start_text.parse::<i64>() else {
        return quarantine(&place, Quarantined::NameUnparseable, report);
    };
    let Ok(seq) = seq_text.parse::<u64>() else {
        return quarantine(&place, Quarantined::NameUnparseable, report);
    };
    if indexed.contains(&(place.camera.to_owned(), seq)) {
        return Ok(());
    }
    let meta = std::fs::symlink_metadata(place.path)
        .map_err(|source| io_error("inspect", place.path.to_path_buf(), source))?;
    if !meta.is_file() || meta.len() > MAX_PIECE_BYTES {
        return quarantine(&place, Quarantined::Seq(seq), report);
    }
    let Ok(bytes) = std::fs::read(place.path) else {
        return quarantine(&place, Quarantined::Seq(seq), report);
    };
    let Ok(facts) = piece_walk::walk(&bytes) else {
        return quarantine(&place, Quarantined::Seq(seq), report);
    };
    let Some(end_ms) = start_ms.checked_add(facts.duration_ms) else {
        return quarantine(&place, Quarantined::Seq(seq), report);
    };
    let duration_s = std::time::Duration::from_millis(
        u64::try_from(facts.duration_ms).expect("a non-negative millisecond count fits u64"),
    )
    .as_secs_f64();
    let row = PieceRow {
        camera: place.camera.to_owned(),
        seq,
        start_ms,
        end_ms,
        duration_s,
        path: format!(
            "recordings/{}/{}/{}/{}",
            place.camera, place.date, place.hour, place.name
        ),
        size_bytes: u64::try_from(bytes.len())
            .expect("an allocated read buffer always fits in u64"),
        codec: facts.codec.to_owned(),
    };
    index.insert(&row).map_err(index_error)?;
    report.inserted += 1;
    eprintln!(
        "corvette-media-bridge: recovery inserted camera={} seq={seq}",
        place.camera
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum Quarantined {
    NameUnparseable,
    Seq(u64),
}

fn quarantine(
    place: &PiecePlace<'_>,
    why: Quarantined,
    report: &mut ScanReport,
) -> Result<(), RecoveryError> {
    let lost_dir = place.root.join("lost+found");
    std::fs::create_dir_all(&lost_dir)
        .map_err(|error| io_error("create lost+found", lost_dir.clone(), error))?;
    let mut dest = lost_dir.join(format!("{}-{}", place.camera, place.name));
    let mut dup = 1u32;
    while dest.exists() {
        dest = lost_dir.join(format!("{}-{}.dup{dup}", place.camera, place.name));
        dup += 1;
    }
    std::fs::rename(place.path, &dest)
        .map_err(|error| io_error("quarantine move", place.path.to_path_buf(), error))?;
    match why {
        Quarantined::NameUnparseable => {
            eprintln!("corvette-media-bridge: recovery quarantined name-unparseable");
        }
        Quarantined::Seq(seq) => {
            eprintln!(
                "corvette-media-bridge: recovery quarantined camera={} seq={seq}",
                place.camera
            );
        }
    }
    report.quarantined += 1;
    Ok(())
}

fn drop_dangling_rows(
    root: &Path,
    index: &mut Index,
    report: &mut ScanReport,
) -> Result<(), RecoveryError> {
    let rows = index.all_pieces().map_err(index_error)?;
    for row in rows {
        if !stored_path_is_safe(&row.path) {
            report.rows_refused += 1;
            eprintln!(
                "corvette-media-bridge: recovery row-path-refused camera={} seq={}",
                row.camera, row.seq
            );
            continue;
        }
        let path = root.join(&row.path);
        if path
            .try_exists()
            .map_err(|source| io_error("stat", path.clone(), source))?
        {
            continue;
        }
        index.remove(&row.camera, row.seq).map_err(index_error)?;
        report.rows_dropped += 1;
        eprintln!(
            "corvette-media-bridge: recovery row-dropped camera={} seq={}",
            row.camera, row.seq
        );
    }
    Ok(())
}
