use std::fmt::{self, Write};

use aws_lc_rs::digest;
use aws_lc_rs::hmac;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// Bytes percent-encoded in canonical paths and query parts: every byte
/// outside the RFC 3986 unreserved set `A-Za-z0-9-_.~`.
const ENCODE_UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Provider credentials for signing requests.
pub struct Credentials {
    access_key_id: String,
    secret_access_key: String,
}

impl Credentials {
    #[must_use]
    pub const fn with(access_key_id: String, secret_access_key: String) -> Self {
        Self {
            access_key_id,
            secret_access_key,
        }
    }

    #[must_use]
    pub fn access_key_id(&self) -> &str {
        &self.access_key_id
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &"[redacted]")
            .finish()
    }
}

/// Request timestamp in the `x-amz-date` form; the signer never reads a clock.
#[derive(Debug)]
pub struct Timestamp {
    date: String,
    amz: String,
}

impl Timestamp {
    /// Builds a timestamp from UTC calendar fields.
    /// # Errors
    /// [`TimestampError`] when a field is out of range.
    pub fn new(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<Self, TimestampError> {
        if !(1..=12).contains(&month) {
            return Err(TimestampError::Month);
        }
        if !(1..=31).contains(&day) || u16::from(day) > days_in_month(year, month) {
            return Err(TimestampError::Day);
        }
        if hour > 23 || minute > 59 || second > 59 {
            return Err(TimestampError::ClockTime);
        }
        let date = format!("{year:04}{month:02}{day:02}");
        Ok(Self {
            amz: format!("{date}T{hour:02}{minute:02}{second:02}Z"),
            date,
        })
    }

    fn date(&self) -> &str {
        &self.date
    }

    fn amz(&self) -> &str {
        &self.amz
    }
}

/// Month lengths; February: leap years are divisible by 4, except centuries not by 400.
const fn days_in_month(year: u16, month: u8) -> u16 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

/// A timestamp calendar field was out of range.
#[derive(Debug)]
pub enum TimestampError {
    Month,
    Day,
    ClockTime,
}

impl fmt::Display for TimestampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Month => f.write_str("timestamp month must be 1 to 12"),
            Self::Day => f.write_str("timestamp day is out of range for its month"),
            Self::ClockTime => {
                f.write_str("timestamp hour must be below 24, minute and second below 60")
            }
        }
    }
}

impl std::error::Error for TimestampError {}

#[derive(Debug)]
pub struct Header<'a> {
    pub name: &'a str,
    pub value: &'a str,
}

#[derive(Debug)]
pub struct QueryParam<'a> {
    pub name: &'a str,
    pub value: &'a str,
}

/// The request half a signature covers. `path` is the decoded request path;
/// a missing leading slash is added, and each segment is percent-encoded once.
#[derive(Debug)]
pub struct SigningInput<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a [QueryParam<'a>],
    pub headers: &'a [Header<'a>],
}

/// The request half could not be signed as given.
#[derive(Debug)]
pub enum SignError {
    PayloadHashInvalid,
}

impl fmt::Display for SignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadHashInvalid => f.write_str(
                "payload hash must be the 64 lowercase hex digits of the body's real SHA-256; unsigned payloads are never signed",
            ),
        }
    }
}

impl std::error::Error for SignError {}

/// The signed values plus the intermediate strings, so callers and tests can
/// inspect exactly what was signed.
pub struct Signature {
    pub authorization: String,
    pub x_amz_date: String,
    pub signed_headers: String,
    pub canonical_request: String,
    pub string_to_sign: String,
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Debug must not expose the signature or the credential scope.
        f.debug_struct("Signature")
            .field("x_amz_date", &self.x_amz_date)
            .field("signed_headers", &self.signed_headers)
            .field("authorization", &"[redacted]")
            .finish_non_exhaustive()
    }
}

/// Signs one request with `AWS4-HMAC-SHA256` for service `s3`: canonical
/// request, string to sign, scoped key chain and signature. A pure function.
/// # Errors
/// [`SignError::PayloadHashInvalid`] unless the payload hash is 64 lowercase
/// hex digits, so `UNSIGNED-PAYLOAD` can never be signed.
pub fn sign(
    credentials: &Credentials,
    region: &str,
    input: &SigningInput<'_>,
    payload_sha256_hex: &str,
    time: &Timestamp,
) -> Result<Signature, SignError> {
    if payload_sha256_hex.len() != 64
        || !payload_sha256_hex
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(SignError::PayloadHashInvalid);
    }
    let uri = canonical_uri(input.path);
    let query = canonical_query(input.query);
    let mut canon: Vec<(String, String)> = input
        .headers
        .iter()
        .map(|h| {
            (
                h.name.trim().to_ascii_lowercase(),
                collapse_spaces(h.value.trim()),
            )
        })
        .collect();
    canon.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers: String = canon.iter().fold(String::new(), |mut acc, (name, value)| {
        acc.push_str(name);
        acc.push(':');
        acc.push_str(value);
        acc.push('\n');
        acc
    });
    let signed_headers = canon
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<&str>>()
        .join(";");
    let canonical_request = format!(
        "{}\n{uri}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_sha256_hex}",
        input.method
    );
    let scope = format!("{}/{region}/s3/aws4_request", time.date());
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        time.amz(),
        sha256_hex(canonical_request.as_bytes())
    );
    let seed = format!("AWS4{}", credentials.secret_access_key);
    let date_key = hmac_bytes(seed.as_bytes(), time.date().as_bytes());
    let region_key = hmac_bytes(&date_key, region.as_bytes());
    let service_key = hmac_bytes(&region_key, b"s3");
    let signing_key = hmac_bytes(&service_key, b"aws4_request");
    let signature_hex = hex_lower(&hmac_bytes(&signing_key, string_to_sign.as_bytes()));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature_hex}",
        credentials.access_key_id
    );
    Ok(Signature {
        authorization,
        x_amz_date: time.amz().to_owned(),
        signed_headers,
        canonical_request,
        string_to_sign,
    })
}

/// Lowercase hex SHA-256 of `data`: the value for `x-amz-content-sha256`.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    hex_lower(digest::digest(&digest::SHA256, data).as_ref())
}

fn hmac_bytes(key: &[u8], msg: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), msg)
        .as_ref()
        .to_vec()
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn canonical_uri(path: &str) -> String {
    let normalized = if path.is_empty() {
        "/".to_owned()
    } else if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    normalized
        .split('/')
        .map(|segment| utf8_percent_encode(segment, ENCODE_UNRESERVED).to_string())
        .collect::<Vec<String>>()
        .join("/")
}

fn canonical_query(params: &[QueryParam<'_>]) -> String {
    let mut encoded: Vec<(String, String)> = params
        .iter()
        .map(|p| {
            (
                utf8_percent_encode(p.name, ENCODE_UNRESERVED).to_string(),
                utf8_percent_encode(p.value, ENCODE_UNRESERVED).to_string(),
            )
        })
        .collect();
    encoded.sort();
    encoded
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<String>>()
        .join("&")
}

fn collapse_spaces(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_was_space = false;
    for ch in value.chars() {
        if ch.is_ascii_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(ch);
            last_was_space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AKID: &str = "AKIAIOSFODNN7EXAMPLE";
    const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
    const REGION: &str = "us-east-1";
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const HOST: &str = "examplebucket.s3.amazonaws.com";

    fn credentials() -> Credentials {
        Credentials::with(AKID.to_owned(), SECRET.to_owned())
    }

    fn morning() -> Timestamp {
        Timestamp::new(2013, 5, 24, 0, 0, 0).expect("fixed fields are in range")
    }

    fn signed(input: &SigningInput<'_>, payload: &str) -> Signature {
        sign(&credentials(), REGION, input, payload, &morning()).expect("vector inputs sign")
    }

    // The page joins the parts with a bare comma; current SDKs emit ", " and the separator is not signed.
    fn page_authorization(page: &str) -> String {
        page.replace(',', ", ")
    }

    #[test]
    fn get_object_with_range_reproduces_published_calculation() {
        let input = SigningInput {
            method: "GET",
            path: "/test.txt",
            query: &[],
            headers: &[
                Header {
                    name: "x-amz-date",
                    value: "20130524T000000Z",
                },
                Header {
                    name: "Host",
                    value: HOST,
                },
                Header {
                    name: "Range",
                    value: "bytes=0-9",
                },
                Header {
                    name: "x-amz-content-sha256",
                    value: EMPTY_SHA256,
                },
            ],
        };
        assert_eq!(sha256_hex(b""), EMPTY_SHA256);
        let got = signed(&input, EMPTY_SHA256);
        assert_eq!(
            got.canonical_request,
            "GET\n/test.txt\n\nhost:examplebucket.s3.amazonaws.com\nrange:bytes=0-9\nx-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nx-amz-date:20130524T000000Z\n\nhost;range;x-amz-content-sha256;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            got.string_to_sign,
            concat!(
                "AWS4-HMAC-SHA256\n",
                "20130524T000000Z\n",
                "20130524/us-east-1/s3/aws4_request\n",
                "7344ae5b7ee6c3e7e6b0fe0640412a37625d1fbfff95c48bbb2dc43964946972"
            )
        );
        assert_eq!(
            got.authorization,
            page_authorization(
                "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
            )
        );
    }

    #[test]
    fn put_object_reproduces_published_calculation() {
        let payload = concat!("Welcome to Amazon S", "3.");
        let body_sha = "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072";
        assert_eq!(sha256_hex(payload.as_bytes()), body_sha);
        let input = SigningInput {
            method: "PUT",
            path: "/test$file.text",
            query: &[],
            headers: &[
                Header {
                    name: "x-amz-storage-class",
                    value: "REDUCED_REDUNDANCY",
                },
                Header {
                    name: "Host",
                    value: HOST,
                },
                Header {
                    name: "x-amz-date",
                    value: "20130524T000000Z",
                },
                Header {
                    name: "Date",
                    value: "Fri, 24 May 2013 00:00:00 GMT",
                },
                Header {
                    name: "x-amz-content-sha256",
                    value: body_sha,
                },
            ],
        };
        let got = signed(&input, body_sha);
        assert_eq!(
            got.canonical_request,
            "PUT\n/test%24file.text\n\ndate:Fri, 24 May 2013 00:00:00 GMT\nhost:examplebucket.s3.amazonaws.com\nx-amz-content-sha256:44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072\nx-amz-date:20130524T000000Z\nx-amz-storage-class:REDUCED_REDUNDANCY\n\ndate;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class\n44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        assert_eq!(
            got.string_to_sign,
            concat!(
                "AWS4-HMAC-SHA256\n",
                "20130524T000000Z\n",
                "20130524/us-east-1/s3/aws4_request\n",
                "9e0e90d9c76de8fa5b200d8c849cd5b8dc7a3be3951ddb7f6a76b4158342019d"
            )
        );
        assert_eq!(
            got.authorization,
            page_authorization(
                "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class,Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
            )
        );
    }

    #[test]
    fn get_bucket_lifecycle_reproduces_published_calculation() {
        let input = SigningInput {
            method: "GET",
            path: "/",
            query: &[QueryParam {
                name: "lifecycle",
                value: "",
            }],
            headers: &[
                Header {
                    name: "Host",
                    value: HOST,
                },
                Header {
                    name: "x-amz-content-sha256",
                    value: EMPTY_SHA256,
                },
                Header {
                    name: "x-amz-date",
                    value: "20130524T000000Z",
                },
            ],
        };
        let got = signed(&input, EMPTY_SHA256);
        assert_eq!(
            got.canonical_request,
            "GET\n/\nlifecycle=\nhost:examplebucket.s3.amazonaws.com\nx-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nx-amz-date:20130524T000000Z\n\nhost;x-amz-content-sha256;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            got.string_to_sign,
            concat!(
                "AWS4-HMAC-SHA256\n",
                "20130524T000000Z\n",
                "20130524/us-east-1/s3/aws4_request\n",
                "9766c798316ff2757b517bc739a67f6213b4ab36dd5da2f94eaebf79c77395ca"
            )
        );
        assert_eq!(
            got.authorization,
            page_authorization(
                "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=host;x-amz-content-sha256;x-amz-date,Signature=fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543"
            )
        );
    }

    #[test]
    fn get_bucket_list_reproduces_published_calculation() {
        let input = SigningInput {
            method: "GET",
            path: "/",
            query: &[
                QueryParam {
                    name: "prefix",
                    value: "J",
                },
                QueryParam {
                    name: "max-keys",
                    value: "2",
                },
            ],
            headers: &[
                Header {
                    name: "Host",
                    value: HOST,
                },
                Header {
                    name: "x-amz-content-sha256",
                    value: EMPTY_SHA256,
                },
                Header {
                    name: "x-amz-date",
                    value: "20130524T000000Z",
                },
            ],
        };
        let got = signed(&input, EMPTY_SHA256);
        assert_eq!(
            got.canonical_request,
            "GET\n/\nmax-keys=2&prefix=J\nhost:examplebucket.s3.amazonaws.com\nx-amz-content-sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\nx-amz-date:20130524T000000Z\n\nhost;x-amz-content-sha256;x-amz-date\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            got.string_to_sign,
            concat!(
                "AWS4-HMAC-SHA256\n",
                "20130524T000000Z\n",
                "20130524/us-east-1/s3/aws4_request\n",
                "df57d21db20da04d7fa30298dd4488ba3a2b47ca3a489c74750e0f1e7df1b9b7"
            )
        );
        assert_eq!(
            got.authorization,
            page_authorization(
                "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=host;x-amz-content-sha256;x-amz-date,Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
            )
        );
    }

    fn canonical_uri_line(path: &str) -> String {
        let input = SigningInput {
            method: "GET",
            path,
            query: &[],
            headers: &[Header {
                name: "Host",
                value: HOST,
            }],
        };
        let got = signed(&input, EMPTY_SHA256);
        got.canonical_request
            .lines()
            .nth(1)
            .unwrap_or_default()
            .to_owned()
    }

    #[test]
    fn key_encoding_table_matches_the_canonical_rules() {
        let cases = [
            ("", "/"),
            ("/", "/"),
            ("/lead", "/lead"),
            ("/a b", "/a%20b"),
            ("/a+b", "/a%2Bb"),
            ("/~", "/~"),
            ("/a*b", "/a%2Ab"),
            ("/caf\u{e9}", "/caf%C3%A9"),
            ("/a=b", "/a%3Db"),
            ("/a&b", "/a%26b"),
            ("/a/b", "/a/b"),
            ("/100%", "/100%25"),
            ("/test$file.text", "/test%24file.text"),
            ("/k/", "/k/"),
        ];
        for (path, want) in cases {
            assert_eq!(canonical_uri_line(path), want, "path {path:?}");
        }
    }

    #[test]
    fn headers_are_lowercased_sorted_trimmed_and_collapsed() {
        let input = SigningInput {
            method: "GET",
            path: "/obj",
            query: &[],
            headers: &[
                Header {
                    name: "X-Amz-Meta-A",
                    value: "  one   two  ",
                },
                Header {
                    name: "HOST",
                    value: " example ",
                },
            ],
        };
        let got = signed(&input, EMPTY_SHA256);
        assert!(
            got.canonical_request
                .contains("host:example\nx-amz-meta-a:one two\n\n"),
            "canonical headers wrong: {:?}",
            got.canonical_request
        );
        assert_eq!(got.signed_headers, "host;x-amz-meta-a");
    }

    #[test]
    fn debug_output_redacts_the_secret_key() {
        let shown = format!("{:?}", credentials());
        assert!(!shown.contains(SECRET), "secret leaked into debug output");
        assert!(
            !shown.contains("wJalrXUtnFEMI"),
            "secret leaked into debug output"
        );
        assert!(shown.contains("[redacted]"));
        assert!(shown.contains(AKID));
    }

    #[test]
    fn an_unsigned_payload_hash_is_refused() {
        let credentials = credentials();
        let input = SigningInput {
            method: "PUT",
            path: "/obj",
            query: &[],
            headers: &[Header {
                name: "Host",
                value: HOST,
            }],
        };
        for bad in [
            "UNSIGNED-PAYLOAD",
            "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855",
            "abc",
        ] {
            let err = sign(&credentials, REGION, &input, bad, &morning())
                .expect_err("bad payload hashes must be refused");
            assert!(matches!(err, SignError::PayloadHashInvalid));
        }
    }

    #[test]
    fn an_out_of_range_timestamp_field_is_refused() {
        assert!(Timestamp::new(2013, 13, 1, 0, 0, 0).is_err());
        assert!(Timestamp::new(2013, 1, 32, 0, 0, 0).is_err());
        assert!(Timestamp::new(2013, 1, 1, 24, 0, 0).is_err());
        assert!(Timestamp::new(2013, 1, 1, 0, 60, 0).is_err());
    }

    #[test]
    fn a_day_that_does_not_exist_in_its_month_is_refused() {
        let rejects = [(2013, 2, 30), (2013, 4, 31), (2013, 2, 29), (1900, 2, 29)];
        for (year, month, day) in rejects {
            assert!(
                Timestamp::new(year, month, day, 0, 0, 0).is_err(),
                "{year}-{month:02}-{day:02} must be refused"
            );
        }
        let accepts = [(2000, 2, 29), (2024, 2, 29), (2013, 4, 30), (2013, 12, 31)];
        for (year, month, day) in accepts {
            assert!(
                Timestamp::new(year, month, day, 0, 0, 0).is_ok(),
                "{year}-{month:02}-{day:02} must be accepted"
            );
        }
    }

    #[test]
    fn debug_output_hides_the_signature_and_credential_scope() {
        let input = SigningInput {
            method: "GET",
            path: "/",
            query: &[],
            headers: &[Header {
                name: "Host",
                value: HOST,
            }],
        };
        let got = signed(&input, EMPTY_SHA256);
        let signature_hex = got
            .authorization
            .rsplit('=')
            .next()
            .expect("authorization ends with the signature");
        assert!(!signature_hex.is_empty());
        let shown = format!("{got:?}");
        assert!(
            !shown.contains(signature_hex),
            "signature leaked into debug output"
        );
        assert!(
            !shown.contains("/us-east-1/s3/aws4_request"),
            "credential scope leaked into debug output"
        );
        assert!(shown.contains("[redacted]"));
        assert!(shown.contains("host"), "signed headers stay visible");
    }

    #[test]
    fn the_x_amz_date_string_and_scope_follow_the_timestamp() {
        let input = SigningInput {
            method: "GET",
            path: "/",
            query: &[],
            headers: &[Header {
                name: "Host",
                value: HOST,
            }],
        };
        let got = signed(&input, EMPTY_SHA256);
        assert_eq!(got.x_amz_date, "20130524T000000Z");
        assert!(
            got.authorization
                .contains("/20130524/us-east-1/s3/aws4_request")
        );
    }
}
