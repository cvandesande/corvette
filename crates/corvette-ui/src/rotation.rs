//! The action the page should take with a player when the phone turns sideways.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Orientation {
    Landscape,
    Portrait,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Player {
    Idle,
    Playing,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fullscreen {
    Unavailable,
    Available,
    Entered,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    NotAttempted,
    Resolved,
    Rejected,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Button {
    Hidden,
    Shown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RotationState {
    pub(crate) orientation: Orientation,
    pub(crate) player: Player,
    pub(crate) fullscreen: Fullscreen,
    pub(crate) outcome: Outcome,
    pub(crate) button: Button,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RotationAction {
    RequestFullscreen,
    ExitFullscreen,
    ShowButton,
    HideButton,
    Nothing,
}

/// The one action the page should take for this state.
pub(crate) fn decide(state: RotationState) -> RotationAction {
    let hide_when_shown = || match state.button {
        Button::Shown => RotationAction::HideButton,
        Button::Hidden => RotationAction::Nothing,
    };
    if state.fullscreen == Fullscreen::Unavailable {
        return RotationAction::Nothing;
    }
    if state.orientation == Orientation::Portrait && state.fullscreen == Fullscreen::Entered {
        return RotationAction::ExitFullscreen;
    }
    if state.orientation == Orientation::Portrait || state.player == Player::Idle {
        return hide_when_shown();
    }
    match state.outcome {
        Outcome::NotAttempted => match state.fullscreen {
            Fullscreen::Available => RotationAction::RequestFullscreen,
            Fullscreen::Entered => hide_when_shown(),
            Fullscreen::Unavailable => RotationAction::Nothing,
        },
        Outcome::Resolved => hide_when_shown(),
        Outcome::Rejected => match state.button {
            Button::Shown => RotationAction::Nothing,
            Button::Hidden => RotationAction::ShowButton,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_ORIENTATIONS: [Orientation; 2] = [Orientation::Landscape, Orientation::Portrait];
    const ALL_PLAYERS: [Player; 2] = [Player::Idle, Player::Playing];
    const ALL_FULLSCREENS: [Fullscreen; 3] = [
        Fullscreen::Unavailable,
        Fullscreen::Available,
        Fullscreen::Entered,
    ];
    const ALL_OUTCOMES: [Outcome; 3] =
        [Outcome::NotAttempted, Outcome::Resolved, Outcome::Rejected];
    const ALL_BUTTONS: [Button; 2] = [Button::Hidden, Button::Shown];

    const PLAYING: RotationState = RotationState {
        orientation: Orientation::Landscape,
        player: Player::Playing,
        fullscreen: Fullscreen::Available,
        outcome: Outcome::NotAttempted,
        button: Button::Hidden,
    };

    #[test]
    fn unavailable_fullscreen_decides_nothing() {
        let mut portrait = PLAYING;
        portrait.orientation = Orientation::Portrait;
        portrait.fullscreen = Fullscreen::Unavailable;
        assert_eq!(decide(portrait), RotationAction::Nothing);
        let mut rejected = PLAYING;
        rejected.fullscreen = Fullscreen::Unavailable;
        rejected.outcome = Outcome::Rejected;
        assert_eq!(decide(rejected), RotationAction::Nothing);
        let mut idle_shown = PLAYING;
        idle_shown.fullscreen = Fullscreen::Unavailable;
        idle_shown.player = Player::Idle;
        idle_shown.button = Button::Shown;
        assert_eq!(decide(idle_shown), RotationAction::Nothing);
    }

    #[test]
    fn portrait_exits_entered_fullscreen_and_clears_the_button() {
        let mut entered = PLAYING;
        entered.orientation = Orientation::Portrait;
        entered.fullscreen = Fullscreen::Entered;
        assert_eq!(decide(entered), RotationAction::ExitFullscreen);
        let mut shown = PLAYING;
        shown.orientation = Orientation::Portrait;
        shown.button = Button::Shown;
        assert_eq!(decide(shown), RotationAction::HideButton);
        let mut plain = PLAYING;
        plain.orientation = Orientation::Portrait;
        assert_eq!(decide(plain), RotationAction::Nothing);
    }

    #[test]
    fn landscape_without_a_playing_player_hides_the_button() {
        let mut shown = PLAYING;
        shown.player = Player::Idle;
        shown.button = Button::Shown;
        assert_eq!(decide(shown), RotationAction::HideButton);
        let mut plain = PLAYING;
        plain.player = Player::Idle;
        assert_eq!(decide(plain), RotationAction::Nothing);
    }

    #[test]
    fn landscape_playing_follows_the_attempt_outcome() {
        assert_eq!(decide(PLAYING), RotationAction::RequestFullscreen);
        let mut entered = PLAYING;
        entered.fullscreen = Fullscreen::Entered;
        assert_eq!(decide(entered), RotationAction::Nothing);
        let mut entered_shown = PLAYING;
        entered_shown.fullscreen = Fullscreen::Entered;
        entered_shown.button = Button::Shown;
        assert_eq!(decide(entered_shown), RotationAction::HideButton);
        let mut resolved_shown = PLAYING;
        resolved_shown.outcome = Outcome::Resolved;
        resolved_shown.button = Button::Shown;
        assert_eq!(decide(resolved_shown), RotationAction::HideButton);
        let mut resolved = PLAYING;
        resolved.outcome = Outcome::Resolved;
        assert_eq!(decide(resolved), RotationAction::Nothing);
        let mut rejected = PLAYING;
        rejected.outcome = Outcome::Rejected;
        assert_eq!(decide(rejected), RotationAction::ShowButton);
        let mut rejected_shown = PLAYING;
        rejected_shown.outcome = Outcome::Rejected;
        rejected_shown.button = Button::Shown;
        assert_eq!(decide(rejected_shown), RotationAction::Nothing);
    }

    fn every_state() -> Vec<RotationState> {
        let mut states = Vec::new();
        for &orientation in &ALL_ORIENTATIONS {
            for &player in &ALL_PLAYERS {
                for &fullscreen in &ALL_FULLSCREENS {
                    for &outcome in &ALL_OUTCOMES {
                        for &button in &ALL_BUTTONS {
                            states.push(RotationState {
                                orientation,
                                player,
                                fullscreen,
                                outcome,
                                button,
                            });
                        }
                    }
                }
            }
        }
        states
    }

    #[test]
    fn rejected_never_requests_fullscreen() {
        for state in every_state() {
            if state.outcome == Outcome::Rejected {
                assert_ne!(decide(state), RotationAction::RequestFullscreen);
            }
        }
    }

    #[test]
    fn fullscreen_exits_only_after_being_entered_in_portrait() {
        for state in every_state() {
            if decide(state) == RotationAction::ExitFullscreen {
                assert_eq!(state.orientation, Orientation::Portrait);
                assert_eq!(state.fullscreen, Fullscreen::Entered);
            }
        }
    }

    #[test]
    fn nothing_happens_whenever_fullscreen_is_unavailable() {
        for state in every_state() {
            if state.fullscreen == Fullscreen::Unavailable {
                assert_eq!(decide(state), RotationAction::Nothing);
            }
        }
    }

    #[test]
    fn fullscreen_is_requested_only_on_a_first_attempt() {
        for state in every_state() {
            if decide(state) == RotationAction::RequestFullscreen {
                assert_eq!(state.orientation, Orientation::Landscape);
                assert_eq!(state.player, Player::Playing);
                assert_eq!(state.fullscreen, Fullscreen::Available);
                assert_eq!(state.outcome, Outcome::NotAttempted);
            }
        }
    }

    #[test]
    fn the_button_shows_only_for_a_rejected_landscape_request() {
        for state in every_state() {
            if decide(state) == RotationAction::ShowButton {
                assert_eq!(state.orientation, Orientation::Landscape);
                assert_eq!(state.player, Player::Playing);
                assert_eq!(state.outcome, Outcome::Rejected);
                assert_eq!(state.button, Button::Hidden);
                assert_ne!(state.fullscreen, Fullscreen::Unavailable);
            }
        }
    }

    #[test]
    fn the_button_hides_only_while_shown() {
        for state in every_state() {
            if decide(state) == RotationAction::HideButton {
                assert_eq!(state.button, Button::Shown);
            }
        }
    }
}
