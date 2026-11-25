use corvette_media_bridge::disk_free::free_bytes;
use std::path::Path;

#[test]
fn a_temporary_root_reports_a_nonzero_free_figure() {
    let dir = std::env::temp_dir().join(format!("corvette-disk-free-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("test root created");
    let free = free_bytes(&dir).expect("the temp dir is readable");
    assert!(free > 0, "a real filesystem reports free space, got {free}");
    std::fs::remove_dir_all(&dir).expect("temp root removed");
}

#[test]
fn a_missing_path_reports_an_error() {
    let missing = Path::new("/corvette-disk-free-does-not-exist");
    let error = free_bytes(missing).expect_err("a missing path cannot be read");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn an_interior_nul_byte_is_invalid_input() {
    let inside = Path::new("front\0door");
    let error = free_bytes(inside).expect_err("a NUL byte cannot reach the syscall");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}
