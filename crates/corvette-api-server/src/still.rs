//! Path and query parsing and the backing fetch for `GET /corvette/api/v1/cameras/<name>/still.jpg`.

use crate::config::get_body;

pub const DEFAULT_HEIGHT: u32 = 270;

pub const MAX_HEIGHT: u32 = 576;

// The largest picture measured on the current backing is 122138 bytes.
pub const MAX_IMAGE_BYTES: usize = 1024 * 1024;

pub(crate) fn camera_segment(path: &str) -> Option<&str> {
    let prefix = format!("{}cameras/", crate::API_PREFIX);
    let rest = path.strip_prefix(prefix.as_str())?;
    let segment = rest.strip_suffix("/still.jpg")?;
    (!segment.is_empty() && !segment.contains('/')).then_some(segment)
}

pub(crate) fn parse_height(query: &str) -> Result<u32, String> {
    let params = crate::activity::parse_parameters(query, &["height"])?;
    let Some(value) = params.get("height") else {
        return Ok(DEFAULT_HEIGHT);
    };
    let whole = value
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| value.parse::<u32>().ok())
        .flatten();
    match whole {
        Some(height) if (1..=MAX_HEIGHT).contains(&height) => Ok(height),
        _ => Err(format!(
            "`height` must be a whole number from 1 to {MAX_HEIGHT}"
        )),
    }
}

pub(crate) async fn fetch_still(addr: &str, camera: &str, height: u32) -> Result<Vec<u8>, String> {
    let path = format!(
        "/api/{}/latest.jpg?height={height}",
        crate::activity::percent_encode_segment(camera)
    );
    let body = get_body(addr, &path, MAX_IMAGE_BYTES, "still")
        .await
        .map_err(|err| err.to_string())?;
    if body.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Ok(body)
    } else {
        Err("backing answered 200 with a body that is not a JPEG".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_segment_returns_the_raw_segment_of_a_still_path() {
        assert_eq!(
            camera_segment("/corvette/api/v1/cameras/front/still.jpg"),
            Some("front")
        );
        assert_eq!(
            camera_segment("/corvette/api/v1/cameras/back%20yard/still.jpg"),
            Some("back%20yard")
        );
    }

    #[test]
    fn camera_segment_refuses_every_path_that_is_not_one_still_address() {
        for path in [
            "/corvette/api/v1/cameras//still.jpg",
            "/corvette/api/v1/cameras/a/b/still.jpg",
            "/corvette/api/v1/cameras/front/still.png",
            "/corvette/api/v1/cameras/front",
            "/corvette/api/v1/cameras/still.jpg",
            "/api/v1/cameras/front/still.jpg",
            "/corvette/api/v2/cameras/front/still.jpg",
        ] {
            assert_eq!(camera_segment(path), None, "path {path:?}");
        }
    }

    #[test]
    fn parse_height_defaults_without_a_height_and_accepts_the_ends_of_its_range() {
        assert_eq!(parse_height(""), Ok(DEFAULT_HEIGHT));
        assert_eq!(parse_height("height=1"), Ok(1));
        assert_eq!(parse_height("height=576"), Ok(576));
    }

    #[test]
    fn parse_height_refuses_anything_but_a_plain_whole_number_in_range() {
        for query in [
            "height=0",
            "height=577",
            "height=abc",
            "height=",
            "height=+5",
            "height=-1",
            "height=1.5",
            "height=99999999999999999999",
        ] {
            assert_eq!(
                parse_height(query),
                Err("`height` must be a whole number from 1 to 576".to_owned()),
                "query {query:?}"
            );
        }
    }

    #[test]
    fn parse_height_refuses_a_repeated_or_unknown_parameter() {
        assert!(parse_height("height=1&height=2").is_err());
        assert!(parse_height("width=1").is_err());
    }
}
