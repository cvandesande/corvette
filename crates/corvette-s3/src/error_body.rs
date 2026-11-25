use std::fmt;

pub const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

const FIVE_ENTITIES: [(&str, &str); 5] = [
    ("amp", "&"),
    ("lt", "<"),
    ("gt", ">"),
    ("quot", "\""),
    ("apos", "'"),
];

/// The two documented text elements of a provider error body.
#[derive(Debug, Default)]
pub struct ProviderErrorBody {
    pub code: Option<String>,
    pub message: Option<String>,
}

/// Why an error body was refused instead of parsed.
#[derive(Debug)]
pub enum ErrorBodyError {
    TooLarge,
    Doctype,
    Cdata,
}

impl fmt::Display for ErrorBodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge => f.write_str("error body is larger than 64 KiB"),
            Self::Doctype => {
                f.write_str("error body declares a DOCTYPE; only plain text elements are accepted")
            }
            Self::Cdata => f.write_str(
                "error body uses a CDATA section; only plain text elements are accepted",
            ),
        }
    }
}

impl std::error::Error for ErrorBodyError {}

#[derive(PartialEq)]
enum Open {
    Nothing,
    Code,
    Message,
    Other,
}

/// Reads `Code` and `Message` out of an `<Error>` body: text elements only,
/// the five predefined entities and numeric character references decoded,
/// attributes skipped. Absent elements stay `None`.
/// # Errors
/// [`ErrorBodyError`] when the body is oversized, declares a DOCTYPE, or uses
/// a CDATA section.
pub fn parse_error_body(bytes: &[u8]) -> Result<ProviderErrorBody, ErrorBodyError> {
    if bytes.len() > MAX_ERROR_BODY_BYTES {
        return Err(ErrorBodyError::TooLarge);
    }
    let mut out = ProviderErrorBody::default();
    let mut inside_error = false;
    let mut open = Open::Nothing;
    let mut code_text = String::new();
    let mut message_text = String::new();
    for (index, chunk) in bytes.split(|byte| *byte == b'<').enumerate() {
        let (tag, text) = match index {
            0 => (b"".as_slice(), chunk),
            _ => chunk.iter().position(|byte| *byte == b'>').map_or_else(
                || (chunk, b"".as_slice()),
                |at| {
                    (
                        chunk.get(..at).unwrap_or_default(),
                        chunk.get(at + 1..).unwrap_or_default(),
                    )
                },
            ),
        };
        if index > 0 && !tag.is_empty() {
            if tag.first() == Some(&b'!') {
                if tag.starts_with(b"!DOCTYPE") {
                    return Err(ErrorBodyError::Doctype);
                }
                if tag.starts_with(b"![CDATA[") {
                    return Err(ErrorBodyError::Cdata);
                }
            } else if tag.first() == Some(&b'/') {
                match tag_name(tag.get(1..).unwrap_or_default()) {
                    b"Error" => inside_error = false,
                    b"Code" => out.code = Some(std::mem::take(&mut code_text)),
                    b"Message" => out.message = Some(std::mem::take(&mut message_text)),
                    _ => {}
                }
                open = Open::Nothing;
            } else {
                let self_closed = tag.ends_with(b"/");
                match tag_name(tag) {
                    b"Error" => {
                        inside_error = !self_closed;
                        open = Open::Nothing;
                    }
                    b"Code" if inside_error => {
                        code_text.clear();
                        if self_closed {
                            out.code = Some(String::new());
                        } else {
                            open = Open::Code;
                        }
                    }
                    b"Message" if inside_error => {
                        message_text.clear();
                        if self_closed {
                            out.message = Some(String::new());
                        } else {
                            open = Open::Message;
                        }
                    }
                    _ => open = Open::Other,
                }
            }
        }
        match open {
            Open::Code => append_text(&mut code_text, text),
            Open::Message => append_text(&mut message_text, text),
            Open::Nothing | Open::Other => {}
        }
    }
    Ok(out)
}

fn tag_name(tag: &[u8]) -> &[u8] {
    let stop = tag
        .iter()
        .position(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'/'))
        .unwrap_or(tag.len());
    tag.get(..stop).unwrap_or(tag)
}

const ENTITY_NAME_MAX: usize = 8;

fn append_text(out: &mut String, text: &[u8]) {
    let chars: Vec<char> = String::from_utf8_lossy(text).chars().collect();
    let mut at = 0;
    while let Some(ch) = chars.get(at) {
        if *ch != '&' {
            out.push(*ch);
            at += 1;
            continue;
        }
        // A decodable name's `;` is within ENTITY_NAME_MAX bytes; scanning on would rescan the body per `&`.
        let rest = chars.get(at + 1..).unwrap_or_default();
        let entity: String = match rest
            .iter()
            .take(ENTITY_NAME_MAX + 2)
            .position(|c| *c == ';')
        {
            Some(offset) if offset <= ENTITY_NAME_MAX => rest.iter().take(offset).collect(),
            _ => String::new(),
        };
        if entity.is_empty() {
            out.push('&');
            at += 1;
            continue;
        }
        let decoded = FIVE_ENTITIES
            .iter()
            .find(|(name, _)| *name == entity)
            .map(|(_, text)| (*text).to_owned())
            .or_else(|| {
                entity
                    .strip_prefix('#')
                    .and_then(numeric_reference)
                    .map(|n| char::from_u32(n).unwrap_or('\u{FFFD}').to_string())
            });
        if let Some(text) = decoded {
            out.push_str(&text);
            at += entity.len() + 2;
        } else {
            out.push('&');
            at += 1;
        }
    }
}

fn numeric_reference(digits: &str) -> Option<u32> {
    if let Some(radix16) = digits
        .strip_prefix('x')
        .or_else(|| digits.strip_prefix('X'))
    {
        return u32::from_str_radix(radix16, 16).ok();
    }
    digits.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_and_message_are_read_from_an_error_body() {
        let body = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                      <Error><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message>\
                      <RequestId>NMSJ5FH9F9V9Z9AX</RequestId><HostId>f/9c</HostId></Error>";
        let got = parse_error_body(body).expect("parses");
        assert_eq!(got.code.as_deref(), Some("NoSuchKey"));
        assert_eq!(
            got.message.as_deref(),
            Some("The specified key does not exist.")
        );
    }

    #[test]
    fn predefined_entities_and_numeric_references_are_decoded() {
        let body = concat!(
            "<Error><Code>Ok</Code><Message>a &amp; b &lt; c &gt; d &quot;e&quot; &apos;f&apos; \u{e9} and &#",
            "70;&#x41;</Message></Error>",
        )
        .as_bytes();
        let got = parse_error_body(body).expect("parses");
        assert_eq!(
            got.message.as_deref(),
            Some("a & b < c > d \"e\" 'f' \u{e9} and FA")
        );
    }

    #[test]
    fn absent_elements_yield_none_fields() {
        let got = parse_error_body(b"<Error><Resource>x</Resource></Error>").expect("parses");
        assert_eq!(got.code, None);
        assert_eq!(got.message, None);
    }

    #[test]
    fn attributes_on_the_tracked_elements_are_skipped() {
        let body = b"<Error><Code xml:lang=\"en\">AccessDenied</Code><Message>no</Message></Error>";
        let got = parse_error_body(body).expect("parses");
        assert_eq!(got.code.as_deref(), Some("AccessDenied"));
    }

    #[test]
    fn a_doctype_is_refused() {
        let body = b"<?xml version=\"1.0\"?><!DOCTYPE foo><Error><Code>X</Code></Error>";
        let err = parse_error_body(body).expect_err("DOCTYPE must be refused");
        assert!(matches!(err, ErrorBodyError::Doctype));
    }

    #[test]
    fn a_cdata_section_is_refused() {
        let body = b"<Error><Message><![CDATA[x]]></Message></Error>";
        let err = parse_error_body(body).expect_err("CDATA must be refused");
        assert!(matches!(err, ErrorBodyError::Cdata));
    }

    #[test]
    fn an_oversized_body_is_refused() {
        let mut body = Vec::new();
        body.extend_from_slice(b"<Error><Message>");
        body.extend(std::iter::repeat_n(b'x', MAX_ERROR_BODY_BYTES));
        assert!(matches!(
            parse_error_body(&body),
            Err(ErrorBodyError::TooLarge)
        ));
    }

    #[test]
    fn an_escaped_ampersand_survives_in_a_camera_name() {
        let body = b"<Error><Code>SlowDown</Code><Message>camera cam &amp; yard upload failed</Message></Error>";
        let got = parse_error_body(body).expect("parses");
        assert_eq!(
            got.message.as_deref(),
            Some("camera cam & yard upload failed")
        );
    }

    #[test]
    fn a_name_longer_than_the_entity_limit_stays_literal() {
        let body =
            b"<Error><Code>Ok</Code><Message>a &notanent9; b &notanentity; c</Message></Error>";
        let got = parse_error_body(body).expect("parses");
        assert_eq!(
            got.message.as_deref(),
            Some("a &notanent9; b &notanentity; c")
        );
    }

    #[test]
    fn a_message_full_of_ampersands_decodes_quickly_and_literally() {
        let ampersands = "&".repeat(60_000);
        let body = format!("<Error><Message>{ampersands}</Message></Error>");
        assert!(body.len() <= MAX_ERROR_BODY_BYTES);
        let start = std::time::Instant::now();
        let got = parse_error_body(body.as_bytes()).expect("parses");
        let elapsed = start.elapsed();
        assert_eq!(got.message.as_deref(), Some(ampersands.as_str()));
        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "took {elapsed:?}"
        );
    }

    #[test]
    fn elements_outside_the_error_root_are_ignored() {
        let body = b"<Other><Code>NotThis</Code></Other><Error><Code>NoSuchBucket</Code></Error>";
        let got = parse_error_body(body).expect("parses");
        assert_eq!(got.code.as_deref(), Some("NoSuchBucket"));
    }
}
