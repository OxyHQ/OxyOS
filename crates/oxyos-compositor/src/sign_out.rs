//! Signing out from Settings (docs/adr/0020-oxy-account-login.mdx, decision 5): the session's
//! vocabulary's `{"signOut": {}}` (`SystemRequest`, read strictly), which ends the session as the
//! keymap's end of the session (Ctrl+Alt+Backspace) does, back to the login screen.
//!
//! - **Who may.** A client with the `settings` grant (Settings, a system application of the image's
//!   policy), citing an input it received lately: the person pressed Sign Out. Any other client is
//!   refused `notPermitted`, one citing no input `needsInput`.
//! - **Never at the login screen**: its session is no one's, so nothing there signs out
//!   (`refused`, `notPermitted`).
//! - **Answered first.** `{"signingOut": {}}` is told before the program ends the session, so
//!   Settings knows its press was taken.

use oxyos_compositor_core::scene::SystemRequest;
use oxyos_compositor_core::Scene;
use serde::Deserialize;
use serde_json::json;

/// The request, read strictly: `{"signOut": {}}` and nothing else.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum Ask {
    SignOut {},
}

/// Whether a request's text is this part's: an object whose one key is `signOut`, whatever its
/// value (one that is not `{}` is refused here, not left for another part).
fn ours(text: &str) -> bool {
    serde_json::from_str::<std::collections::HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.contains_key("signOut"))
}

/// A turn: answers the scene's sign-out requests. True when one was taken, and the program then
/// ends the session; `greeter` (the login screen's compositor) takes none.
pub fn serve(scene: &mut Scene, greeter: bool) -> bool {
    let mut signing_out = false;
    for request in scene.take_system_requests_where(|request| ours(&request.text)) {
        let answer = answer(scene, &request, greeter);
        signing_out |= answer.is_ok();
        let text = match answer {
            Ok(()) => json!({"signingOut": {}}),
            Err(reason) => json!({"refused": {"reason": reason}}),
        };
        scene.system_text(request.client, request.serial, &text.to_string());
    }
    signing_out
}

fn answer(scene: &Scene, request: &SystemRequest, greeter: bool) -> Result<(), &'static str> {
    if serde_json::from_str::<Ask>(&request.text).is_err() {
        return Err("invalidRequest");
    }
    let settings = scene
        .grants_of(request.client)
        .is_some_and(|grants| grants.settings);
    if greeter || !settings {
        return Err("notPermitted");
    }
    if !request.with_input {
        return Err("needsInput");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_compositor_core::{ClientId, Grants};
    use oxyos_protocol::{Event, PreeditText, Request};

    const SETTINGS: ClientId = 1;
    const APP: ClientId = 2;
    const SHELL: ClientId = 3;

    fn scene() -> Scene {
        let mut scene = Scene::new();
        for (client, grants) in [
            (
                SETTINGS,
                Grants {
                    settings: true,
                    ..Grants::default()
                },
            ),
            // An application that asks the session for something else (the store).
            (
                APP,
                Grants {
                    store: true,
                    ..Grants::default()
                },
            ),
            (
                SHELL,
                Grants {
                    shell: true,
                    ..Grants::default()
                },
            ),
        ] {
            scene.connect(client, grants).unwrap();
        }
        scene
    }

    fn request(client: ClientId, text: &str, with_input: bool) -> SystemRequest {
        SystemRequest {
            client,
            serial: 7,
            with_input,
            text: text.into(),
        }
    }

    #[test]
    fn settings_signs_out_for_the_persons_press_and_nobody_else_does() {
        let s = scene();
        let asked = r#"{"signOut":{}}"#;
        assert_eq!(answer(&s, &request(SETTINGS, asked, true), false), Ok(()));
        assert_eq!(
            answer(&s, &request(SETTINGS, asked, false), false),
            Err("needsInput")
        );
        for client in [APP, SHELL] {
            assert_eq!(
                answer(&s, &request(client, asked, true), false),
                Err("notPermitted")
            );
        }
        // The login screen's session is no one's.
        assert_eq!(
            answer(&s, &request(SETTINGS, asked, true), true),
            Err("notPermitted")
        );
        let longer = r#"{"signOut":{"now":true}}"#;
        assert_eq!(
            answer(&s, &request(SETTINGS, longer, true), false),
            Err("invalidRequest")
        );
        assert!(ours(asked) && ours(r#"{"signOut":1}"#));
        assert!(!ours(r#"{"privacy":{}}"#) && !ours(r#"{"signOut":{},"x":{}}"#));
    }

    #[test]
    fn a_request_is_answered_through_the_scene_and_another_parts_left_for_it() {
        let mut s = scene();
        for (serial, text) in [(1, r#"{"signOut":{}}"#), (2, r#"{"privacy":{}}"#)] {
            s.handle(
                SETTINGS,
                Request::SystemRequest {
                    serial,
                    input: 0,
                    index: 0,
                    last: true,
                    text: PreeditText::new(text).unwrap(),
                },
            )
            .unwrap();
        }
        // Citing no input the scene saw: refused, and nothing signs out.
        assert!(!serve(&mut s, false));
        let told: String = s
            .drain_events()
            .into_iter()
            .filter_map(|(to, event)| match event {
                Event::SystemText {
                    serial: 1, text, ..
                } if to == SETTINGS => Some(text.as_str().to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(told, r#"{"refused":{"reason":"needsInput"}}"#);
        assert_eq!(s.take_system_requests().len(), 1);
    }
}
