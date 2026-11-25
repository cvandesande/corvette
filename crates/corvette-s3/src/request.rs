use std::fmt;

/// The values for one `PutObject` request head, as produced by the signer.
pub struct PutHead<'a> {
    pub target: &'a str,
    pub host: &'a str,
    pub content_length: u64,
    pub x_amz_date: &'a str,
    pub x_amz_content_sha256: &'a str,
    pub authorization: &'a str,
}

impl fmt::Debug for PutHead<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PutHead")
            .field("target", &self.target)
            .field("host", &self.host)
            .field("content_length", &self.content_length)
            .field("x_amz_date", &self.x_amz_date)
            .field("x_amz_content_sha256", &self.x_amz_content_sha256)
            .field("authorization", &"[redacted]")
            .finish()
    }
}

#[derive(Debug)]
pub enum RequestHeadError {
    ControlByte(&'static str),
}

impl fmt::Display for RequestHeadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ControlByte(field) => {
                write!(f, "request head field {field} contains a control byte")
            }
        }
    }
}

impl std::error::Error for RequestHeadError {}

/// The request head bytes for one `PutObject`; the caller appends the body.
/// One request per connection ends with `Connection: close`.
/// # Errors [`RequestHeadError::ControlByte`] unless every field is free of bytes that could split the head.
pub fn put_object_head(head: &PutHead<'_>) -> Result<Vec<u8>, RequestHeadError> {
    check_field("target", head.target)?;
    check_field("host", head.host)?;
    check_field("x-amz-date", head.x_amz_date)?;
    check_field("x-amz-content-sha256", head.x_amz_content_sha256)?;
    check_field("authorization", head.authorization)?;
    Ok(format!(
        "PUT {target} HTTP/1.1\r\n\
         Host: {host}\r\n\
         Content-Length: {content_length}\r\n\
         x-amz-date: {x_amz_date}\r\n\
         x-amz-content-sha256: {x_amz_content_sha256}\r\n\
         Authorization: {authorization}\r\n\
         Connection: close\r\n\r\n",
        target = head.target,
        host = head.host,
        content_length = head.content_length,
        x_amz_date = head.x_amz_date,
        x_amz_content_sha256 = head.x_amz_content_sha256,
        authorization = head.authorization,
    )
    .into_bytes())
}

fn check_field(field: &'static str, value: &str) -> Result<(), RequestHeadError> {
    if value.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        return Err(RequestHeadError::ControlByte(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head<'a>(target: &'a str, host: &'a str, authorization: &'a str) -> PutHead<'a> {
        PutHead {
            target,
            host,
            content_length: 21,
            x_amz_date: "20130524T000000Z",
            x_amz_content_sha256: "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072",
            authorization,
        }
    }

    #[test]
    fn put_object_head_writes_the_expected_bytes() {
        let payload = concat!("Welcome to Amazon S", "3.");
        assert_eq!(payload.len(), 21);
        let good = head(
            "/test%24file.text",
            "examplebucket.s3.amazonaws.com",
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class,Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd",
        );
        assert_eq!(
            put_object_head(&good).expect("clean fixture writes"),
            b"PUT /test%24file.text HTTP/1.1\r\n\
               Host: examplebucket.s3.amazonaws.com\r\n\
               Content-Length: 21\r\n\
               x-amz-date: 20130524T000000Z\r\n\
               x-amz-content-sha256: 44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072\r\n\
               Authorization: AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,SignedHeaders=date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class,Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd\r\n\
               Connection: close\r\n\r\n"
                .to_vec()
        );
    }

    #[test]
    fn a_control_byte_in_host_or_target_is_refused_without_echoing_it() {
        let injected = "example.com\r\nX-Injected: 1";
        for (field, value) in [
            ("host", injected),
            ("target", "/a\r\nX-Injected: 1"),
            ("host", "/a\nX: 1"),
        ] {
            let err = match field {
                "host" => put_object_head(&head("/obj", value, "sig")).expect_err("must refuse"),
                _ => put_object_head(&head(value, "example.com", "sig")).expect_err("must refuse"),
            };
            assert!(matches!(err, RequestHeadError::ControlByte(_)));
            let text = format!("{err}");
            assert!(
                !text.contains("X-Injected"),
                "error echoed the input: {text}"
            );
        }
    }

    #[test]
    fn nul_del_and_tab_in_a_head_field_are_refused() {
        for control in ["a\u{0}b", "a\u{7f}b", "a\tb"] {
            let err = put_object_head(&head(control, "example.com", "sig"))
                .expect_err("control byte must be refused");
            assert!(matches!(err, RequestHeadError::ControlByte("target")));
            assert!(
                !format!("{err}").contains(control),
                "error echoed the input"
            );
        }
    }
}
