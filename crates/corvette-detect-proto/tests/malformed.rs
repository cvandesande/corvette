use corvette_detect_proto::{
    DecodeError, EncodeError, Header, HeaderError, Kind, Reply, Request, Row, parse_header,
};

fn frame(kind_byte: u8, declared: u32, body: &[u8]) -> Vec<u8> {
    let mut out = vec![1, kind_byte];
    out.extend_from_slice(&declared.to_le_bytes());
    out.extend_from_slice(body);
    out
}

#[test]
fn short_input_is_truncated() {
    let full = [1u8, 3, 12, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 0xDC, 5, 0, 0];
    for len in 0..6 {
        assert_eq!(Request::decode(&full[..len]), Err(DecodeError::Truncated));
        assert_eq!(Reply::decode(&full[..len]), Err(DecodeError::Truncated));
    }
}

#[test]
fn an_unknown_version_is_refused() {
    for version in [0_u8, 2] {
        let head = [version, 3, 12, 0, 0, 0];
        assert_eq!(parse_header(&head), Err(HeaderError::Version(version)));
        let mut full = vec![0_u8; 18];
        full[..6].copy_from_slice(&head);
        assert_eq!(
            Request::decode(&full),
            Err(DecodeError::Header(HeaderError::Version(version)))
        );
        assert_eq!(
            Reply::decode(&full),
            Err(DecodeError::Header(HeaderError::Version(version)))
        );
    }
}

#[test]
fn an_unknown_kind_is_refused() {
    for byte in [0x00_u8, 0x04, 0x80, 0x84, 0xFF] {
        let head = [1, byte, 0, 0, 0, 0];
        assert_eq!(parse_header(&head), Err(HeaderError::Kind(byte)));
        assert_eq!(
            Request::decode(&head),
            Err(DecodeError::Header(HeaderError::Kind(byte)))
        );
        assert_eq!(
            Reply::decode(&head),
            Err(DecodeError::Header(HeaderError::Kind(byte)))
        );
    }
}

#[test]
fn an_oversize_declared_length_is_refused_before_any_body_exists() {
    for declared in [1_680_009_u32, u32::MAX] {
        let mut head = [1_u8, 3, 0, 0, 0, 0];
        head[2..].copy_from_slice(&declared.to_le_bytes());
        assert_eq!(
            parse_header(&head),
            Err(HeaderError::BodyTooLarge(declared))
        );
    }
    let mut head = [1_u8, 3, 0, 0, 0, 0];
    head[2..].copy_from_slice(&1_680_008_u32.to_le_bytes());
    assert_eq!(
        parse_header(&head),
        Ok(Header {
            kind: Kind::HealthRequest,
            body_len: 1_680_008,
        })
    );
}

#[test]
fn a_frame_one_byte_short_or_long_is_refused() {
    let mut short = frame(0x01, 1_228_812, &vec![0_u8; 1_228_812]);
    short.pop();
    assert_eq!(
        Request::decode(&short),
        Err(DecodeError::BodyLength {
            expected: 1_228_812,
            found: 1_228_811,
        })
    );
    let mut long_body = vec![0_u8; 1_228_812];
    long_body.push(0);
    let long = frame(0x01, 1_228_813, &long_body);
    assert_eq!(
        Request::decode(&long),
        Err(DecodeError::BodyLength {
            expected: 1_228_812,
            found: 1_228_813,
        })
    );
    assert_eq!(
        Request::decode(&frame(0x03, 11, &[0_u8; 11])),
        Err(DecodeError::BodyLength {
            expected: 12,
            found: 11,
        })
    );
    assert_eq!(
        Request::decode(&frame(0x03, 13, &[0_u8; 13])),
        Err(DecodeError::BodyLength {
            expected: 12,
            found: 13,
        })
    );
}

#[test]
fn a_reply_kind_is_refused_by_the_request_decoder_and_the_other_way_round() {
    assert_eq!(
        Request::decode(&[1u8, 0x81, 9, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0]),
        Err(DecodeError::UnexpectedKind(Kind::RowsReply))
    );
    assert_eq!(
        Request::decode(&frame(0x82, 1_680_008, &vec![0_u8; 1_680_008])),
        Err(DecodeError::UnexpectedKind(Kind::RawReply))
    );
    assert_eq!(
        Request::decode(&[1u8, 0x83, 8, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0]),
        Err(DecodeError::UnexpectedKind(Kind::HealthReply))
    );
    assert_eq!(
        Request::decode(&[1u8, 0x8F, 9, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 1]),
        Err(DecodeError::UnexpectedKind(Kind::ErrorReply))
    );
    assert_eq!(
        Reply::decode(&frame(0x01, 1_228_812, &vec![0_u8; 1_228_812])),
        Err(DecodeError::UnexpectedKind(Kind::InferRequest))
    );
    assert_eq!(
        Reply::decode(&frame(0x02, 1_228_812, &vec![0_u8; 1_228_812])),
        Err(DecodeError::UnexpectedKind(Kind::InferRawRequest))
    );
    assert_eq!(
        Reply::decode(&[1u8, 3, 12, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 0xDC, 5, 0, 0]),
        Err(DecodeError::UnexpectedKind(Kind::HealthRequest))
    );
}

#[test]
fn a_rows_reply_with_too_many_rows_is_refused() {
    let mut many = vec![0_u8; 8];
    many.push(21);
    many.extend_from_slice(&[0_u8; 504]);
    assert_eq!(
        Reply::decode(&frame(0x81, 513, &many)),
        Err(DecodeError::RowCount(21))
    );

    let mut mismatch = vec![0_u8; 8];
    mismatch.push(2);
    mismatch.extend_from_slice(&[0_u8; 24]);
    assert_eq!(
        Reply::decode(&frame(0x81, 33, &mismatch)),
        Err(DecodeError::BodyLength {
            expected: 57,
            found: 33,
        })
    );

    assert_eq!(
        Reply::decode(&frame(0x81, 8, &[0_u8; 8])),
        Err(DecodeError::BodyLength {
            expected: 9,
            found: 8,
        })
    );
}

#[test]
fn an_unknown_error_code_is_refused() {
    for code in [0_u8, 4] {
        let mut body = vec![0_u8; 8];
        body.push(code);
        assert_eq!(
            Reply::decode(&frame(0x8F, 9, &body)),
            Err(DecodeError::ErrorCode(code))
        );
    }
}

#[test]
fn a_raw_reply_one_value_short_is_refused() {
    assert_eq!(
        Reply::decode(&frame(0x82, 1_680_004, &vec![0_u8; 1_680_004])),
        Err(DecodeError::BodyLength {
            expected: 1_680_008,
            found: 1_680_004,
        })
    );
}

#[test]
fn every_prefix_of_a_valid_frame_is_refused() {
    let health_request = [1u8, 3, 12, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 0xDC, 5, 0, 0];
    for len in 0..health_request.len() {
        assert!(Request::decode(&health_request[..len]).is_err());
    }

    let mut rows = vec![1_u8, 0x81, 57, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 2];
    rows.extend_from_slice(&[0_u8; 48]);
    for len in 0..rows.len() {
        assert!(Reply::decode(&rows[..len]).is_err());
    }

    let error = [1u8, 0x8F, 9, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 2];
    for len in 0..error.len() {
        assert!(Reply::decode(&error[..len]).is_err());
    }
}

#[test]
fn encode_refuses_what_it_cannot_describe() {
    let rows: Vec<Row> = (0..21usize)
        .map(|_| Row {
            class_id: 0,
            score: 0.0,
            y_min: 0.0,
            x_min: 0.0,
            y_max: 0.0,
            x_max: 0.0,
        })
        .collect();
    let too_many = Reply::Rows { id: 1, rows };
    assert_eq!(too_many.encode(), Err(EncodeError::RowCount(21)));

    let short = Reply::Raw {
        id: 1,
        values: vec![0.0; 419_999],
    };
    assert_eq!(short.encode(), Err(EncodeError::RawLength(419_999)));

    let long = Reply::Raw {
        id: 1,
        values: vec![0.0; 420_001],
    };
    assert_eq!(long.encode(), Err(EncodeError::RawLength(420_001)));
}

#[test]
fn an_unknown_version_wins_over_an_oversize_length_and_an_unknown_kind() {
    let oversize = 1_680_009_u32.to_le_bytes();
    let mut head = [2_u8, 0x42, 0, 0, 0, 0];
    head[2..].copy_from_slice(&oversize);
    assert_eq!(parse_header(&head), Err(HeaderError::Version(2)));
    let mut head = [1_u8, 0x42, 0, 0, 0, 0];
    head[2..].copy_from_slice(&oversize);
    assert_eq!(parse_header(&head), Err(HeaderError::Kind(0x42)));
    let mut head = [1_u8, 3, 0, 0, 0, 0];
    head[2..].copy_from_slice(&oversize);
    assert_eq!(
        parse_header(&head),
        Err(HeaderError::BodyTooLarge(1_680_009))
    );
}

#[test]
fn a_declared_length_that_differs_from_the_bytes_is_refused_even_when_the_bytes_fit_the_kind() {
    let health_body = [2_u8, 1, 0, 0, 0, 0, 0, 0, 0xDC, 5, 0, 0];
    assert_eq!(
        Request::decode(&frame(0x03, 11, &health_body)),
        Err(DecodeError::BodyLength {
            expected: 11,
            found: 12,
        })
    );
    assert_eq!(
        Request::decode(&frame(0x03, 13, &health_body)),
        Err(DecodeError::BodyLength {
            expected: 13,
            found: 12,
        })
    );
    assert_eq!(
        Reply::decode(&frame(0x8F, 8, &[5, 0, 0, 0, 0, 0, 0, 0, 1])),
        Err(DecodeError::BodyLength {
            expected: 8,
            found: 9,
        })
    );
}

#[test]
fn a_wrong_direction_frame_is_refused_before_its_length_is_compared() {
    assert_eq!(
        Request::decode(&frame(0x82, 10, &[0_u8; 12])),
        Err(DecodeError::UnexpectedKind(Kind::RawReply))
    );
    assert_eq!(
        Reply::decode(&frame(0x03, 5, &[0_u8; 12])),
        Err(DecodeError::UnexpectedKind(Kind::HealthRequest))
    );
}

#[test]
fn a_row_count_above_the_limit_is_refused_even_when_the_length_does_not_fit_it() {
    let mut body = vec![0_u8; 8];
    body.push(21);
    body.extend_from_slice(&[0_u8; 48]);
    assert_eq!(
        Reply::decode(&frame(0x81, 57, &body)),
        Err(DecodeError::RowCount(21))
    );
    let mut short = vec![0_u8; 8];
    short.push(255);
    assert_eq!(
        Reply::decode(&frame(0x81, 9, &short)),
        Err(DecodeError::RowCount(255))
    );
}
