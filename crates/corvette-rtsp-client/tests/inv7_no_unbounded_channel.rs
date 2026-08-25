//! INV-7's grep-equivalent enforcement: no unbounded channel construction
//! appears anywhere in this crate's own source. Complements
//! `tests/client.rs`'s `slow_subscriber_sees_lagged_fast_subscriber_does_not`,
//! which asserts the *behavior* a bounded channel guarantees.

const FORBIDDEN_LITERALS: &[&str] = &["unbounded_channel", "unboundedsender", "unboundedreceiver"];

#[test]
fn source_never_constructs_an_unbounded_channel() {
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_source_files(&src_dir) {
        let contents = std::fs::read_to_string(&path).expect("reads a source file");
        let lowercase = contents.to_ascii_lowercase();
        for literal in FORBIDDEN_LITERALS {
            if lowercase.contains(literal) {
                offenders.push(format!("{}: {literal}", path.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "found unbounded-channel constructs INV-7 forbids in this crate's source: {offenders:?}"
    );
}

fn rust_source_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
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
