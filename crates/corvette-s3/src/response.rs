use std::fmt;

use httparse::EMPTY_HEADER;
use httparse::Response;

pub const MAX_HEAD_BYTES: usize = 16 * 1024;

/// `Retry-After` values are clamped to this many seconds.
pub const RETRY_AFTER_CAP_SECONDS: u32 = 3_600;

const MAX_HEADER_FIELDS: usize = 64;

#[derive(Debug)]
pub struct ResponseHead {
    pub status: u16,
    pub retry_after_secs: Option<u32>,
    pub content_length: Option<u64>,
}

#[derive(Debug)]
pub enum HeadError {
    TooLarge,
    Incomplete,
    Malformed(&'static str),
    TransferEncoding,
}

impl fmt::Display for HeadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("response head is larger than 16 KiB"),
            Self::Incomplete => f.write_str("response head ended before its header block did"),
            Self::Malformed(reason) => write!(f, "response head is malformed: {reason}"),
            Self::TransferEncoding => f.write_str(
                "response head names a transfer encoding; only unencoded bodies are accepted",
            ),
        }
    }
}

impl std::error::Error for HeadError {}

/// Parses a complete HTTP/1.1 response head: status, capped `Retry-After`, `Content-Length`.
/// # Errors
/// [`HeadError`] when oversized, truncated, malformed, or transfer-encoded.
pub fn parse_head(bytes: &[u8]) -> Result<ResponseHead, HeadError> {
    let end = find_head_end(bytes).ok_or(if bytes.len() >= MAX_HEAD_BYTES {
        HeadError::TooLarge
    } else {
        HeadError::Incomplete
    })?;
    let head = bytes.get(..end + 4).ok_or(HeadError::Incomplete)?;
    let mut slots = [EMPTY_HEADER; MAX_HEADER_FIELDS];
    let mut response = Response::new(&mut slots);
    match response.parse(head) {
        Ok(httparse::Status::Complete(_)) => {}
        Ok(httparse::Status::Partial) => return Err(HeadError::Incomplete),
        Err(httparse::Error::TooManyHeaders) => {
            return Err(HeadError::Malformed("more header fields than expected"));
        }
        Err(_other) => return Err(HeadError::Malformed("status line or header name")),
    }
    let status = response
        .code
        .ok_or(HeadError::Malformed("status line has no code"))?;
    let mut retry_after_secs = None;
    let mut content_length = None;
    for header in response.headers {
        if header.name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(HeadError::TransferEncoding);
        }
        if header.name.eq_ignore_ascii_case("retry-after") {
            retry_after_secs = std::str::from_utf8(header.value)
                .ok()
                .and_then(parse_retry_after);
        }
        if header.name.eq_ignore_ascii_case("content-length") {
            let length = std::str::from_utf8(header.value)
                .ok()
                .and_then(|value| value.trim().parse::<u64>().ok())
                .ok_or(HeadError::Malformed("content-length is not a number"))?;
            if content_length.is_some_and(|seen| seen != length) {
                return Err(HeadError::Malformed("conflicting content-length values"));
            }
            content_length = Some(length);
        }
    }
    Ok(ResponseHead {
        status,
        retry_after_secs,
        content_length,
    })
}

// All-digits delta-seconds only; a digit string too big for u32 exceeds the
// cap anyway, so it lands on the cap rather than on no delay.
fn parse_retry_after(value: &str) -> Option<u32> {
    let digits = value.trim();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let capped = digits
        .parse::<u32>()
        .map_or(RETRY_AFTER_CAP_SECONDS, |s| s.min(RETRY_AFTER_CAP_SECONDS));
    Some(capped)
}

fn find_head_end(bytes: &[u8]) -> Option<usize> {
    let limit = bytes.len().min(MAX_HEAD_BYTES);
    bytes
        .get(..limit)?
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_ok_head_yields_status_and_content_length() {
        let got = parse_head(b"HTTP/1.1 200 OK\r\nContent-Length: 1234\r\n\r\n").expect("parses");
        assert_eq!(got.status, 200);
        assert_eq!(got.content_length, Some(1234));
        assert_eq!(got.retry_after_secs, None);
    }

    #[test]
    fn a_503_head_reads_retry_after_seconds() {
        let got = parse_head(b"HTTP/1.1 503 Slow Down\r\nRetry-After: 2\r\n\r\n").expect("parses");
        assert_eq!(got.status, 503);
        assert_eq!(got.retry_after_secs, Some(2));
    }

    #[test]
    fn a_retry_after_http_date_is_none() {
        let got = parse_head(
            b"HTTP/1.1 503 Slow Down\r\nRetry-After: Fri, 24 May 2013 00:00:00 GMT\r\n\r\n",
        )
        .expect("parses");
        assert_eq!(got.retry_after_secs, None);
    }

    #[test]
    fn a_huge_retry_after_number_is_capped() {
        let got =
            parse_head(b"HTTP/1.1 503 Slow Down\r\nRetry-After: 99999\r\n\r\n").expect("parses");
        assert_eq!(got.retry_after_secs, Some(RETRY_AFTER_CAP_SECONDS));
    }

    #[test]
    fn a_retry_after_beyond_the_u32_range_is_capped() {
        let got = parse_head(b"HTTP/1.1 503 Slow Down\r\nRetry-After: 99999999999\r\n\r\n")
            .expect("all digits must yield the cap");
        assert_eq!(got.retry_after_secs, Some(RETRY_AFTER_CAP_SECONDS));
    }

    #[test]
    fn a_retry_after_that_is_not_all_digits_is_none() {
        let got = parse_head(b"HTTP/1.1 503 Slow Down\r\nRetry-After: +5\r\n\r\n").expect("parses");
        assert_eq!(got.retry_after_secs, None);
    }

    #[test]
    fn duplicate_content_lengths_must_agree() {
        let got = parse_head(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nContent-Length: 9\r\n\r\n")
            .expect("equal duplicates are accepted");
        assert_eq!(got.content_length, Some(9));
        let err = parse_head(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nContent-Length: 5\r\n\r\n")
            .expect_err("conflicting lengths must be refused");
        assert!(matches!(err, HeadError::Malformed(_)));
    }

    #[test]
    fn an_over_large_head_is_refused() {
        let mut filler = vec![b'a'; MAX_HEAD_BYTES + 8];
        filler.splice(0..0, *b"HTTP/1.1 200 OK\r\nX-Fill: ");
        assert!(matches!(parse_head(&filler), Err(HeadError::TooLarge)));
    }

    #[test]
    fn a_chunked_response_is_refused() {
        let err = parse_head(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
            .expect_err("chunked must be refused");
        assert!(matches!(err, HeadError::TransferEncoding));
        let err = parse_head(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n")
            .expect_err("any transfer encoding must be refused");
        assert!(matches!(err, HeadError::TransferEncoding));
    }

    #[test]
    fn a_truncated_head_is_incomplete() {
        let err = parse_head(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n")
            .expect_err("truncated head must be refused");
        assert!(matches!(err, HeadError::Incomplete));
    }

    #[test]
    fn a_header_block_without_a_status_line_is_malformed() {
        let err = parse_head(b"Host: example\r\nContent-Length: 3\r\n\r\n")
            .expect_err("missing status line must be refused");
        assert!(matches!(err, HeadError::Malformed(_)));
    }
}
