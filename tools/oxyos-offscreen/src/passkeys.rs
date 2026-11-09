//! The development compositor's fixture accounts' service for passkeys' sheets
//! (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7), with `--passkey-fixture ID`: a
//! [`PasskeyPrompts`] source asking, for the application `ID` once a client of it is connected,
//! what the service asks when an application uses passkeys: to use one for `github.com` (two
//! accounts to choose from), then, once verified, to create one for `example.org`, then that
//! `example.net` has none. It checks the password itself, as the service does: a wrong one is
//! asked again, two wrong ones in a row are throttled for 30 seconds. It is a fixture of this
//! fixture compositor, never installed, never a session's: its password is made up, and nothing it
//! holds is a secret. What it was answered so far is told in each step's result (`passkeys`),
//! never the password typed.

use oxyos_compositor::autofill::Secret;
use oxyos_compositor::passkeys::{Choice, Kind, PasskeyPrompts, Prompt, Reply};

/// The fixture account's password: made up, the scenarios type it.
pub const PASSWORD: &str = "fixture-oxy-password";
/// The fixture's relying parties, in the order it asks.
const ASKS: [(&str, Kind); 3] = [
    ("github.com", Kind::Get),
    ("example.org", Kind::Create),
    ("example.net", Kind::None),
];

pub struct FixtureService {
    /// The application it asks for.
    pub application: String,
    /// Its process, once a client of it is connected (the director sets it).
    pub pid: Option<i32>,
    /// The next of `ASKS` to ask, and whether one is shown.
    next: usize,
    shown: Option<String>,
    wrong: u32,
    prompts: Vec<Prompt>,
    /// What it was answered, for a step's result: never a password.
    pub told: Vec<String>,
}

impl FixtureService {
    pub fn new(application: String) -> Self {
        Self {
            application,
            pid: None,
            next: 0,
            shown: None,
            wrong: 0,
            prompts: Vec::new(),
            told: Vec::new(),
        }
    }

    /// Asks the next of its relying parties, if none is shown and its application is there.
    fn ask_next(&mut self) {
        let (Some(pid), None) = (self.pid, self.shown.as_ref()) else {
            return;
        };
        let Some((rp_id, kind)) = ASKS.get(self.next).copied() else {
            return;
        };
        let id = format!("fixture-{}", self.next + 1);
        self.next += 1;
        self.shown = Some(id.clone());
        let choices = match kind {
            Kind::Get => vec![
                Choice {
                    user_name: "ada@example.com".into(),
                    display_name: "Ada Lovelace".into(),
                },
                Choice {
                    user_name: "bea@example.com".into(),
                    display_name: "Bea".into(),
                },
            ],
            _ => Vec::new(),
        };
        self.prompts.push(Prompt::Confirm {
            id,
            desktop_entry_id: self.application.clone(),
            pid,
            rp_id: rp_id.into(),
            user_name: (kind == Kind::Create).then(|| Secret::new("ada@example.com")),
            kind,
            choices,
        });
    }

    fn done(&mut self, id: String) {
        self.prompts.push(Prompt::Done { id });
        self.shown = None;
        self.wrong = 0;
    }
}

/// A shared handle the director keeps to set the application's process and read what was told,
/// while the compositor's passkeys part owns the source.
pub struct Shared(pub std::rc::Rc<std::cell::RefCell<FixtureService>>);

impl PasskeyPrompts for Shared {
    fn take(&mut self) -> Vec<Prompt> {
        let mut service = self.0.borrow_mut();
        service.ask_next();
        std::mem::take(&mut service.prompts)
    }

    fn reply(&mut self, reply: Reply) {
        let mut service = self.0.borrow_mut();
        let shown = service.shown.clone();
        match reply {
            Reply::Cancel { id } if Some(&id) == shown.as_ref() => {
                service.told.push(format!("{id} cancelled"));
                service.done(id);
            }
            Reply::Confirm {
                id,
                choice,
                password,
            } if Some(&id) == shown.as_ref() => {
                let verifies = !matches!(ASKS[service.next - 1].1, Kind::Excluded | Kind::None);
                if !verifies {
                    service.told.push(format!("{id} acknowledged"));
                    service.done(id);
                } else if password.as_ref().is_some_and(|p| p.as_str() == PASSWORD) {
                    service.told.push(match choice {
                        Some(choice) => format!("{id} verified, account {choice}"),
                        None => format!("{id} verified"),
                    });
                    service.done(id);
                } else {
                    service.wrong += 1;
                    let throttled = service.wrong >= 2;
                    service.told.push(format!(
                        "{id} {}",
                        if throttled {
                            "throttled"
                        } else {
                            "wrong password"
                        }
                    ));
                    service.prompts.push(Prompt::Retry {
                        id,
                        throttled,
                        retry_after: throttled.then_some(30),
                    });
                }
            }
            other => service.told.push(format!("unexpected {other:?}")),
        }
    }
}
