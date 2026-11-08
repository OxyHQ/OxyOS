//! AutoFill's provider and the save confirmations for the session
//! (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decisions 5 and 6): the accounts' service's
//! keychain, reached on registrar connections of the session's own, each on a thread of its own,
//! so the compositor's turn never waits for the service.
//!
//! - **The provider** asks the service for a field's suggestions, the item the person picked and a
//!   strong password (`RegistrarClient`), one question after another on `ThreadedProvider`'s
//!   thread. A connection that broke is made again for the next question; the service unreachable,
//!   a field is offered nothing. A service that refuses the session (`notPermitted`: a session the
//!   login program did not start is no registrar, ADR 0020 step 5) is not asked again: AutoFill
//!   stays off, which is not an error.
//! - **The save confirmations** (`SaveConfirmations`): an application's request to save, told by
//!   the service, becomes the compositor's [`SaveAsk`], asked in the shell's sheet; the person's
//!   answer goes back.
//!
//! The development compositor's fixture keychain is never this: a session asks only the service.

use oxyos_account::keychain::{
    Field as Asked, Fill, RegistrarClient, SaveConfirmations, SaveEvent,
};
use oxyos_account::session::Reason;
use oxyos_compositor::autofill::{
    Answer, Ask, Field, SaveAsk, Secret, Suggestion, ThreadedProvider,
};
use std::path::PathBuf;

fn asked(field: Field) -> Asked {
    match field {
        Field::Username => Asked::Username,
        Field::Email => Asked::Email,
        Field::Password => Asked::Password,
        Field::NewPassword => Asked::NewPassword,
        Field::OneTimeCode => Asked::OneTimeCode,
    }
}

/// One question on `connection`: its answer, `Ok(Err)` the service's refusal, `Err` the
/// connection's failure.
fn answer(connection: &RegistrarClient, ask: &Ask) -> std::io::Result<Result<Answer, Reason>> {
    Ok(match ask {
        Ask::Suggest {
            desktop_entry_id,
            field,
        } => connection
            .autofill_suggest(desktop_entry_id, asked(*field))?
            .map(|suggestions| {
                Answer::Suggestions(
                    suggestions
                        .into_iter()
                        .map(|suggestion| Suggestion {
                            reference: suggestion.reference,
                            title: suggestion.title.expose().to_owned(),
                            username: suggestion.username.expose().to_owned(),
                        })
                        .collect(),
                )
            }),
        Ask::Fill {
            desktop_entry_id,
            reference,
            field,
        } => connection
            .autofill_fill(desktop_entry_id, reference, asked(*field))?
            .map(|fill| match fill {
                Fill::Credential { username, password } => Answer::Credential {
                    username: Secret::new(username.expose()),
                    password: Secret::new(password.expose()),
                },
                Fill::Code(code) => Answer::Code(Secret::new(code.expose())),
            }),
        Ask::Generate => connection
            .autofill_generate()?
            .map(|password| Answer::Generated(Secret::new(password.expose()))),
    })
}

/// The reason's name on the wire (`locked`), for the compositor's log of a refusal.
fn reason_name(reason: Reason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "refused".into())
}

/// AutoFill's provider: the service at `socket`, asked on a thread of its own.
pub fn provider(socket: PathBuf) -> std::io::Result<ThreadedProvider> {
    let mut client: Option<RegistrarClient> = None;
    // The service refused this session: no registrar, no AutoFill, for the session's life.
    let mut off = false;
    let mut said = false;
    ThreadedProvider::spawn(move |ask| {
        if off {
            return Answer::Refused("notPermitted".into());
        }
        // A connection kept from an earlier question may have broken since (the service
        // restarted): asked once again on a new one.
        for _ in 0..2 {
            if client.is_none() {
                match RegistrarClient::connect(&socket) {
                    Ok(connected) => {
                        said = false;
                        client = Some(connected);
                    }
                    Err(error) => {
                        if !std::mem::replace(&mut said, true) {
                            eprintln!(
                                "[session] AutoFill: the accounts' service at {}: {error}; \
                                 nothing is offered until it answers",
                                socket.display()
                            );
                        }
                        break;
                    }
                }
            }
            let Some(connection) = client.as_ref() else {
                break;
            };
            match answer(connection, &ask) {
                Ok(Ok(answered)) => return answered,
                Ok(Err(Reason::NotPermitted)) => {
                    eprintln!("[session] AutoFill: the accounts' service does not take this session's questions: off");
                    off = true;
                    client = None;
                    return Answer::Refused("notPermitted".into());
                }
                Ok(Err(reason)) => return Answer::Refused(reason_name(reason)),
                Err(_) => client = None,
            }
        }
        Answer::Refused("unreachable".into())
    })
}

/// The service's save confirmations, as the compositor's AutoFill asks them.
pub struct Saves(pub SaveConfirmations);

impl oxyos_compositor::autofill::SaveConfirmations for Saves {
    fn take(&mut self) -> Vec<SaveAsk> {
        self.0
            .take()
            .into_iter()
            .map(|event| match event {
                SaveEvent::Confirm(confirmation) => SaveAsk::Confirm {
                    id: confirmation.id,
                    desktop_entry_id: confirmation.desktop_entry_id,
                    username: confirmation.username.expose().to_owned(),
                    domain: confirmation.domain,
                },
                SaveEvent::Withdrawn(id) => SaveAsk::Withdrawn { id },
            })
            .collect()
    }

    fn decide(&mut self, id: &str, save: bool) {
        self.0.decide(id, save);
    }
}

/// The save confirmations of the service at `socket`, on a thread of their own.
pub fn saves(socket: PathBuf) -> std::io::Result<Saves> {
    SaveConfirmations::spawn(socket, |said| {
        eprintln!("[session] saving passwords: {said}")
    })
    .map(Saves)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_account::ipc::{Connection, Listener};
    use oxyos_account::keychain::{SaveConfirmation, Suggestion as Offered};
    use oxyos_account::session::{Answer as Told, Request};
    use oxyos_account::Secret as AccountSecret;
    use oxyos_compositor::autofill::{AutofillProvider, SaveConfirmations as _};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, Instant};

    /// A directory of the test's own for a socket.
    fn socket_path(name: &str) -> (PathBuf, impl Drop) {
        struct Gone(PathBuf);
        impl Drop for Gone {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let dir = std::env::temp_dir().join(format!(
            "oxyos-session-autofill-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (dir.join("session"), Gone(dir))
    }

    /// A played accounts' service: each connection on a thread, each request answered by
    /// `answer` (none: the connection is closed); connections counted.
    fn played(
        path: &std::path::Path,
        answer: impl Fn(&Connection, Request) -> Option<Told> + Send + Sync + 'static,
    ) -> Arc<AtomicUsize> {
        let listener = Listener::bind(path).unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let counted = connections.clone();
        let answer = Arc::new(answer);
        std::thread::spawn(move || loop {
            let Ok(connection) = listener.accept() else {
                return;
            };
            counted.fetch_add(1, Ordering::SeqCst);
            let answer = answer.clone();
            std::thread::spawn(move || {
                while let Ok(Some(Some(request))) = connection.receive::<Request>() {
                    match answer(&connection, request) {
                        Some(told) => {
                            if connection.send(&told).is_err() {
                                return;
                            }
                        }
                        None => return,
                    }
                }
            });
        });
        connections
    }

    /// The answer to `ask`, waiting on the provider's descriptor as the compositor's turn does.
    fn asked_of(provider: &mut ThreadedProvider, id: u64, ask: Ask) -> Answer {
        let wake = provider.wake_fd().unwrap();
        provider.ask(id, ask);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut fds = [rustix::event::PollFd::new(
                &wake,
                rustix::event::PollFlags::IN,
            )];
            let _ = rustix::event::poll(
                &mut fds,
                Some(&rustix::time::Timespec {
                    tv_sec: 1,
                    tv_nsec: 0,
                }),
            );
            if let Some((answered, answer)) = provider.answers().pop() {
                assert_eq!(answered, id);
                return answer;
            }
            assert!(Instant::now() < deadline, "no answer");
        }
    }

    #[test]
    fn the_provider_asks_the_service_and_hands_its_answers_on() {
        let (path, _gone) = socket_path("provider");
        let asked = Arc::new(Mutex::new(Vec::new()));
        let held = asked.clone();
        played(&path, move |_, request| {
            held.lock()
                .unwrap()
                .push(serde_json::to_value(&request).unwrap());
            Some(match request {
                Request::AutofillSuggest {
                    desktop_entry_id, ..
                } if desktop_entry_id == "so.oxy.mail" => {
                    Told::AutofillSuggestions(vec![Offered {
                        reference: "r1".into(),
                        title: AccountSecret::new("example.com"),
                        username: AccountSecret::new("bea@example.com"),
                    }])
                }
                Request::AutofillSuggest { .. } => Told::AutofillSuggestions(Vec::new()),
                Request::AutofillFill {
                    field: Asked::OneTimeCode,
                    ..
                } => Told::AutofillCode {
                    code: AccountSecret::new("123456"),
                },
                Request::AutofillFill { .. } => Told::AutofillCredential {
                    username: AccountSecret::new("bea@example.com"),
                    password: AccountSecret::new("made-up-1"),
                },
                Request::AutofillGenerate {} => {
                    Told::GeneratedPassword(AccountSecret::new("abcde-FGHIJ-12345-klmno"))
                }
                _ => Told::refused(Reason::Locked),
            })
        });
        let mut provider = provider(path).unwrap();
        assert_eq!(
            asked_of(
                &mut provider,
                1,
                Ask::Suggest {
                    desktop_entry_id: "so.oxy.mail".into(),
                    field: Field::Username,
                }
            ),
            Answer::Suggestions(vec![Suggestion {
                reference: "r1".into(),
                title: "example.com".into(),
                username: "bea@example.com".into(),
            }])
        );
        assert_eq!(
            asked_of(
                &mut provider,
                2,
                Ask::Fill {
                    desktop_entry_id: "so.oxy.mail".into(),
                    reference: "r1".into(),
                    field: Field::Password,
                }
            ),
            Answer::Credential {
                username: Secret::new("bea@example.com"),
                password: Secret::new("made-up-1"),
            }
        );
        assert_eq!(
            asked_of(
                &mut provider,
                3,
                Ask::Fill {
                    desktop_entry_id: "so.oxy.mail".into(),
                    reference: "r1".into(),
                    field: Field::OneTimeCode,
                }
            ),
            Answer::Code(Secret::new("123456"))
        );
        assert_eq!(
            asked_of(&mut provider, 4, Ask::Generate),
            Answer::Generated(Secret::new("abcde-FGHIJ-12345-klmno"))
        );
        // The wire as the service reads it: the launch record's id, the field's name.
        let asked = asked.lock().unwrap();
        assert_eq!(
            asked[0],
            serde_json::json!({"autofillSuggest": {"desktopEntryId": "so.oxy.mail", "field": "username"}})
        );
        assert_eq!(
            asked[1],
            serde_json::json!({"autofillFill": {"desktopEntryId": "so.oxy.mail", "ref": "r1", "field": "password"}})
        );
    }

    #[test]
    fn a_refusal_offers_nothing_and_a_service_that_refuses_the_session_is_not_asked_again() {
        let (path, _gone) = socket_path("refused");
        let locked = Arc::new(Mutex::new(true));
        let still = locked.clone();
        let connections = played(&path, move |_, _| {
            Some(if *still.lock().unwrap() {
                Told::refused(Reason::Locked)
            } else {
                Told::refused(Reason::NotPermitted)
            })
        });
        let mut provider = provider(path).unwrap();
        assert_eq!(
            asked_of(&mut provider, 1, Ask::Generate),
            Answer::Refused("locked".into())
        );
        *locked.lock().unwrap() = false;
        assert_eq!(
            asked_of(&mut provider, 2, Ask::Generate),
            Answer::Refused("notPermitted".into())
        );
        let made = connections.load(Ordering::SeqCst);
        for id in 3..6 {
            assert_eq!(
                asked_of(&mut provider, id, Ask::Generate),
                Answer::Refused("notPermitted".into())
            );
        }
        assert_eq!(connections.load(Ordering::SeqCst), made, "asked again");
    }

    #[test]
    fn an_unreachable_service_offers_nothing_and_a_restarted_one_is_asked_again() {
        let (path, _gone) = socket_path("unreachable");
        let mut provider = provider(path.clone()).unwrap();
        assert_eq!(
            asked_of(&mut provider, 1, Ask::Generate),
            Answer::Refused("unreachable".into())
        );
        played(&path, |_, _| {
            Some(Told::GeneratedPassword(AccountSecret::new(
                "abcde-FGHIJ-12345-klmno",
            )))
        });
        assert_eq!(
            asked_of(&mut provider, 2, Ask::Generate),
            Answer::Generated(Secret::new("abcde-FGHIJ-12345-klmno"))
        );
    }

    /// Waits until `saves` has something, as the compositor's turn does on its descriptor.
    fn arrived(saves: &mut Saves) -> Vec<SaveAsk> {
        let wake = saves.0.wake_fd().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let taken = saves.take();
            if !taken.is_empty() {
                return taken;
            }
            assert!(Instant::now() < deadline, "nothing arrived");
            let mut fds = [rustix::event::PollFd::new(
                &wake,
                rustix::event::PollFlags::IN,
            )];
            let _ = rustix::event::poll(
                &mut fds,
                Some(&rustix::time::Timespec {
                    tv_sec: 1,
                    tv_nsec: 0,
                }),
            );
        }
    }

    #[test]
    fn a_request_to_save_is_handed_on_and_only_its_answer_sent_back() {
        let (path, _gone) = socket_path("saves");
        // The played service: the session's connection asks for the confirmations, then the test
        // speaks on it, and a thread of its own reads the answers.
        let listener = Listener::bind(&path).unwrap();
        let (connected, confirmer) = mpsc::channel::<Arc<Connection>>();
        let (decided, decisions) = mpsc::channel();
        std::thread::spawn(move || {
            let connection = Arc::new(listener.accept().unwrap());
            assert!(matches!(
                connection.receive::<Request>(),
                Ok(Some(Some(Request::ConfirmSaves {})))
            ));
            connection.send(&Told::ConfirmingSaves).unwrap();
            connected.send(connection.clone()).unwrap();
            while let Ok(Some(Some(Request::SaveDecision { id, save }))) =
                connection.receive::<Request>()
            {
                let _ = decided.send((id, save));
            }
        });
        let mut saves = saves(path).unwrap();
        let service = confirmer.recv_timeout(Duration::from_secs(10)).unwrap();
        service
            .send(&Told::ConfirmSave(SaveConfirmation {
                id: "1".into(),
                desktop_entry_id: "so.oxy.mail".into(),
                username: AccountSecret::new("bea@example.com"),
                domain: Some("example.com".into()),
            }))
            .unwrap();
        assert_eq!(
            arrived(&mut saves),
            [SaveAsk::Confirm {
                id: "1".into(),
                desktop_entry_id: "so.oxy.mail".into(),
                username: "bea@example.com".into(),
                domain: Some("example.com".into()),
            }]
        );
        // An answer for a request it was never asked is not sent; this one's is, once.
        saves.decide("9", true);
        saves.decide("1", true);
        saves.decide("1", false);
        assert_eq!(
            decisions.recv_timeout(Duration::from_secs(10)).unwrap(),
            ("1".to_string(), true)
        );
        assert!(decisions.recv_timeout(Duration::from_millis(300)).is_err());
        // A request withdrawn is handed on, and its answer is not sent.
        service
            .send(&Told::ConfirmSave(SaveConfirmation {
                id: "2".into(),
                desktop_entry_id: "so.oxy.mail".into(),
                username: AccountSecret::new("bea@example.com"),
                domain: None,
            }))
            .unwrap();
        assert_eq!(arrived(&mut saves).len(), 1);
        service
            .send(&Told::SaveWithdrawn { id: "2".into() })
            .unwrap();
        assert_eq!(arrived(&mut saves), [SaveAsk::Withdrawn { id: "2".into() }]);
        saves.decide("2", true);
        assert!(decisions.recv_timeout(Duration::from_millis(300)).is_err());
        // A request open when the service goes is withdrawn.
        service
            .send(&Told::ConfirmSave(SaveConfirmation {
                id: "3".into(),
                desktop_entry_id: "so.oxy.mail".into(),
                username: AccountSecret::new("bea@example.com"),
                domain: None,
            }))
            .unwrap();
        assert_eq!(arrived(&mut saves).len(), 1);
        rustix::net::shutdown(&*service, rustix::net::Shutdown::Both).unwrap();
        assert_eq!(arrived(&mut saves), [SaveAsk::Withdrawn { id: "3".into() }]);
    }
}
