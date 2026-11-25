//! Head-probed download offers for a selected range; answer shapes: `docs/design/api-contracts.md`.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;

use gloo_net::http::Request;
use leptos::prelude::*;
use leptos::reactive::owner::LocalStorage;

use crate::media::RecordingRange;

const NEXT_START_HEADER: &str = "X-Corvette-Next-Start";
const MAX_OFFERED_PARTS: usize = 8; // the ceiling stops one selection from chaining forever

pub(crate) enum ProbeOutcome {
    Offered { next_start: Option<String> },
    NotOffered,
}

pub(crate) fn percent_encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}"); // writing to a String cannot fail
            }
        }
    }
    encoded
}

pub(crate) fn whole_seconds(unix_seconds: f64) -> String {
    format!("{}", unix_seconds.floor()) // same flooring the server applies to its own filename
}

pub(crate) fn download_url(camera: &str, start: &str, end: &str) -> String {
    format!(
        "/corvette/api/v1/media/{}/start/{}/end/{}/download.mp4",
        percent_encode_component(camera),
        percent_encode_component(start),
        percent_encode_component(end),
    )
}

fn media_type_of(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .map_or_else(String::new, |base| base.trim().to_lowercase())
}

pub(crate) fn probe_offers(status: u16, content_type: Option<&str>) -> bool {
    // a request landing without the route prefix answers 200 with the page itself
    status == 200 && content_type.is_some_and(|value| media_type_of(value) == "application/mp4")
}

pub(crate) fn next_start_text(header: Option<&str>) -> Option<String> {
    header
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned) // decimal text kept exactly as sent, never re-parsed
}

pub(crate) fn next_part_start(next_start: Option<&str>, parts_offered: usize) -> Option<String> {
    if parts_offered >= MAX_OFFERED_PARTS {
        return None;
    }
    next_start_text(next_start)
}

fn head_outcome(status: u16, content_type: Option<&str>, next_start: Option<&str>) -> ProbeOutcome {
    if !probe_offers(status, content_type) {
        return ProbeOutcome::NotOffered;
    }
    ProbeOutcome::Offered {
        next_start: next_start_text(next_start),
    }
}

#[derive(Clone, Default)]
pub(crate) struct DownloadSelection {
    urls: RwSignal<Vec<String>>,
    runs: Rc<RefCell<ProbeRuns>>,
}

#[derive(Default)]
struct ProbeRuns {
    generation: u64,
    active: Option<web_sys::AbortController>,
}

impl DownloadSelection {
    // Repoints the offer at a new selection: stops the previous chain, clears the list,
    // then probes the new range part by part until a part is not offered, carries no next
    // start, or the ceiling is reached; answers older than the newest generation are
    // discarded, so only the newest chain writes the list. `None` leaves nothing offered.
    pub(crate) fn restart(&self, selection: Option<RecordingRange>) {
        self.cancel();
        self.urls.set(Vec::new());
        let Some(selection) = selection else {
            return;
        };
        let Ok(controller) = web_sys::AbortController::new() else {
            return; // the constructor has no failure path in browsers; answering one offers nothing
        };
        {
            let mut runs = self.runs.borrow_mut();
            runs.generation += 1;
            runs.active = Some(controller.clone());
        }
        let runs = Rc::clone(&self.runs);
        let generation = runs.borrow().generation;
        let signal = controller.signal();
        let urls = self.urls;
        leptos::task::spawn_local(async move {
            // a named async fn here would trip the not-Send nursery lint on host targets
            let probe = |url: String| {
                let signal = signal.clone();
                async move {
                    let Ok(response) = Request::get(&url).abort_signal(Some(&signal)).send().await
                    else {
                        return ProbeOutcome::NotOffered; // an aborted chain is stale by definition
                    };
                    let status = response.status();
                    let content_type = response.headers().get("content-type");
                    let next_start = response.headers().get(NEXT_START_HEADER);
                    if let Some(body) = response.body() {
                        let _ = body.cancel(); // the promise cannot change the offer; the server stops on the closed read
                    }
                    head_outcome(status, content_type.as_deref(), next_start.as_deref())
                }
            };
            let mut start = whole_seconds(selection.start_time);
            let end = whole_seconds(selection.end_time);
            let mut offered: Vec<String> = Vec::new();
            loop {
                let url = download_url(&selection.camera, &start, &end);
                let ProbeOutcome::Offered { next_start } = probe(url.clone()).await else {
                    return;
                };
                if runs.borrow().generation != generation {
                    return; // the selection moved on; the newer chain owns the list
                }
                offered.push(url);
                urls.set(offered.clone());
                let Some(next) = next_part_start(next_start.as_deref(), offered.len()) else {
                    return;
                };
                start = next;
            }
        });
    }

    // Stops the chain in flight; an aborted chain can no longer write anything.
    pub(crate) fn cancel(&self) {
        if let Some(controller) = self.runs.borrow_mut().active.take() {
            controller.abort();
        }
    }
}

#[component]
pub(crate) fn DownloadOffers(
    selection: StoredValue<DownloadSelection, LocalStorage>,
) -> impl IntoView {
    move || {
        let urls = selection.get_value().urls.get();
        if urls.is_empty() {
            return ().into_any();
        }
        let links = urls
            .iter()
            .enumerate()
            .map(|(index, url)| {
                let label = if index == 0 {
                    "Download video".to_owned()
                } else {
                    format!("Download part {}", index + 1)
                };
                let href = url.clone();
                view! { <a class="download-offer" href=href download="">{label}</a> }
            })
            .collect_view();
        let note = (urls.len() > 1).then(|| {
            view! {
                <p class="download-note">
                    "The recording changed camera settings partway, so it comes in parts."
                </p>
            }
        });
        view! {
            <div class="download-offers" role="group" aria-label="Download the recording">
                {links}
                {note}
            </div>
        }
        .into_any()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_url_encodes_camera_and_uses_integer_seconds() {
        assert_eq!(
            download_url("front yard", &whole_seconds(100.0), &whole_seconds(200.9)),
            "/corvette/api/v1/media/front%20yard/start/100/end/200/download.mp4"
        );
        assert_eq!(
            download_url("a/b", "1", "2"),
            "/corvette/api/v1/media/a%2Fb/start/1/end/2/download.mp4"
        );
        assert_eq!(whole_seconds(100.9), "100");
        assert_eq!(whole_seconds(1_700_000_000.999), "1700000000");
    }

    #[test]
    fn next_start_text_keeps_decimal_text_exactly() {
        assert_eq!(
            next_start_text(Some("1008.500")).as_deref(),
            Some("1008.500")
        );
        assert_eq!(next_start_text(None), None);
        assert_eq!(next_start_text(Some("")), None);
        assert_eq!(next_start_text(Some("   ")), None);
    }

    #[test]
    fn only_a_two_hundred_mp4_head_offers() {
        let cases = [
            (200, Some("application/mp4"), true),
            (200, Some("application/mp4; codecs=\"avc1\""), true),
            (200, Some("Application/MP4"), true),
            (200, Some("text/html"), false),
            (200, None, false),
            (404, Some("application/mp4"), false),
            (400, Some("application/mp4"), false),
            (503, Some("application/mp4"), false),
            (404, None, false),
        ];
        for (status, content_type, offered) in cases {
            assert_eq!(
                probe_offers(status, content_type),
                offered,
                "status {status} with {content_type:?}"
            );
        }
    }

    #[test]
    fn the_part_chain_stops_at_the_ceiling() {
        assert_eq!(
            next_part_start(Some("1008.500"), 0).as_deref(),
            Some("1008.500")
        );
        assert_eq!(
            next_part_start(Some("1008.500"), 7).as_deref(),
            Some("1008.500")
        );
        assert_eq!(next_part_start(Some("1008.500"), 8), None);
        assert_eq!(next_part_start(None, 0), None);
        assert_eq!(next_part_start(Some(""), 0), None);
    }

    #[test]
    fn percent_encoder_passes_the_unreserved_set() {
        assert_eq!(percent_encode_component("A-Za-z0-9-_.~"), "A-Za-z0-9-_.~");
        assert_eq!(percent_encode_component("ä"), "%C3%A4");
    }
}
