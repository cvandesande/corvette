use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Free bytes this process may still write on the filesystem holding `root`: `f_bavail` times `f_frsize`.
///
/// # Errors
/// [`std::io::ErrorKind::InvalidInput`] when `root` holds an interior NUL byte, otherwise the operating-system error.
pub fn free_bytes(root: &Path) -> std::io::Result<u64> {
    let path = CString::new(root.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: `path` is a valid NUL-terminated C string and `stat` is writable, aligned storage
    // of the exact size `statvfs` fills; the value is read only after the call returned zero.
    let stat = unsafe {
        if libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        stat.assume_init()
    };
    Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
}
