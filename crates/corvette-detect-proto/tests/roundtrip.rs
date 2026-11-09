use corvette_detect_proto::{ErrorCode, Kind, Reply, Request, Row};

fn row_bits(rows: &[Row]) -> Vec<(u32, u32, u32, u32, u32, u32)> {
    rows.iter()
        .map(|row| {
            (
                row.class_id,
                row.score.to_bits(),
                row.y_min.to_bits(),
                row.x_min.to_bits(),
                row.y_max.to_bits(),
                row.x_max.to_bits(),
            )
        })
        .collect()
}

fn tensor_pattern() -> Vec<u8> {
    (0..1_228_800usize)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect()
}

#[test]
fn an_infer_request_encodes_to_the_documented_bytes_and_decodes_back() {
    let tensor_bytes = tensor_pattern();
    let tensor = <&[u8; 1_228_800]>::try_from(tensor_bytes.as_slice()).unwrap();
    let request = Request::InferRequest {
        id: 0x0102_0304_0506_0708,
        deadline_ms: 1_500,
        tensor,
    };
    let frame = request.encode();
    assert_eq!(frame.len(), 1_228_818);
    assert_eq!(&frame[..6], [1, 0x01, 0x0C, 0xC0, 0x12, 0x00]);
    assert_eq!(&frame[6..14], 0x0102_0304_0506_0708_u64.to_le_bytes());
    assert_eq!(&frame[14..18], 1_500_u32.to_le_bytes());
    assert_eq!(&frame[18..], tensor_bytes);
    assert_eq!(Request::decode(&frame).unwrap(), request);
}

#[test]
fn an_infer_raw_request_differs_from_an_infer_request_only_in_the_kind_byte() {
    let tensor_vec = vec![7u8; 1_228_800];
    let tensor = <&[u8; 1_228_800]>::try_from(tensor_vec.as_slice()).unwrap();
    let plain = Request::InferRequest {
        id: 41,
        deadline_ms: 60,
        tensor,
    };
    let raw = Request::InferRawRequest {
        id: 41,
        deadline_ms: 60,
        tensor,
    };
    let plain_frame = plain.encode();
    let mut raw_frame = raw.encode();
    assert_eq!(plain_frame.len(), raw_frame.len());
    assert_eq!(plain_frame[1], 0x01);
    assert_eq!(raw_frame[1], 0x02);
    raw_frame[1] = plain_frame[1];
    assert_eq!(raw_frame, plain_frame);
    assert_eq!(Request::decode(&raw.encode()).unwrap(), raw);
}

#[test]
fn a_health_request_is_eighteen_bytes() {
    let request = Request::HealthRequest {
        id: 258,
        deadline_ms: 1_500,
    };
    let frame = request.encode();
    assert_eq!(frame.len(), 18);
    assert_eq!(
        frame,
        [1, 3, 12, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 0xDC, 5, 0, 0]
    );
    assert_eq!(Request::decode(&frame).unwrap(), request);
}

#[test]
fn rows_replies_round_trip_with_zero_one_and_max_rows() {
    let zero = Reply::Rows {
        id: 5,
        rows: vec![],
    };
    let zero_frame = zero.encode().unwrap();
    assert_eq!(zero_frame, [1, 0x81, 9, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(Reply::decode(&zero_frame).unwrap(), zero);

    let row = Row {
        class_id: 2,
        score: 0.75,
        y_min: 0.25,
        x_min: 0.5,
        y_max: 0.875,
        x_max: 0.125,
    };
    let one = Reply::Rows {
        id: 5,
        rows: vec![row.clone()],
    };
    let one_frame = one.encode().unwrap();
    let mut expected: Vec<u8> = vec![1, 0x81, 33, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 1];
    expected.extend_from_slice(&2_u32.to_le_bytes());
    for value in [0.75_f32, 0.25, 0.5, 0.875, 0.125] {
        expected.extend_from_slice(&value.to_le_bytes());
    }
    assert_eq!(one_frame, expected);
    let Reply::Rows {
        id: one_id,
        rows: one_rows,
    } = Reply::decode(&one_frame).unwrap()
    else {
        panic!("a rows reply decodes as a rows reply");
    };
    assert_eq!(one_id, 5);
    assert_eq!(row_bits(&one_rows), row_bits(&[row]));

    let rows: Vec<Row> = (0..20usize)
        .map(|index| Row {
            class_id: u32::try_from(index).unwrap(),
            score: f32::from(u8::try_from(index % 251).unwrap()),
            y_min: f32::from(u8::try_from(index * 3 % 251).unwrap()),
            x_min: f32::from(u8::try_from(index * 5 % 251).unwrap()),
            y_max: f32::from(u8::try_from(index * 7 % 251).unwrap()),
            x_max: f32::from(u8::try_from(index * 11 % 251).unwrap()),
        })
        .collect();
    let many = Reply::Rows {
        id: 9,
        rows: rows.clone(),
    };
    let many_frame = many.encode().unwrap();
    assert_eq!(many_frame.len(), 495);
    let Reply::Rows {
        id: many_id,
        rows: many_rows,
    } = Reply::decode(&many_frame).unwrap()
    else {
        panic!("a rows reply decodes as a rows reply");
    };
    assert_eq!(many_id, 9);
    assert_eq!(row_bits(&many_rows), row_bits(&rows));
}

#[test]
fn a_raw_reply_keeps_every_bit_pattern() {
    let specials = [
        f32::from_bits(0x7fc0_1234),
        -0.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::from_bits(1),
    ];
    let values: Vec<f32> = (0..420_000usize)
        .map(|index| {
            specials
                .get(index)
                .copied()
                .unwrap_or_else(|| f32::from_bits(u32::try_from(index).unwrap()))
        })
        .collect();
    let reply = Reply::Raw {
        id: 4_096,
        values: values.clone(),
    };
    let frame = reply.encode().unwrap();
    assert_eq!(frame.len(), 6 + 8 + 1_680_000);
    assert_eq!(&frame[..6], [1, 0x82, 0x88, 0xA2, 0x19, 0x00]);
    let decoded = Reply::decode(&frame).unwrap();
    let Reply::Raw {
        id,
        values: decoded_values,
    } = decoded
    else {
        panic!("a raw reply decodes as a raw reply");
    };
    assert_eq!(id, 4_096);
    assert_eq!(decoded_values.len(), 420_000);
    for (decoded_value, value) in decoded_values.iter().zip(&values) {
        assert_eq!(decoded_value.to_bits(), value.to_bits());
    }
}

#[test]
fn health_and_error_replies_round_trip() {
    let health = Reply::Health { id: 300 };
    let health_frame = health.encode().unwrap();
    assert_eq!(
        health_frame,
        [1, 0x83, 8, 0, 0, 0, 0x2C, 1, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(Reply::decode(&health_frame).unwrap(), health);

    let cases = [
        (ErrorCode::DeviceLost, 1_u8),
        (ErrorCode::DeadlineOverrun, 2),
        (ErrorCode::BadRequest, 3),
    ];
    for (code, byte) in cases {
        let error = Reply::Error { id: 9, code };
        let frame = error.encode().unwrap();
        assert_eq!(frame.len(), 15);
        assert_eq!(*frame.last().unwrap(), byte);
        assert_eq!(frame, [1, 0x8F, 9, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, byte]);
        assert_eq!(Reply::decode(&frame).unwrap(), error);
        assert_eq!(code.byte(), byte);
        assert_eq!(ErrorCode::from_byte(byte), Some(code));
    }
}

#[test]
fn every_kind_byte_parses_to_its_kind() {
    let cases = [
        (0x01_u8, Kind::InferRequest),
        (0x02, Kind::InferRawRequest),
        (0x03, Kind::HealthRequest),
        (0x81, Kind::RowsReply),
        (0x82, Kind::RawReply),
        (0x83, Kind::HealthReply),
        (0x8F, Kind::ErrorReply),
    ];
    for (byte, kind) in cases {
        assert_eq!(Kind::from_byte(byte), Some(kind));
        assert_eq!(kind.byte(), byte);
    }
}
