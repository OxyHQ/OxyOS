//! AutoFill for the program running the compositor (docs/adr/0050-keychain-autofill-and-passkeys.mdx,
//! decision 6). The scene says which field is offered and which item the person picked
//! (`oxyos_compositor_core::scene`'s AutoFill); this asks an [`AutofillProvider`] for that field's
//! suggestions and for the item picked, never waiting for it, and gives the scene the list it shows
//! and the text it fills. The session's provider is the accounts' service on its registrar
//! connection; the development compositor's a fixture keychain of its own.
//!
//! - **What is asked.** For a username, email or password field, the application's suggestions
//!   for that field (titles and usernames, never a password); for a new password, a strong
//!   password, shown as it will be filled; for a verification code, the logins whose code it may
//!   be. The application is the one the session started the field's process as (its launch
//!   record), never what the process says.
//! - **Filling.** Only the item picked is asked for, and only then: a login's username and
//!   password (the field's own and its paired field's, by what the field holds), a login's
//!   current code. A strong password picked is filled as it was shown.
//! - **The shell's list.** What the scene tells a shell to draw is told as the session's vocabulary
//!   (`SystemText` serial 0, `{"autofill": {...}}`, `{"autofill": null}` to hide it); the shell's
//!   press on an item is `{"chooseAutofill": {"index"}}`, citing the press.

use crate::Compositor;
use oxyos_compositor_core::scene::{AutofillItem, AutofillKind, AutofillList, SystemRequest};
use oxyos_compositor_core::Scene;
use oxyos_protocol::TextContentType;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

/// A credential, or a code: overwritten when it goes, never shown by `Debug`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(..)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

/// The field a suggestion or a fill is for, as the accounts' service names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Username,
    Email,
    Password,
    NewPassword,
    OneTimeCode,
}

impl Field {
    fn of(content: TextContentType) -> Option<Self> {
        match content {
            TextContentType::None => None,
            TextContentType::Username => Some(Self::Username),
            TextContentType::Email => Some(Self::Email),
            TextContentType::Password => Some(Self::Password),
            TextContentType::NewPassword => Some(Self::NewPassword),
            TextContentType::OneTimeCode => Some(Self::OneTimeCode),
        }
    }

    /// Its name in the accounts' service's vocabulary.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Username => "username",
            Self::Email => "email",
            Self::Password => "password",
            Self::NewPassword => "newPassword",
            Self::OneTimeCode => "oneTimeCode",
        }
    }
}

/// What the provider is asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// The suggestions of the application `desktop_entry_id` for a field.
    Suggest {
        desktop_entry_id: String,
        field: Field,
    },
    /// The item `reference` of the application's suggestions, for a field: picked by the person.
    Fill {
        desktop_entry_id: String,
        reference: String,
        field: Field,
    },
    /// A strong password.
    Generate,
}

/// One of an application's suggestions: its reference (what a fill names), its title and its
/// username. `Debug` shows neither text.
#[derive(Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub reference: String,
    pub title: String,
    pub username: String,
}

impl std::fmt::Debug for Suggestion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Suggestion(..)")
    }
}

/// The provider's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Suggestions(Vec<Suggestion>),
    Credential {
        username: Secret,
        password: Secret,
    },
    Code(Secret),
    Generated(Secret),
    /// Refused, and why (`locked`, `notPermitted`, …), or unreachable: nothing is offered or
    /// filled.
    Refused(String),
}

/// What answers AutoFill's questions: the session's keychain (the accounts' service), or a
/// fixture's. It never makes the compositor wait: `ask` begins a question and returns, and its
/// answer is taken from `answers` in a later turn (or the same one, for a provider that answers at
/// once). One whose answers come from another thread wakes the turn through a descriptor among the
/// compositor's wakers.
pub trait AutofillProvider {
    /// Begins answering `ask`, its answer to come under `id`.
    fn ask(&mut self, id: u64, ask: Ask);
    /// The answers since the last call, each under its question's id.
    fn answers(&mut self) -> Vec<(u64, Answer)>;
}

/// A provider whose questions are answered on a thread of its own, one after another, by
/// `answer` (the session's: a request to the accounts' service and its answer, which may wait):
/// the compositor's turn never waits for one, and is woken when an answer is ready
/// (`wake_fd`, among the compositor's wakers).
pub struct ThreadedProvider {
    asks: std::sync::mpsc::Sender<(u64, Ask)>,
    answered: std::sync::mpsc::Receiver<(u64, Answer)>,
    woken: std::os::unix::net::UnixStream,
}

impl ThreadedProvider {
    pub fn spawn(mut answer: impl FnMut(Ask) -> Answer + Send + 'static) -> std::io::Result<Self> {
        let (asks, asked) = std::sync::mpsc::channel::<(u64, Ask)>();
        let (answers, answered) = std::sync::mpsc::channel();
        let (woken, wake) = std::os::unix::net::UnixStream::pair()?;
        woken.set_nonblocking(true)?;
        std::thread::Builder::new()
            .name("oxyos-autofill".into())
            .spawn(move || {
                use std::io::Write;
                for (id, ask) in asked {
                    if answers.send((id, answer(ask))).is_err() {
                        return;
                    }
                    let _ = (&wake).write(&[1]);
                }
            })?;
        Ok(Self {
            asks,
            answered,
            woken,
        })
    }

    /// What wakes the compositor's turn when an answer is ready (`Compositor::wakers`).
    pub fn wake_fd(&self) -> std::io::Result<std::os::fd::OwnedFd> {
        Ok(self.woken.try_clone()?.into())
    }
}

impl AutofillProvider for ThreadedProvider {
    fn ask(&mut self, id: u64, ask: Ask) {
        // A thread gone answers nothing: the field is offered nothing.
        let _ = self.asks.send((id, ask));
    }

    fn answers(&mut self) -> Vec<(u64, Answer)> {
        use std::io::Read;
        let mut buffer = [0u8; 64];
        while matches!((&self.woken).read(&mut buffer), Ok(read) if read > 0) {}
        self.answered.try_iter().collect()
    }
}

/// What an item of the list shown fills when picked.
enum Entry {
    Login { reference: String },
    Code { reference: String },
    Strong(Secret),
}

/// The field offered last: its application, what it holds, and its items' entries.
struct Offer {
    token: u64,
    desktop_entry_id: String,
    content: TextContentType,
    entries: Vec<Entry>,
}

/// A question asked, for the field `token`.
enum Asked {
    Suggest(u64),
    Generate(u64),
    Fill(u64),
}

/// The shell's request, read strictly.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
enum ShellAsk {
    ChooseAutofill { index: usize },
}

fn ours(text: &str) -> bool {
    serde_json::from_str::<HashMap<String, serde::de::IgnoredAny>>(text)
        .is_ok_and(|asked| asked.len() == 1 && asked.contains_key("chooseAutofill"))
}

fn kind_name(kind: AutofillKind) -> &'static str {
    match kind {
        AutofillKind::Login => "login",
        AutofillKind::StrongPassword => "strongPassword",
        AutofillKind::Code => "code",
    }
}

fn content_name(content: TextContentType) -> &'static str {
    Field::of(content).map_or("none", Field::as_str)
}

/// The list as the shell is told it.
fn list_text(list: Option<&AutofillList>) -> String {
    let list = list.map(|list| {
        json!({
            "content": content_name(list.content),
            "highlighted": list.highlighted,
            "items": list.items.iter().map(|item| json!({
                "kind": kind_name(item.kind),
                "title": item.title,
                "detail": item.detail,
            })).collect::<Vec<_>>(),
        })
    });
    json!({ "autofill": list }).to_string()
}

pub struct Autofill {
    provider: Box<dyn AutofillProvider>,
    next_id: u64,
    asked: HashMap<u64, Asked>,
    offer: Option<Offer>,
}

impl Autofill {
    pub fn new(provider: Box<dyn AutofillProvider>) -> Self {
        Self {
            provider,
            next_id: 0,
            asked: HashMap::new(),
            offer: None,
        }
    }

    fn ask(&mut self, asked: Asked, ask: Ask) {
        self.next_id += 1;
        self.asked.insert(self.next_id, asked);
        self.provider.ask(self.next_id, ask);
    }

    /// A turn: the fields offered, the shell's presses, the items picked and the provider's
    /// answers, then what the shells are told; sent at once.
    pub fn serve(&mut self, compositor: &mut Compositor) {
        if self.serve_scene(&mut compositor.scene) {
            compositor.flush();
        }
    }

    /// The same on the scene alone; whether anything was told or filled.
    pub fn serve_scene(&mut self, scene: &mut Scene) -> bool {
        let mut told = false;
        for request in scene.take_system_requests_where(|request| ours(&request.text)) {
            let answer = self.choose(scene, &request);
            let text = match answer {
                Ok(()) => json!({"chosen": {}}),
                Err(reason) => json!({"refused": {"reason": reason}}),
            };
            scene.system_text(request.client, request.serial, &text.to_string());
            told = true;
        }
        // A provider that answers at once has answered the questions asked here: they are served
        // in the same turn, a fill's after its pick's.
        loop {
            let mut progress = false;
            for query in scene.take_autofill_queries() {
                progress = true;
                let Some(field) = Field::of(query.content) else {
                    continue;
                };
                self.asked
                    .retain(|_, asked| !matches!(asked, Asked::Suggest(_) | Asked::Generate(_)));
                self.offer = Some(Offer {
                    token: query.token,
                    desktop_entry_id: query.application.clone(),
                    content: query.content,
                    entries: Vec::new(),
                });
                if field == Field::NewPassword {
                    self.ask(Asked::Generate(query.token), Ask::Generate);
                } else {
                    self.ask(
                        Asked::Suggest(query.token),
                        Ask::Suggest {
                            desktop_entry_id: query.application,
                            field,
                        },
                    );
                }
            }
            for pick in scene.take_autofill_picks() {
                progress = true;
                told |= self.picked(scene, pick.token, pick.index);
            }
            for (id, answer) in self.provider.answers() {
                progress = true;
                if let Some(asked) = self.asked.remove(&id) {
                    told |= self.answered(scene, asked, answer);
                }
            }
            if !progress {
                break;
            }
        }
        for (shell, list) in scene.take_autofill_tells() {
            scene.system_text(shell, 0, &list_text(list.as_ref()));
            told = true;
        }
        told
    }

    fn choose(&mut self, scene: &mut Scene, request: &SystemRequest) -> Result<(), &'static str> {
        let Ok(ShellAsk::ChooseAutofill { index }) = serde_json::from_str(&request.text) else {
            return Err("invalidRequest");
        };
        if !request.with_input {
            return Err("needsInput");
        }
        if !scene.choose_autofill(request.client, index, request.with_input) {
            return Err("notPermitted");
        }
        Ok(())
    }

    /// The person picked `index` of the field `token`'s items: a login or a code is asked for, a
    /// strong password filled as shown.
    fn picked(&mut self, scene: &mut Scene, token: u64, index: usize) -> bool {
        let Some(offer) = self.offer.as_ref().filter(|offer| offer.token == token) else {
            return false;
        };
        let Some(field) = Field::of(offer.content) else {
            return false;
        };
        let ask = match offer.entries.get(index) {
            Some(Entry::Login { reference }) => Ask::Fill {
                desktop_entry_id: offer.desktop_entry_id.clone(),
                reference: reference.clone(),
                field,
            },
            Some(Entry::Code { reference }) => Ask::Fill {
                desktop_entry_id: offer.desktop_entry_id.clone(),
                reference: reference.clone(),
                field: Field::OneTimeCode,
            },
            Some(Entry::Strong(password)) => {
                return scene.fill_autofill(token, Some(password.as_str()), None);
            }
            None => return false,
        };
        self.ask(Asked::Fill(token), ask);
        false
    }

    fn answered(&mut self, scene: &mut Scene, asked: Asked, answer: Answer) -> bool {
        match (asked, answer) {
            (Asked::Suggest(token), Answer::Suggestions(suggestions)) => {
                let Some(offer) = self.offer.as_mut().filter(|offer| offer.token == token) else {
                    return false;
                };
                let code = offer.content == TextContentType::OneTimeCode;
                let mut items = Vec::new();
                offer.entries.clear();
                for suggestion in suggestions {
                    let reference = suggestion.reference;
                    offer.entries.push(if code {
                        Entry::Code { reference }
                    } else {
                        Entry::Login { reference }
                    });
                    items.push(AutofillItem {
                        kind: if code {
                            AutofillKind::Code
                        } else {
                            AutofillKind::Login
                        },
                        title: suggestion.title,
                        detail: suggestion.username,
                    });
                }
                scene.answer_autofill(token, items);
                false
            }
            (Asked::Generate(token), Answer::Generated(password)) => {
                let Some(offer) = self.offer.as_mut().filter(|offer| offer.token == token) else {
                    return false;
                };
                let item = AutofillItem {
                    kind: AutofillKind::StrongPassword,
                    title: String::new(),
                    detail: password.as_str().to_string(),
                };
                offer.entries = vec![Entry::Strong(password)];
                scene.answer_autofill(token, vec![item]);
                false
            }
            (Asked::Fill(token), Answer::Credential { username, password }) => {
                let Some(offer) = self.offer.as_ref().filter(|offer| offer.token == token) else {
                    return false;
                };
                let (own, paired) = match offer.content {
                    TextContentType::Password => (password, username),
                    _ => (username, password),
                };
                scene.fill_autofill(token, Some(own.as_str()), Some(paired.as_str()))
            }
            (Asked::Fill(token), Answer::Code(code)) => {
                scene.fill_autofill(token, Some(code.as_str()), None)
            }
            // Refused (locked, not permitted), unreachable, or an answer that is not this
            // question's: nothing is offered or filled.
            (Asked::Suggest(token) | Asked::Generate(token), _) => {
                scene.answer_autofill(token, Vec::new());
                false
            }
            (Asked::Fill(_), _) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_compositor_core::scene::Output;
    use oxyos_compositor_core::{ClientId, Grants, SurfaceKey};
    use oxyos_protocol::{Event, PanelEdge, PreeditText, Rect, Request, ShellKind, Size};
    use std::cell::RefCell;
    use std::rc::Rc;

    const APP: ClientId = 1;
    const SHELL: ClientId = 3;
    const RETURN: u32 = 28;
    const DOWN: u32 = 108;

    /// A provider for these tests: answers at once from what each test gives it, and keeps what
    /// it was asked.
    #[derive(Default)]
    struct Answers {
        asked: Vec<Ask>,
        answers: Vec<(u64, Answer)>,
        locked: bool,
    }

    struct TestProvider(Rc<RefCell<Answers>>);

    impl AutofillProvider for TestProvider {
        fn ask(&mut self, id: u64, ask: Ask) {
            let mut state = self.0.borrow_mut();
            state.asked.push(ask.clone());
            let answer = if state.locked {
                Answer::Refused("locked".into())
            } else {
                match ask {
                    Ask::Suggest { .. } => Answer::Suggestions(vec![
                        Suggestion {
                            reference: "item-1".into(),
                            title: "example.com".into(),
                            username: "bea@example.com".into(),
                        },
                        Suggestion {
                            reference: "item-2".into(),
                            title: "example.com".into(),
                            username: "ana@example.com".into(),
                        },
                    ]),
                    Ask::Fill {
                        reference, field, ..
                    } => match field {
                        Field::OneTimeCode => Answer::Code(Secret::new("123456")),
                        _ => Answer::Credential {
                            username: Secret::new(format!("user-of-{reference}")),
                            password: Secret::new(format!("password-of-{reference}")),
                        },
                    },
                    Ask::Generate => Answer::Generated(Secret::new("abcde-FGHIJ-12345-klmno")),
                }
            };
            state.answers.push((id, answer));
        }

        fn answers(&mut self) -> Vec<(u64, Answer)> {
            std::mem::take(&mut self.0.borrow_mut().answers)
        }
    }

    fn scene(content: TextContentType) -> (Scene, Autofill, Rc<RefCell<Answers>>) {
        let mut scene = Scene::new();
        scene
            .add_output(Output {
                id: 1,
                rect: Rect::new(0, 0, 1280, 720),
                scale_120: 120,
                refresh_mhz: 60_000,
            })
            .unwrap();
        scene.connect(APP, Grants::default()).unwrap();
        scene
            .connect(
                SHELL,
                Grants {
                    shell: true,
                    shortcuts: true,
                    ..Grants::default()
                },
            )
            .unwrap();
        scene.set_application(APP, "so.example.mail");
        for (client, request) in [
            (
                SHELL,
                Request::CreateShellSurface {
                    surface: 1,
                    kind: ShellKind::Panel,
                    edge: PanelEdge::Bottom,
                    output: 1,
                    size: Size::new(1, 48),
                    exclusive: 48,
                },
            ),
            (
                APP,
                Request::CreateToplevel {
                    surface: 1,
                    size: Size::new(400, 300),
                },
            ),
        ] {
            scene.handle(client, request).unwrap();
            present(&mut scene, SurfaceKey { client, id: 1 });
        }
        scene.activate(SurfaceKey { client: APP, id: 1 });
        for request in [
            Request::TextInputEnable { surface: 1 },
            Request::TextInputCaret {
                surface: 1,
                rect: Rect::new(10, 20, 2, 18),
            },
            Request::TextInputContentType {
                surface: 1,
                content,
                paired: true,
            },
            Request::TextInputCommit {
                surface: 1,
                serial: 1,
            },
        ] {
            scene.handle(APP, request).unwrap();
        }
        let answers = Rc::new(RefCell::new(Answers::default()));
        let autofill = Autofill::new(Box::new(TestProvider(answers.clone())));
        (scene, autofill, answers)
    }

    /// Maps `surface` with a frame of the size it was configured at, presented.
    fn present(scene: &mut Scene, surface: SurfaceKey) {
        let (serial, rect, scale) = scene
            .drain_events()
            .into_iter()
            .rev()
            .find_map(|(client, event)| match event {
                Event::Configure {
                    surface: id,
                    serial,
                    rect,
                    scale_120,
                    ..
                } if client == surface.client && id == surface.id => {
                    Some((serial, rect, scale_120))
                }
                _ => None,
            })
            .expect("configured");
        let extent = oxyos_compositor_core::physical(Size::new(rect.width, rect.height), scale);
        let buffer = u64::from(surface.id) + 100 * u64::from(surface.client);
        for request in [
            Request::AttachBuffer {
                buffer,
                format: oxyos_protocol::Format::Xrgb8888,
                extent,
                stride: extent.width * 4,
                offset: 0,
            },
            Request::Submit {
                surface: surface.id,
                sequence: 1,
                buffer,
                configure: serial,
                extent,
            },
        ] {
            scene.handle(surface.client, request).unwrap();
        }
        scene.begin_frame();
        scene.end_frame(1, oxyos_protocol::Clock::Synthetic);
    }

    /// The shell's answers and the autofill texts it was told.
    fn told_shell(scene: &mut Scene) -> Vec<serde_json::Value> {
        let mut texts: Vec<String> = Vec::new();
        let mut current = String::new();
        for (client, event) in scene.drain_events() {
            if let (SHELL, Event::SystemText { text, last, .. }) = (client, event) {
                current.push_str(text.as_str());
                if last {
                    texts.push(std::mem::take(&mut current));
                }
            }
        }
        texts
            .iter()
            .map(|text| serde_json::from_str(text).unwrap())
            .collect()
    }

    fn fills(scene: &mut Scene) -> Vec<(bool, String)> {
        let mut fills: Vec<(bool, String)> = Vec::new();
        for (client, event) in scene.drain_events() {
            if let (APP, Event::TextInputFill { paired, text, .. }) = (client, event) {
                match fills.last_mut() {
                    Some((last, so_far)) if *last == paired => so_far.push_str(text.as_str()),
                    _ => fills.push((paired, text.as_str().to_string())),
                }
            }
        }
        fills
    }

    #[test]
    fn a_threaded_provider_answers_off_the_turn_and_wakes_it() {
        let mut provider = ThreadedProvider::spawn(|ask| match ask {
            Ask::Generate => Answer::Generated(Secret::new("abcde-FGHIJ-12345-klmno")),
            _ => Answer::Refused("locked".into()),
        })
        .unwrap();
        let wake = provider.wake_fd().unwrap();
        provider.ask(1, Ask::Generate);
        provider.ask(
            2,
            Ask::Suggest {
                desktop_entry_id: "so.example.mail".into(),
                field: Field::Username,
            },
        );
        let mut answers = Vec::new();
        while answers.len() < 2 {
            let mut fds = [rustix::event::PollFd::new(
                &wake,
                rustix::event::PollFlags::IN,
            )];
            rustix::event::poll(&mut fds, None).unwrap();
            answers.extend(provider.answers());
        }
        assert_eq!(
            answers,
            [
                (1, Answer::Generated(Secret::new("abcde-FGHIJ-12345-klmno"))),
                (2, Answer::Refused("locked".into()))
            ]
        );
    }

    #[test]
    fn a_field_is_offered_its_applications_logins_and_the_pick_filled() {
        let (mut scene, mut autofill, answers) = scene(TextContentType::Username);
        autofill.serve_scene(&mut scene);
        assert_eq!(
            answers.borrow().asked,
            [Ask::Suggest {
                desktop_entry_id: "so.example.mail".into(),
                field: Field::Username,
            }]
        );
        let told = told_shell(&mut scene);
        assert_eq!(
            told.last().unwrap()["autofill"],
            json!({
                "content": "username",
                "highlighted": 0,
                "items": [
                    {"kind": "login", "title": "example.com", "detail": "bea@example.com"},
                    {"kind": "login", "title": "example.com", "detail": "ana@example.com"},
                ],
            })
        );
        // ↓ then Return: the second asked for, and filled, the username into the field and the
        // password into its pair.
        scene.keyboard(DOWN, true, None);
        scene.keyboard(DOWN, false, None);
        scene.keyboard(RETURN, true, None);
        scene.keyboard(RETURN, false, None);
        autofill.serve_scene(&mut scene);
        assert_eq!(
            answers.borrow().asked.last(),
            Some(&Ask::Fill {
                desktop_entry_id: "so.example.mail".into(),
                reference: "item-2".into(),
                field: Field::Username,
            })
        );
        let events = scene.drain_events();
        let shell: Vec<_> = events.iter().filter(|(c, _)| *c == SHELL).collect();
        assert!(!shell.is_empty());
        let fills: Vec<(bool, String)> = events
            .iter()
            .filter_map(|(client, event)| match (client, event) {
                (&APP, Event::TextInputFill { paired, text, .. }) => {
                    Some((*paired, text.as_str().to_string()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            fills,
            [
                (false, "user-of-item-2".to_string()),
                (true, "password-of-item-2".to_string())
            ]
        );
    }

    #[test]
    fn a_password_field_is_filled_with_the_password_and_its_pair_with_the_username() {
        let (mut scene, mut autofill, _) = scene(TextContentType::Password);
        autofill.serve_scene(&mut scene);
        scene.drain_events();
        scene.keyboard(RETURN, true, None);
        autofill.serve_scene(&mut scene);
        assert_eq!(
            fills(&mut scene),
            [
                (false, "password-of-item-1".to_string()),
                (true, "user-of-item-1".to_string())
            ]
        );
    }

    #[test]
    fn a_new_password_is_offered_a_strong_one_shown_and_filled_as_shown() {
        let (mut scene, mut autofill, answers) = scene(TextContentType::NewPassword);
        autofill.serve_scene(&mut scene);
        assert_eq!(answers.borrow().asked, [Ask::Generate]);
        let told = told_shell(&mut scene);
        assert_eq!(
            told.last().unwrap()["autofill"]["items"],
            json!([{"kind": "strongPassword", "title": "", "detail": "abcde-FGHIJ-12345-klmno"}])
        );
        scene.keyboard(RETURN, true, None);
        autofill.serve_scene(&mut scene);
        // Nothing more asked: the password shown is the one filled.
        assert_eq!(answers.borrow().asked.len(), 1);
        assert_eq!(
            fills(&mut scene),
            [(false, "abcde-FGHIJ-12345-klmno".to_string())]
        );
    }

    #[test]
    fn a_verification_code_is_asked_for_when_picked() {
        let (mut scene, mut autofill, answers) = scene(TextContentType::OneTimeCode);
        autofill.serve_scene(&mut scene);
        let told = told_shell(&mut scene);
        assert_eq!(told.last().unwrap()["autofill"]["items"][0]["kind"], "code");
        scene.keyboard(RETURN, true, None);
        autofill.serve_scene(&mut scene);
        assert!(matches!(
            answers.borrow().asked.last(),
            Some(Ask::Fill {
                field: Field::OneTimeCode,
                ..
            })
        ));
        assert_eq!(fills(&mut scene), [(false, "123456".to_string())]);
    }

    #[test]
    fn a_refusal_offers_nothing() {
        let (mut scene, mut autofill, answers) = scene(TextContentType::Username);
        answers.borrow_mut().locked = true;
        autofill.serve_scene(&mut scene);
        assert!(told_shell(&mut scene).is_empty());
        assert!(scene.autofill_shown().is_none());
    }

    #[test]
    fn the_shells_press_picks_citing_its_input() {
        let (mut scene, mut autofill, answers) = scene(TextContentType::Email);
        autofill.serve_scene(&mut scene);
        scene.drain_events();
        // Citing no input it received: refused, nothing asked.
        let text = r#"{"chooseAutofill":{"index":1}}"#;
        scene
            .handle(
                SHELL,
                Request::SystemRequest {
                    serial: 7,
                    input: 0,
                    index: 0,
                    last: true,
                    text: PreeditText::new(text).unwrap(),
                },
            )
            .unwrap();
        autofill.serve_scene(&mut scene);
        assert_eq!(
            told_shell(&mut scene),
            [json!({"refused": {"reason": "needsInput"}})]
        );
        assert_eq!(answers.borrow().asked.len(), 1);
        // Another client's press is not the shell's; the shell's, citing its press, picks.
        let request = |client, with_input| SystemRequest {
            client,
            serial: 8,
            with_input,
            text: text.into(),
        };
        assert_eq!(
            autofill.choose(&mut scene, &request(APP, true)),
            Err("notPermitted")
        );
        assert_eq!(autofill.choose(&mut scene, &request(SHELL, true)), Ok(()));
        autofill.serve_scene(&mut scene);
        assert!(matches!(
            answers.borrow().asked.last(),
            Some(Ask::Fill { reference, field: Field::Email, .. }) if reference == "item-2"
        ));
        assert_eq!(
            fills(&mut scene),
            [
                (false, "user-of-item-2".to_string()),
                (true, "password-of-item-2".to_string())
            ]
        );
    }
}
