//! Parsing and reads for `GET /corvette/api/v1/activity` (issue #5).

use std::collections::BTreeMap;

/// Longest accepted range: 31 days in seconds (issue #5 D-4).
pub const MAX_RANGE_SECONDS: f64 = 2_678_400.0;

/// Validated parameters of one activity query.
#[derive(Clone, Debug, PartialEq)]
pub struct ActivityQuery {
    /// Lower end of the requested half-open time range, in epoch seconds.
    pub after: f64,
    /// Upper end of the range; strictly greater than `after` and at most
    /// [`MAX_RANGE_SECONDS`] seconds after it.
    pub before: f64,
    /// Camera identifiers to restrict to; `None` when the request named none.
    pub cameras: Option<Vec<String>>,
    /// Whether reviews are requested.
    pub include_reviews: bool,
    /// Whether motion buckets are requested.
    pub include_motion: bool,
}

impl ActivityQuery {
    /// Parses the raw query of an activity request: the text after `?`,
    /// without the `?`, and empty when the request has none.
    ///
    /// # Errors
    ///
    /// Returns a message naming the offending parameter when a parameter is
    /// empty, unknown, repeated, missing its value, missing, or outside its
    /// rules, and quotes the failing text when a percent escape is malformed
    /// or decodes to invalid UTF-8.
    pub fn parse(query: &str) -> Result<Self, String> {
        let params = parse_parameters(query)?;
        let after = params
            .get("after")
            .map(String::as_str)
            .ok_or_else(|| "missing query parameter `after`".to_owned())
            .and_then(|value| parse_timestamp("after", value))?;
        let before = params
            .get("before")
            .map(String::as_str)
            .ok_or_else(|| "missing query parameter `before`".to_owned())
            .and_then(|value| parse_timestamp("before", value))?;
        if after >= before {
            return Err("`before` must be greater than `after`".to_owned());
        }
        if before - after > MAX_RANGE_SECONDS {
            return Err(format!(
                "`before` is more than {MAX_RANGE_SECONDS} seconds after `after`"
            ));
        }

        let cameras = params.get("cameras").map(|value| {
            value
                .split(',')
                .filter(|camera| !camera.is_empty())
                .map(str::to_owned)
                .collect()
        });

        let mut include_reviews = true;
        let mut include_motion = true;
        if let Some(value) = params.get("include") {
            include_reviews = false;
            include_motion = false;
            for piece in value.split(',') {
                match piece {
                    "" => {}
                    "reviews" => include_reviews = true,
                    "motion" => include_motion = true,
                    other => return Err(format!("`include` does not accept `{other}`")),
                }
            }
            if !include_reviews && !include_motion {
                return Err("`include` must name `reviews`, `motion`, or both".to_owned());
            }
        }

        Ok(Self {
            after,
            before,
            cameras,
            include_reviews,
            include_motion,
        })
    }
}

/// Splits the query on `&`, decodes each name and value, and rejects empty,
/// unknown, value-less, or repeated parameters.
fn parse_parameters(query: &str) -> Result<BTreeMap<String, String>, String> {
    let mut params = BTreeMap::new();
    if query.is_empty() {
        return Ok(params);
    }
    for segment in query.split('&') {
        if segment.is_empty() {
            return Err("empty query parameter".to_owned());
        }
        let Some((raw_name, raw_value)) = segment.split_once('=') else {
            let name = percent_decode(segment)?;
            return Err(format!("query parameter `{name}` is missing its value"));
        };
        let name = percent_decode(raw_name)?;
        if !matches!(name.as_str(), "after" | "before" | "cameras" | "include") {
            return Err(format!("unknown query parameter `{name}`"));
        }
        if params.contains_key(name.as_str()) {
            return Err(format!("repeated query parameter `{name}`"));
        }
        params.insert(name, percent_decode(raw_value)?);
    }
    Ok(params)
}

/// Parses one timestamp parameter: a finite, non-negative number of seconds.
fn parse_timestamp(name: &str, value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .ok_or_else(|| format!("`{name}` must be a finite number of seconds, at least 0"))
}

/// Expands `%XX` escapes (hex digits in either case) to their bytes and
/// requires the result to be UTF-8. `+` stays `+`: this grammar has no
/// plus-as-space convention.
fn percent_decode(text: &str) -> Result<String, String> {
    let mut decoded = Vec::with_capacity(text.len());
    let mut bytes = text.as_bytes().iter();
    while let Some(&byte) = bytes.next() {
        if byte != b'%' {
            decoded.push(byte);
            continue;
        }
        let (Some(&first), Some(&second)) = (bytes.next(), bytes.next()) else {
            return Err(format!("`{text}` ends in an incomplete percent escape"));
        };
        let (Some(high), Some(low)) = (hex_digit(first), hex_digit(second)) else {
            return Err(format!(
                "`{text}` has a percent escape that is not two hex digits"
            ));
        };
        decoded.push(high * 16 + low);
    }
    String::from_utf8(decoded).map_err(|_| format!("`{text}` is not valid UTF-8"))
}

const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_query_parses_every_parameter() {
        let query = ActivityQuery::parse(
            "after=1760227200&before=1760313600.5&cameras=front,back&include=reviews",
        )
        .expect("valid");
        assert_eq!(
            query,
            ActivityQuery {
                after: 1_760_227_200.0,
                before: 1_760_313_600.5,
                cameras: Some(vec!["front".to_owned(), "back".to_owned()]),
                include_reviews: true,
                include_motion: false,
            }
        );
    }

    #[test]
    fn absent_cameras_and_include_take_their_defaults() {
        let query = ActivityQuery::parse("after=100&before=200").expect("valid");
        assert_eq!(query.cameras, None);
        assert!(query.include_reviews);
        assert!(query.include_motion);
    }

    #[test]
    fn missing_after_or_before_is_named() {
        let err = ActivityQuery::parse("before=200").unwrap_err();
        assert!(err.contains("`after`"), "{err}");
        let err = ActivityQuery::parse("after=100").unwrap_err();
        assert!(err.contains("`before`"), "{err}");
        let err = ActivityQuery::parse("").unwrap_err();
        assert!(err.contains("`after`"), "{err}");
    }

    #[test]
    fn timestamps_must_be_finite_and_non_negative() {
        for value in ["abc", "inf", "NaN", "-5"] {
            let err = ActivityQuery::parse(&format!("after={value}&before=200")).unwrap_err();
            assert!(err.contains("`after`"), "{value}: {err}");
            let err = ActivityQuery::parse(&format!("after=100&before={value}")).unwrap_err();
            assert!(err.contains("`before`"), "{value}: {err}");
        }
    }

    #[test]
    fn range_requires_after_below_before() {
        let err = ActivityQuery::parse("after=200&before=200").unwrap_err();
        assert!(err.contains("`before`"), "{err}");
        let err = ActivityQuery::parse("after=300&before=200").unwrap_err();
        assert!(err.contains("`before`"), "{err}");
    }

    #[test]
    fn range_is_at_most_31_days() {
        ActivityQuery::parse("after=1000000&before=3678400").expect("exactly 31 days accepted");
        let err = ActivityQuery::parse("after=1000000&before=3678400.001").unwrap_err();
        assert!(err.contains("`before`"), "{err}");
    }

    #[test]
    fn camera_list_is_decoded_split_and_pruned() {
        let query = ActivityQuery::parse("after=100&before=200&cameras=front%2Cback%20yard")
            .expect("valid");
        assert_eq!(
            query.cameras,
            Some(vec!["front".to_owned(), "back yard".to_owned()])
        );
        let query = ActivityQuery::parse("after=100&before=200&cameras=").expect("valid");
        assert_eq!(query.cameras, Some(vec![]));
        let query = ActivityQuery::parse("after=100&before=200&cameras=a,,b").expect("valid");
        assert_eq!(query.cameras, Some(vec!["a".to_owned(), "b".to_owned()]));
    }

    #[test]
    fn include_values_select_their_flags() {
        let query = ActivityQuery::parse("after=100&before=200&include=motion").expect("valid");
        assert!(!query.include_reviews);
        assert!(query.include_motion);
        let query = ActivityQuery::parse("after=100&before=200&include=reviews,motion,reviews")
            .expect("valid");
        assert!(query.include_reviews);
        assert!(query.include_motion);
        let err = ActivityQuery::parse("after=100&before=200&include=").unwrap_err();
        assert!(err.contains("`include`"), "{err}");
        let err = ActivityQuery::parse("after=100&before=200&include=reviews,alerts").unwrap_err();
        assert!(err.contains("`include`"), "{err}");
    }

    #[test]
    fn malformed_segments_are_named() {
        let err = ActivityQuery::parse("after=100&before=200&limit=10").unwrap_err();
        assert!(err.contains("`limit`"), "{err}");
        let err = ActivityQuery::parse("after=100&after=150&before=200").unwrap_err();
        assert!(err.contains("`after`"), "{err}");
        let err = ActivityQuery::parse("after=100&before=200&oops").unwrap_err();
        assert!(err.contains("`oops`"), "{err}");
        let err = ActivityQuery::parse("after=100&before=200&").unwrap_err();
        assert!(err.contains("empty query parameter"), "{err}");
    }

    #[test]
    fn percent_decoding_needs_two_hex_digits() {
        assert_eq!(
            percent_decode("front%2cback%20yard").expect("valid"),
            "front,back yard"
        );
        assert!(percent_decode("%2").is_err());
        assert!(percent_decode("%zz").is_err());
        assert_eq!(percent_decode("a+b").expect("plus kept"), "a+b");
        assert!(percent_decode("%FF").is_err());
        let query = ActivityQuery::parse("%61fter=100&before=200").expect("valid");
        assert_eq!(
            query,
            ActivityQuery {
                after: 100.0,
                before: 200.0,
                cameras: None,
                include_reviews: true,
                include_motion: true,
            }
        );
    }
}
