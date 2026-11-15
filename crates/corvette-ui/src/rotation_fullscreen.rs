//! Listens for the phone turning and takes the playing player fullscreen with it.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

use crate::rotation::{
    Button, Fullscreen, Orientation, Outcome, Player, RotationAction, RotationState, decide,
};

const PLAYER_SELECTOR: &str = "[data-rotate-fullscreen]";

struct Module {
    outcome: Outcome,
    entered: bool,
    pending: bool,
    button: Option<web_sys::Element>,
    native: Option<web_sys::HtmlVideoElement>,
}

pub(crate) fn install() {
    let state = Rc::new(RefCell::new(Module {
        outcome: Outcome::NotAttempted,
        entered: false,
        pending: false,
        button: None,
        native: None,
    }));
    listen_for_orientation(Rc::clone(&state));
    listen_for_fullscreen_exit(Rc::clone(&state));
    listen_for_native_fullscreen_exit(state);
}

fn listen_for_orientation(state: Rc<RefCell<Module>>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let listener = Closure::<dyn FnMut()>::new(move || evaluate(&state));
    let screen_orientation = window
        .screen()
        .ok()
        .and_then(|screen| js_sys::Reflect::get(screen.as_ref(), &"orientation".into()).ok())
        .filter(|value| !value.is_undefined() && !value.is_null());
    if let Some(object) = &screen_orientation
        && let Some(target) = object.dyn_ref::<web_sys::EventTarget>()
    {
        target
            .add_event_listener_with_callback("change", listener.as_ref().unchecked_ref())
            .ok();
        listener.forget();
        return;
    }
    window
        .add_event_listener_with_callback("orientationchange", listener.as_ref().unchecked_ref())
        .ok();
    // The page has no later point at which a listener is removed, so the closures it owns are leaked.
    listener.forget();
}

fn listen_for_fullscreen_exit(state: Rc<RefCell<Module>>) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let watch = document.clone();
    let listener = Closure::<dyn FnMut()>::new(move || {
        if watch.fullscreen_element().is_none() {
            state.borrow_mut().entered = false;
        }
    });
    document
        .add_event_listener_with_callback("fullscreenchange", listener.as_ref().unchecked_ref())
        .ok();
    listener.forget();
}

fn listen_for_native_fullscreen_exit(state: Rc<RefCell<Module>>) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let listener = Closure::<dyn FnMut()>::new(move || {
        let mut module = state.borrow_mut();
        module.entered = false;
        module.native = None;
    });
    // Media events do not bubble, so a document-wide listener must capture.
    document
        .add_event_listener_with_callback_and_bool(
            "webkitendfullscreen",
            listener.as_ref().unchecked_ref(),
            true,
        )
        .ok();
    listener.forget();
}

fn orientation_from_type(text: &str) -> Orientation {
    if text.starts_with("landscape") {
        Orientation::Landscape
    } else {
        Orientation::Portrait
    }
}

fn orientation_from_degrees(degrees: f64) -> Orientation {
    let sideways = (degrees.abs() - 90.0).abs() <= 1.0;
    if sideways {
        Orientation::Landscape
    } else {
        Orientation::Portrait
    }
}

fn current_orientation() -> Orientation {
    let Some(window) = web_sys::window() else {
        return Orientation::Portrait;
    };
    if let Ok(screen) = window.screen()
        && let Ok(orientation) = js_sys::Reflect::get(screen.as_ref(), &"orientation".into())
        && let Ok(kind) = js_sys::Reflect::get(&orientation, &"type".into())
        && let Some(text) = kind.as_string()
    {
        return orientation_from_type(&text);
    }
    if let Ok(degrees) = js_sys::Reflect::get(window.as_ref(), &"orientation".into())
        && let Some(number) = degrees.as_f64()
    {
        return orientation_from_degrees(number);
    }
    Orientation::Portrait
}

fn native_video(player: &web_sys::Element) -> Option<web_sys::HtmlVideoElement> {
    let video = player
        .dyn_ref::<web_sys::HtmlVideoElement>()
        .cloned()
        .or_else(|| {
            player
                .query_selector("video")
                .ok()
                .flatten()?
                .dyn_into::<web_sys::HtmlVideoElement>()
                .ok()
        })?;
    js_sys::Reflect::get(video.as_ref(), &"webkitEnterFullscreen".into())
        .is_ok_and(|member| member.is_function())
        .then_some(video)
}

fn fullscreen_capability(
    entered: bool,
    player: Option<&web_sys::Element>,
    native: Option<&web_sys::HtmlVideoElement>,
) -> Fullscreen {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return Fullscreen::Unavailable;
    };
    let requestable = document.document_element().is_some_and(|element| {
        js_sys::Reflect::get(element.as_ref(), &"requestFullscreen".into())
            .is_ok_and(|value| value.is_function())
    });
    if !document.fullscreen_enabled() || !requestable {
        if entered
            && let Some(video) = native
            && js_sys::Reflect::get(video.as_ref(), &"webkitDisplayingFullscreen".into())
                .is_ok_and(|value| value.as_bool() == Some(true))
        {
            return Fullscreen::Entered;
        }
        return if player.is_some_and(|element| native_video(element).is_some()) {
            Fullscreen::Available
        } else {
            Fullscreen::Unavailable
        };
    }
    if entered && document.fullscreen_element().is_some() {
        Fullscreen::Entered
    } else {
        Fullscreen::Available
    }
}

fn is_live_video(element: &web_sys::Element) -> bool {
    element
        .dyn_ref::<web_sys::HtmlVideoElement>()
        .is_some_and(|video| !video.paused() && !video.ended())
}

fn is_playing(element: &web_sys::Element) -> bool {
    if is_live_video(element) {
        return true;
    }
    if element
        .query_selector("video")
        .is_ok_and(|found| found.is_some_and(|video| is_live_video(&video)))
    {
        return true;
    }
    // The live player draws into a canvas and has no paused state.
    element
        .query_selector("moq-watch")
        .is_ok_and(|watch| watch.is_some())
}

fn is_visible(element: &web_sys::Element) -> bool {
    let bounds = element.get_bounding_client_rect();
    bounds.width() > 0.0 && bounds.height() > 0.0
}

fn eligible_player() -> Option<web_sys::Element> {
    let document = web_sys::window().and_then(|window| window.document())?;
    let Ok(players) = document.query_selector_all(PLAYER_SELECTOR) else {
        return None;
    };
    (0..players.length())
        .filter_map(|index| players.item(index))
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .find(|element| is_visible(element) && is_playing(element))
}

fn evaluate(state: &Rc<RefCell<Module>>) {
    let orientation = current_orientation();
    let player = eligible_player();
    let (outcome, entered, button_shown, native) = {
        let module = state.borrow();
        (
            module.outcome,
            module.entered,
            module.button.is_some(),
            module.native.clone(),
        )
    };
    let action = decide(RotationState {
        orientation,
        player: if player.is_some() {
            Player::Playing
        } else {
            Player::Idle
        },
        fullscreen: fullscreen_capability(entered, player.as_ref(), native.as_ref()),
        outcome,
        button: if button_shown {
            Button::Shown
        } else {
            Button::Hidden
        },
    });
    match action {
        RotationAction::RequestFullscreen => {
            if let Some(target) = player
                && !state.borrow().pending
            {
                request(state, &target);
            }
        }
        RotationAction::ExitFullscreen => {
            let video = state.borrow_mut().native.take();
            match video {
                Some(video) => {
                    if let Ok(method) =
                        js_sys::Reflect::get(video.as_ref(), &"webkitExitFullscreen".into())
                        && let Some(function) = method.dyn_ref::<js_sys::Function>()
                    {
                        function.call0(video.as_ref()).ok();
                    }
                }
                None => {
                    if let Some(document) = web_sys::window().and_then(|window| window.document()) {
                        document.exit_fullscreen();
                    }
                }
            }
            let mut module = state.borrow_mut();
            module.entered = false;
            remove_button(&mut module);
        }
        RotationAction::ShowButton => show_button(state),
        RotationAction::HideButton => remove_button(&mut state.borrow_mut()),
        RotationAction::Nothing => {}
    }
    if orientation == Orientation::Portrait {
        state.borrow_mut().outcome = Outcome::NotAttempted;
    }
}

fn reject(state: &Rc<RefCell<Module>>) {
    {
        let mut module = state.borrow_mut();
        module.outcome = Outcome::Rejected;
        module.pending = false;
    }
    evaluate(state);
}

fn request(state: &Rc<RefCell<Module>>, target: &web_sys::Element) {
    state.borrow_mut().pending = true;
    let route_available = web_sys::window()
        .and_then(|window| window.document())
        .is_some_and(|document| {
            document.fullscreen_enabled()
                && document.document_element().is_some_and(|element| {
                    js_sys::Reflect::get(element.as_ref(), &"requestFullscreen".into())
                        .is_ok_and(|value| value.is_function())
                })
        });
    if !route_available {
        request_native(state, target);
        return;
    }
    let Ok(method) = js_sys::Reflect::get(target.as_ref(), &"requestFullscreen".into()) else {
        reject(state);
        return;
    };
    let Some(function) = method.dyn_ref::<js_sys::Function>() else {
        reject(state);
        return;
    };
    let Ok(returned) = function.call0(target.as_ref()) else {
        reject(state);
        return;
    };
    let on_fulfilled = {
        let state = Rc::clone(state);
        Closure::<dyn FnMut(JsValue)>::new(move |_value: JsValue| {
            {
                let mut module = state.borrow_mut();
                module.outcome = Outcome::Resolved;
                module.entered = true;
                module.pending = false;
            }
            evaluate(&state);
        })
    };
    let on_rejected = {
        let state = Rc::clone(state);
        Closure::<dyn FnMut(JsValue)>::new(move |_reason: JsValue| reject(&state))
    };
    let promise = js_sys::Promise::resolve(&returned);
    let _ = promise.then2(&on_fulfilled, &on_rejected);
    on_fulfilled.forget();
    on_rejected.forget();
}

fn request_native(state: &Rc<RefCell<Module>>, target: &web_sys::Element) {
    let Some(video) = native_video(target) else {
        reject(state);
        return;
    };
    let Ok(method) = js_sys::Reflect::get(video.as_ref(), &"webkitEnterFullscreen".into()) else {
        reject(state);
        return;
    };
    let Some(function) = method.dyn_ref::<js_sys::Function>() else {
        reject(state);
        return;
    };
    // The native request call is synchronous and returns no promise to await.
    if function.call0(video.as_ref()).is_err() {
        reject(state);
        return;
    }
    {
        let mut module = state.borrow_mut();
        module.outcome = Outcome::Resolved;
        module.entered = true;
        module.pending = false;
        module.native = Some(video);
    }
    evaluate(state);
}

fn show_button(state: &Rc<RefCell<Module>>) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(body) = document.body() else { return };
    let Ok(button) = document.create_element("button") else {
        return;
    };
    if button
        .set_attribute("class", "rotation-fullscreen-button")
        .is_err()
        || button.set_attribute("type", "button").is_err()
    {
        return;
    }
    button.set_text_content(Some("Full screen"));
    let click_state = Rc::clone(state);
    let click = Closure::<dyn FnMut()>::new(move || {
        if let Some(target) = eligible_player() {
            request(&click_state, &target);
        }
    });
    button
        .add_event_listener_with_callback("click", click.as_ref().unchecked_ref())
        .ok();
    click.forget();
    if body.append_child(button.as_ref()).is_err() {
        return;
    }
    state.borrow_mut().button = Some(button);
}

fn remove_button(module: &mut Module) {
    let Some(button) = module.button.take() else {
        return;
    };
    if let Some(parent) = button.parent_node() {
        parent.remove_child(button.as_ref()).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::{Orientation, orientation_from_degrees, orientation_from_type};

    #[test]
    fn both_landscape_type_names_come_out_landscape() {
        assert_eq!(
            orientation_from_type("landscape-primary"),
            Orientation::Landscape
        );
        assert_eq!(
            orientation_from_type("landscape-secondary"),
            Orientation::Landscape
        );
    }

    #[test]
    fn every_other_type_name_comes_out_portrait() {
        for text in ["portrait-primary", "portrait-secondary", "", "unknown"] {
            assert_eq!(orientation_from_type(text), Orientation::Portrait);
        }
    }

    #[test]
    fn sideways_degrees_come_out_landscape() {
        assert_eq!(orientation_from_degrees(90.0), Orientation::Landscape);
        assert_eq!(orientation_from_degrees(-90.0), Orientation::Landscape);
    }

    #[test]
    fn upright_and_oblique_degrees_come_out_portrait() {
        for degrees in [0.0, 180.0, 45.0] {
            assert_eq!(orientation_from_degrees(degrees), Orientation::Portrait);
        }
    }
}
