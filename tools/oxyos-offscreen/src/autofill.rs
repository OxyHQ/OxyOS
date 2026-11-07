//! The development compositor's fixture keychain for AutoFill
//! (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 6), with `--autofill-fixture ID`:
//! an `AutofillProvider` answering at once from fixed, made-up items associated with the
//! application `ID` alone, as the accounts' service answers a session from the person's keychain.
//! It is a fixture of this fixture compositor, never installed, never a session's: no real
//! credential is in it, and nothing it holds is a secret.

use oxyos_compositor::autofill::{Answer, Ask, AutofillProvider, Field, Secret, Suggestion};

/// The fixture's logins: reference, title, username, password.
const LOGINS: [(&str, &str, &str, &str); 2] = [
    (
        "fixture-login-1",
        "example.com",
        "bea@example.com",
        "fixture-Passw0rd-1",
    ),
    (
        "fixture-login-2",
        "example.com",
        "ana@example.com",
        "fixture-Passw0rd-2",
    ),
];
/// The strong password it makes, in the generator's shape (four groups of five).
pub const STRONG: &str = "fixtr-Strng-Pass0-wordX";
/// The verification code it gives for a login.
const CODE: &str = "246810";

pub struct FixtureKeychain {
    /// The application its items are associated with: any other is offered nothing.
    application: String,
    answers: Vec<(u64, Answer)>,
    /// What it was asked, for a step's result: the question's kind and field, never an item.
    pub asked: Vec<String>,
}

impl FixtureKeychain {
    pub fn new(application: String) -> Self {
        Self {
            application,
            answers: Vec::new(),
            asked: Vec::new(),
        }
    }
}

/// A shared handle the director keeps to read what was asked, while the compositor's AutoFill
/// owns the provider.
pub struct Shared(pub std::rc::Rc<std::cell::RefCell<FixtureKeychain>>);

impl AutofillProvider for Shared {
    fn ask(&mut self, id: u64, ask: Ask) {
        let mut keychain = self.0.borrow_mut();
        let ours = |application: &str| application == keychain.application;
        let (said, answer) = match &ask {
            Ask::Suggest {
                desktop_entry_id,
                field,
            } => (
                format!("suggest {}", field.as_str()),
                Answer::Suggestions(if ours(desktop_entry_id) {
                    LOGINS
                        .iter()
                        .map(|(reference, title, username, _)| Suggestion {
                            reference: (*reference).into(),
                            title: (*title).into(),
                            username: (*username).into(),
                        })
                        .collect()
                } else {
                    Vec::new()
                }),
            ),
            Ask::Fill {
                desktop_entry_id,
                reference,
                field,
            } => {
                let login = LOGINS.iter().find(|(r, ..)| r == reference);
                let answer = match (ours(desktop_entry_id), login, field) {
                    (true, Some(_), Field::OneTimeCode) => Answer::Code(Secret::new(CODE)),
                    (true, Some((_, _, username, password)), _) => Answer::Credential {
                        username: Secret::new(*username),
                        password: Secret::new(*password),
                    },
                    _ => Answer::Refused("notPermitted".into()),
                };
                (format!("fill {}", field.as_str()), answer)
            }
            Ask::Generate => ("generate".into(), Answer::Generated(Secret::new(STRONG))),
        };
        keychain.asked.push(said);
        keychain.answers.push((id, answer));
    }

    fn answers(&mut self) -> Vec<(u64, Answer)> {
        std::mem::take(&mut self.0.borrow_mut().answers)
    }
}
