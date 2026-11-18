use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use corvette_detect::{EngineError, LabelError, Labels, WorkerClient};
use corvette_detect_proto::{
    ErrorCode, HEADER_LEN, RAW_VALUES, Reply, Request, Row, TENSOR_BYTES, parse_header,
};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "engine-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn read_request_frame(stream: &mut UnixStream) -> Vec<u8> {
    let mut header_bytes = [0u8; HEADER_LEN];
    stream
        .read_exact(&mut header_bytes)
        .expect("request header");
    let header = parse_header(&header_bytes).expect("header decodes");
    let mut frame = vec![0u8; HEADER_LEN + header.body_len];
    frame[..HEADER_LEN].copy_from_slice(&header_bytes);
    stream
        .read_exact(&mut frame[HEADER_LEN..])
        .expect("request body");
    frame
}

fn write_reply(stream: &mut UnixStream, reply: &Reply) {
    let bytes = reply.encode().expect("reply encodes");
    stream.write_all(&bytes).expect("reply written");
}

fn serve_replies(listener: UnixListener, replies: Vec<Reply>) {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        for reply in replies {
            read_request_frame(&mut stream);
            let bytes = reply.encode().expect("reply encodes");
            stream.write_all(&bytes).expect("reply written");
        }
    });
}

const fn row(class_id: u32, score: f32) -> Row {
    Row {
        class_id,
        score,
        y_min: 0.25,
        x_min: 0.25,
        y_max: 0.75,
        x_max: 0.75,
    }
}

fn sample_tensor() -> Box<[u8; TENSOR_BYTES]> {
    let mut bytes = vec![0x5A; TENSOR_BYTES];
    bytes[7] = 0xF0;
    <Box<[u8; TENSOR_BYTES]>>::try_from(bytes.into_boxed_slice()).expect("tensor length")
}

#[test]
fn parses_the_label_map_shape_and_ignores_blank_lines() {
    let text = "0 person\n1 dhl\n2 deer\n\n3 dog\n4 horse\n";
    let labels = Labels::parse(text).expect("valid label map");
    assert_eq!(labels.len(), 5);
    assert!(!labels.is_empty());
    assert_eq!(labels.name(0), Some("person"));
    assert_eq!(labels.name(2), Some("deer"));
    assert_eq!(labels.name(4), Some("horse"));
    assert_eq!(labels.name(5), None);
    assert_eq!(labels.name(u32::MAX), None);
}

#[test]
fn keeps_spaces_inside_a_label_name() {
    let labels = Labels::parse("0 license plate\n").expect("names may hold spaces");
    assert_eq!(labels.name(0), Some("license plate"));
}

#[test]
fn parses_an_empty_label_map_as_no_labels() {
    let labels = Labels::parse("").expect("empty is valid");
    assert!(labels.is_empty());
    assert_eq!(labels.name(0), None);
}

#[test]
fn rejects_index_gaps_repeats_non_numeric_indices_and_missing_names_by_line() {
    let cases: [(&str, LabelError); 5] = [
        (
            "0 person\n1 dhl\n3 deer\n",
            LabelError::IndexMismatch {
                line: 3,
                expected: 2,
                found: 3,
            },
        ),
        (
            "0 person\n1 dhl\n1 deer\n",
            LabelError::IndexMismatch {
                line: 3,
                expected: 2,
                found: 1,
            },
        ),
        (
            "0 person\nx dhl\n",
            LabelError::NotANumber {
                line: 2,
                text: "x".to_owned(),
            },
        ),
        ("0 person\n1\n", LabelError::MissingName { line: 2 }),
        ("0 \n", LabelError::MissingName { line: 1 }),
    ];
    for (text, expected) in cases {
        assert_eq!(Labels::parse(text), Err(expected), "text was {text:?}");
    }
}

#[test]
fn infer_returns_the_rows_a_worker_replies() {
    let dir = temp_dir("rows");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let rows = vec![row(3, 0.75), row(17, 0.5)];
    let reply = Reply::Rows {
        id: 0,
        rows: rows.clone(),
    };
    serve_replies(listener, vec![reply]);
    let mut client = WorkerClient::connect(&socket, Duration::from_secs(5)).expect("connect");
    let outcome = client.infer(&sample_tensor()).expect("rows reply");
    assert_eq!(outcome, rows);
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn an_error_reply_becomes_worker_with_the_code_message() {
    let dir = temp_dir("error");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let error = Reply::Error {
        id: 0,
        code: ErrorCode::DeadlineOverrun,
    };
    serve_replies(listener, vec![error]);
    let mut client = WorkerClient::connect(&socket, Duration::from_secs(5)).expect("connect");
    let outcome = client.infer(&sample_tensor());
    assert!(
        matches!(&outcome, Err(EngineError::Worker(text)) if text.contains("DeadlineOverrun")),
        "{outcome:?}"
    );
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn health_and_raw_replies_to_an_infer_are_unexpected() {
    let dir = temp_dir("kinds");
    let health_socket = dir.join("health.sock");
    let raw_socket = dir.join("raw.sock");
    let health_listener = UnixListener::bind(&health_socket).expect("bind");
    let raw_listener = UnixListener::bind(&raw_socket).expect("bind");
    let raw_values = vec![0.5; RAW_VALUES];
    serve_replies(health_listener, vec![Reply::Health { id: 0 }]);
    serve_replies(
        raw_listener,
        vec![Reply::Raw {
            id: 0,
            values: raw_values,
        }],
    );
    for socket in [&health_socket, &raw_socket] {
        let mut client = WorkerClient::connect(socket, Duration::from_secs(5)).expect("connect");
        let outcome = client.infer(&sample_tensor());
        assert!(
            matches!(outcome, Err(EngineError::UnexpectedReply)),
            "{outcome:?}"
        );
    }
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_socket_closed_before_the_reply_is_an_io_error() {
    let dir = temp_dir("closed");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        read_request_frame(&mut stream);
        drop(stream);
    });
    let mut client = WorkerClient::connect(&socket, Duration::from_secs(5)).expect("connect");
    let outcome = client.infer(&sample_tensor());
    assert!(matches!(outcome, Err(EngineError::Io(_))), "{outcome:?}");
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn infer_gives_up_at_the_deadline_when_the_worker_never_replies() {
    let dir = temp_dir("deadline");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        thread::sleep(Duration::from_secs(5));
        drop(stream);
    });
    let (tx, rx) = mpsc::channel();
    let tensor = sample_tensor();
    thread::spawn(move || {
        let client_deadline = Duration::from_millis(100);
        let mut client = WorkerClient::connect(&socket, client_deadline).expect("connect");
        tx.send(client.infer(&tensor)).expect("outcome sent");
    });
    let outcome = rx.recv_timeout(Duration::from_millis(1500));
    assert!(
        matches!(outcome, Ok(Err(EngineError::Io(_)))),
        "the deadline must surface within 1.5 s: {outcome:?}"
    );
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn connect_reports_io_for_a_socket_path_that_is_not_listening() {
    let dir = temp_dir("missing");
    let socket = dir.join("absent.sock");
    let outcome = WorkerClient::connect(&socket, Duration::from_secs(5));
    assert!(matches!(outcome, Err(EngineError::Io(_))), "{outcome:?}");
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn connect_rejects_a_zero_deadline() {
    let dir = temp_dir("zero");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let outcome = WorkerClient::connect(&socket, Duration::ZERO);
    assert!(
        matches!(outcome, Err(EngineError::ZeroDeadline)),
        "{outcome:?}"
    );
    drop(listener);
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn two_consecutive_infers_each_get_their_own_reply_on_one_connection() {
    let dir = temp_dir("twice");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let first = vec![row(1, 0.9)];
    let second = vec![row(2, 0.8)];
    let first_reply = Reply::Rows {
        id: 0,
        rows: first.clone(),
    };
    let second_reply = Reply::Rows {
        id: 1,
        rows: second.clone(),
    };
    serve_replies(listener, vec![first_reply, second_reply]);
    let mut client = WorkerClient::connect(&socket, Duration::from_secs(5)).expect("connect");
    let first_outcome = client.infer(&sample_tensor()).expect("first rows");
    let second_outcome = client.infer(&sample_tensor()).expect("second rows");
    assert_eq!(first_outcome, first);
    assert_eq!(second_outcome, second);
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_reply_with_the_wrong_id_is_an_id_mismatch() {
    let dir = temp_dir("idmismatch");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    serve_replies(listener, vec![Reply::Health { id: 99 }]);
    let mut client = WorkerClient::connect(&socket, Duration::from_secs(5)).expect("connect");
    let Err(EngineError::IdMismatch { sent, received }) = client.infer(&sample_tensor()) else {
        panic!("expected an id mismatch");
    };
    assert_eq!((sent, received), (0, 99));
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_stale_second_reply_is_an_id_mismatch_and_not_fresh_rows() {
    let dir = temp_dir("stale");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let fresh_reply = Reply::Rows {
        id: 0,
        rows: vec![row(1, 0.9)],
    };
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        read_request_frame(&mut stream);
        write_reply(&mut stream, &fresh_reply);
        write_reply(&mut stream, &Reply::Health { id: 0 });
        read_request_frame(&mut stream);
    });
    let mut client = WorkerClient::connect(&socket, Duration::from_secs(5)).expect("connect");
    let first_outcome = client.infer(&sample_tensor()).expect("first rows");
    assert_eq!(first_outcome, vec![row(1, 0.9)]);
    let Err(EngineError::IdMismatch { sent, received }) = client.infer(&sample_tensor()) else {
        panic!("expected the stale reply rejected");
    };
    assert_eq!((sent, received), (1, 0));
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn consecutive_infer_requests_advance_the_request_id_by_one() {
    let dir = temp_dir("ids");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        for _ in 0..2 {
            let frame = read_request_frame(&mut stream);
            let decoded = Request::decode(&frame).expect("request decodes");
            let Request::InferRequest { id, .. } = decoded else {
                panic!("the client must send an infer request");
            };
            tx.send(id).expect("id sent");
            write_reply(&mut stream, &Reply::Health { id });
        }
    });
    let mut client = WorkerClient::connect(&socket, Duration::from_secs(5)).expect("connect");
    client.infer(&sample_tensor()).ok();
    client.infer(&sample_tensor()).ok();
    let first_id = rx.recv_timeout(Duration::from_secs(5)).expect("first id");
    let second_id = rx.recv_timeout(Duration::from_secs(5)).expect("second id");
    assert_eq!(second_id, first_id + 1);
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn a_half_reply_then_silence_is_an_io_error_within_the_deadline() {
    let dir = temp_dir("halfreply");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        read_request_frame(&mut stream);
        stream.write_all(&[0u8; HEADER_LEN / 2]).expect("half");
        thread::sleep(Duration::from_secs(5));
    });
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let client_deadline = Duration::from_millis(300);
        let mut client = WorkerClient::connect(&socket, client_deadline).expect("connect");
        tx.send(client.infer(&sample_tensor())).expect("sent");
    });
    let outcome = rx.recv_timeout(Duration::from_secs(2));
    assert!(
        matches!(outcome, Ok(Err(EngineError::Io(_)))),
        "io: {outcome:?}"
    );
    fs::remove_dir_all(&dir).expect("remove temp dir");
}

#[test]
fn the_request_the_worker_reads_carries_exactly_the_tensor_sent() {
    let dir = temp_dir("tensor");
    let socket = dir.join("worker.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let frame = read_request_frame(&mut stream);
        tx.send(frame).expect("frame sent");
        let bytes = Reply::Health { id: 0 }.encode().expect("reply encodes");
        stream.write_all(&bytes).expect("reply written");
    });
    let tensor = sample_tensor();
    let mut client = WorkerClient::connect(&socket, Duration::from_secs(5)).expect("connect");
    let outcome = client.infer(&tensor);
    assert!(
        matches!(outcome, Err(EngineError::UnexpectedReply)),
        "{outcome:?}"
    );
    let frame = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("request arrived");
    match Request::decode(&frame).expect("request decodes") {
        Request::InferRequest {
            id, tensor: seen, ..
        } => {
            assert_eq!(id, 0);
            assert_eq!(seen.as_slice(), tensor.as_slice());
        }
        Request::InferRawRequest { .. } | Request::HealthRequest { .. } => {
            panic!("the client must send an infer request");
        }
    }
    fs::remove_dir_all(&dir).expect("remove temp dir");
}
