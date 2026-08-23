//! INV-1's grep-equivalent enforcement: no container-format literal appears
//! anywhere in this crate's own source. Complements the unit tests in
//! `src/depacketize` that assert emitted H.264/H.265 bytes are actually
//! Annex-B start-code framed.

const FORBIDDEN_LITERALS: &[&str] = &["mpeg2ts", "mp4box", "moq_mux::container"];

#[test]
fn source_never_references_a_container_format_literal() {
    let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_source_files(&src_dir) {
        let contents = std::fs::read_to_string(&path).expect("reads a source file");
        let lowercase = contents.to_ascii_lowercase();
        for literal in FORBIDDEN_LITERALS {
            if lowercase.contains(&literal.to_ascii_lowercase()) {
                offenders.push(format!("{}: {literal}", path.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "found container-format literals INV-1 forbids in this crate's source: {offenders:?}"
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
