use std::cell::RefCell;

use corvette_detect::{StreamAddressError, StreamKind, StreamSource, resolve_stream_url};

fn source(username: &str, secret_env: Option<&str>) -> StreamSource {
    StreamSource {
        host: "cam.example".to_owned(),
        port: 554,
        path: "/live".to_owned(),
        username: username.to_owned(),
        secret_env: secret_env.map(str::to_owned),
    }
}

const CANARY: &str = "p@ss:wor/d 1";
const CANARY_ENCODED: &str = "p%40ss%3Awor%2Fd%201";
const VARIABLE: &str = "CAM_FRONT_SECRET";

enum Outcome {
    Address(&'static str),
    Refusal(&'static str),
}

struct Row {
    camera: &'static str,
    kind: StreamKind,
    host: &'static str,
    port: u32,
    path: &'static str,
    username: &'static str,
    secret_env: Option<&'static str>,
    secret_value: Option<&'static str>,
    outcome: Outcome,
}

fn stream(row: &Row) -> StreamSource {
    StreamSource {
        host: row.host.to_owned(),
        port: row.port,
        path: row.path.to_owned(),
        username: row.username.to_owned(),
        secret_env: row.secret_env.map(str::to_owned),
    }
}

fn lookup(row: &Row) -> impl Fn(&str) -> Option<String> + '_ {
    move |name: &str| {
        if row.secret_env == Some(name) {
            row.secret_value.map(str::to_owned)
        } else {
            Some(CANARY.to_owned())
        }
    }
}

fn rows() -> Vec<Row> {
    vec![
        Row {
            camera: "front",
            kind: StreamKind::Main,
            host: "cam.example",
            port: 554,
            path: "/live",
            username: "admin",
            secret_env: Some(VARIABLE),
            secret_value: Some(CANARY),
            outcome: Outcome::Address("rtsp://admin:p%40ss%3Awor%2Fd%201@cam.example:554/live"),
        },
        Row {
            camera: "front",
            kind: StreamKind::Sub,
            host: "cam.example",
            port: 8554,
            path: "/sub",
            username: "front.cam-1_x2",
            secret_env: None,
            secret_value: None,
            outcome: Outcome::Address("rtsp://front.cam-1_x2@cam.example:8554/sub"),
        },
        Row {
            camera: "back",
            kind: StreamKind::Main,
            host: "cam.example",
            port: 554,
            path: "/",
            username: "café",
            secret_env: None,
            secret_value: None,
            outcome: Outcome::Address("rtsp://caf%C3%A9@cam.example:554/"),
        },
        Row {
            camera: "back",
            kind: StreamKind::Sub,
            host: "cam.example",
            port: 554,
            path: "/media",
            username: "user",
            secret_env: Some(VARIABLE),
            secret_value: Some("pw1"),
            outcome: Outcome::Address("rtsp://user:pw1@cam.example:554/media"),
        },
        Row {
            camera: "back",
            kind: StreamKind::Main,
            host: "cam.example",
            port: 554,
            path: "/live",
            username: "",
            secret_env: None,
            secret_value: None,
            outcome: Outcome::Address("rtsp://cam.example:554/live"),
        },
        Row {
            camera: "front",
            kind: StreamKind::Main,
            host: "cam.example",
            port: 554,
            path: "/live",
            username: "admin",
            secret_env: Some(VARIABLE),
            secret_value: None,
            outcome: Outcome::Refusal("camera front: source main: secret_env: variable is not set"),
        },
        Row {
            camera: "front",
            kind: StreamKind::Sub,
            host: "cam.example",
            port: 554,
            path: "/sub",
            username: "admin",
            secret_env: Some(VARIABLE),
            secret_value: Some(""),
            outcome: Outcome::Refusal("camera front: source sub: secret_env: variable is empty"),
        },
    ]
}

#[test]
fn assembled_addresses_and_refusals_match_the_table() {
    for row in rows() {
        let source = stream(&row);
        let result = resolve_stream_url(row.camera, row.kind, &source, lookup(&row));
        match &row.outcome {
            Outcome::Address(want) => {
                let address = result.expect("configured address");
                assert_eq!(*want, address, "camera {}", row.camera);
                assert!(address.starts_with("rtsp://"));
                assert!(address.ends_with(row.path));
            }
            Outcome::Refusal(want) => {
                let error = result.expect_err("refusal");
                assert_eq!(*want, error.to_string());
                assert_no_secret(format!("{error}").as_str());
                assert_no_secret(format!("{error:?}").as_str());
            }
        }
    }
}

fn assert_no_secret(text: &str) {
    assert!(!text.contains(CANARY), "canary in {text}");
    assert!(!text.contains(CANARY_ENCODED), "encoded canary in {text}");
}

#[test]
fn the_password_is_percent_encoded_and_appears_once() {
    let stream = source("admin", Some(VARIABLE));
    let address = resolve_stream_url("front", StreamKind::Main, &stream, |_| {
        Some(CANARY.to_owned())
    })
    .expect("configured address");
    for token in ["%40", "%3A", "%2F", "%20"] {
        assert!(address.contains(token), "missing {token}");
    }
    assert!(!address.contains('+'));
    assert_eq!(1, address.matches(CANARY_ENCODED).count());
    let (userinfo, _host) = address
        .strip_prefix("rtsp://")
        .and_then(|rest| rest.split_once('@'))
        .expect("userinfo");
    let (_user, password) = userinfo.split_once(':').expect("separator");
    for byte in ['@', ':', '/', ' '] {
        assert!(!password.contains(byte), "raw {byte:?} in the userinfo");
    }
    assert!(!userinfo.contains(CANARY));
}

#[test]
fn the_secret_variable_is_looked_up_exactly_once() {
    let names = RefCell::new(Vec::new());
    let stream = source("admin", Some(VARIABLE));
    let address = resolve_stream_url("front", StreamKind::Main, &stream, |name| {
        names.borrow_mut().push(name.to_owned());
        Some(CANARY.to_owned())
    })
    .expect("configured address");
    assert!(address.contains(CANARY_ENCODED));
    assert_eq!(vec![VARIABLE.to_owned()], *names.borrow());
}

#[test]
fn the_refusal_names_camera_kind_and_key() {
    let stream = source("admin", Some("CAM_ABSENT_SECRET"));
    let error = resolve_stream_url("front", StreamKind::Main, &stream, |name| {
        (name != "CAM_ABSENT_SECRET").then(|| CANARY.to_owned())
    })
    .expect_err("refusal");
    assert_eq!(
        StreamAddressError {
            camera: "front".to_owned(),
            kind: StreamKind::Main,
            key: "secret_env",
            rule: "variable is not set",
        },
        error
    );
    assert_no_secret(&error.to_string());
    assert_no_secret(&format!("{error:?}"));
}
