use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use corvette_detect_proto::{
    DEADLINE_LEN, ErrorCode, HEADER_LEN, ID_LEN, Kind, Reply, Request, parse_header,
};

const ACCEPT_POLL: Duration = Duration::from_millis(20);
const HEALTH_BODY_LEN: usize = ID_LEN + DEADLINE_LEN;

/// Answers health requests on its own socket, one connection at a time, until the stop flag is set.
/// # Errors
/// Returns the I/O error of setting the listener non-blocking or of a failed accept.
pub fn serve_health(
    listener: &UnixListener,
    stop: &AtomicBool,
    io_timeout: Duration,
) -> Result<(), io::Error> {
    listener.set_nonblocking(true)?;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        match listener.accept() {
            Ok((stream, _addr)) => serve_health_connection(stream, io_timeout),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(ACCEPT_POLL),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

fn serve_health_connection(mut stream: UnixStream, io_timeout: Duration) {
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(io_timeout)).is_err()
        || stream.set_write_timeout(Some(io_timeout)).is_err()
    {
        return;
    }
    while let Some(id) = next_health_id(&mut stream) {
        if !write_health(&stream, id) {
            return;
        }
    }
}

fn next_health_id(stream: &mut UnixStream) -> Option<u64> {
    let mut head = [0u8; HEADER_LEN];
    if stream.read_exact(&mut head).is_err() {
        return None;
    }
    let Ok(header) = parse_header(&head) else {
        refuse(stream);
        return None;
    };
    if header.kind != Kind::HealthRequest || header.body_len != HEALTH_BODY_LEN {
        refuse(stream);
        return None;
    }
    let mut body = [0u8; HEALTH_BODY_LEN];
    if stream.read_exact(&mut body).is_err() {
        return None;
    }
    let frame = [head.as_slice(), body.as_slice()].concat();
    let Ok(Request::HealthRequest { id, .. }) = Request::decode(&frame) else {
        refuse(stream);
        return None;
    };
    Some(id)
}

fn refuse(stream: &mut UnixStream) {
    let _ = Reply::Error {
        id: 0,
        code: ErrorCode::BadRequest,
    }
    .encode()
    .is_ok_and(|bytes| stream.write_all(&bytes).is_ok());
}

fn write_health(mut stream: &UnixStream, id: u64) -> bool {
    Reply::Health { id }
        .encode()
        .is_ok_and(|bytes| stream.write_all(&bytes).is_ok())
}
