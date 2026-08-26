//! INV-1's enforcement for this crate: no AGPL-licensed dependency, and no
//! literal reference to the two AGPL restream servers D-7 read and rejected
//! (`bairelay-rtsp`, `lvqr-rtsp`) or to Corvette-specific crates this
//! reusable crate must never depend on (D-6: usable outside Corvette).

use std::path::{Path, PathBuf};

const FORBIDDEN_DEPENDENCY_NAMES: &[&str] = &["bairelay", "lvqr"];

/// Literals a grep-equivalent check denies anywhere in this crate's own
/// source: the two rejected AGPL crates' names, and the Corvette-specific
/// crates this reusable crate must never know exist (issue #12 X1's own
/// scope guard).
const FORBIDDEN_SOURCE_LITERALS: &[&str] = &[
    "bairelay",
    "lvqr",
    "corvette-rtsp-client",
    "corvette_rtsp_client",
    "moq_mux",
    "moq_net",
    "frigate",
];

#[test]
fn this_crates_own_declared_license_is_not_agpl() {
    let manifest = std::fs::read_to_string(manifest_dir().join("Cargo.toml"))
        .expect("reads this crate's own Cargo.toml");
    let license_line = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("license = "))
        .expect("Cargo.toml declares a license");

    assert!(
        !license_line.to_ascii_uppercase().contains("AGPL"),
        "crates/rtsp-restream must never declare an AGPL license (D-7); found: {license_line}"
    );
}

#[test]
fn no_agpl_restream_server_appears_as_a_dependency_in_the_workspace_lockfile() {
    let lockfile_path = workspace_root().join("Cargo.lock");
    let lockfile = std::fs::read_to_string(&lockfile_path)
        .unwrap_or_else(|err| panic!("reads {}: {err}", lockfile_path.display()));

    let mut offenders = Vec::new();
    for forbidden in FORBIDDEN_DEPENDENCY_NAMES {
        for line in lockfile.lines() {
            if line.trim_start().starts_with("name = ")
                && line.to_ascii_lowercase().contains(forbidden)
            {
                offenders.push(format!("{forbidden}: {}", line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "Cargo.lock names a dependency D-7 rejected as AGPL-incompatible: {offenders:?}"
    );
}

#[test]
fn source_never_references_a_forbidden_literal() {
    let src_dir = manifest_dir().join("src");
    let mut offenders = Vec::new();
    for path in rust_source_files(&src_dir) {
        let contents = std::fs::read_to_string(&path).expect("reads a source file");
        let lowercase = contents.to_ascii_lowercase();
        for literal in FORBIDDEN_SOURCE_LITERALS {
            if lowercase.contains(&literal.to_ascii_lowercase()) {
                offenders.push(format!("{}: {literal}", path.display()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "found a literal issue #12 X1's scope guard forbids in this crate's source: {offenders:?}"
    );
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_root() -> PathBuf {
    manifest_dir()
        .parent()
        .and_then(Path::parent)
        .expect("crates/rtsp-restream is two directories below the workspace root")
        .to_path_buf()
}

fn rust_source_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let entries = std::fs::read_dir(dir).expect("reads the src directory");
    for entry in entries {
        let path = entry.expect("reads a directory entry").path();
        if path.is_dir() {
            files.extend(rust_source_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    files
}
