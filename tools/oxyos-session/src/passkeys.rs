//! The session's passkey sheets (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7):
//! the accounts' service's prompts, on a registrar connection of the session's own
//! (`oxyos_account::keychain::passkey::PasskeySheets`, on its thread), handed to the compositor's
//! passkeys part, which shows them in the shell's system sheet; the person's answers go back the
//! same way. The login screen has none: it runs no application.

use oxyos_account::keychain::passkey::Prompt as Told;
use oxyos_account::keychain::passkey::{PasskeySheets, PromptKind, RetryReason};
use oxyos_account::Secret;
use oxyos_compositor::passkeys::{Choice, Kind, PasskeyPrompts, Prompt, Reply};

pub struct Sheets(pub PasskeySheets);

fn kind(kind: PromptKind) -> Kind {
    match kind {
        PromptKind::Create => Kind::Create,
        PromptKind::Get => Kind::Get,
        PromptKind::Excluded => Kind::Excluded,
        PromptKind::None => Kind::None,
    }
}

impl PasskeyPrompts for Sheets {
    fn take(&mut self) -> Vec<Prompt> {
        self.0
            .take()
            .into_iter()
            .map(|told| match told {
                Told::ConfirmPasskey(confirm) => Prompt::Confirm {
                    id: confirm.id,
                    desktop_entry_id: confirm.desktop_entry_id,
                    pid: confirm.pid,
                    rp_id: confirm.rp_id,
                    user_name: confirm
                        .user_name
                        .map(|name| oxyos_compositor::autofill::Secret::new(name.expose())),
                    kind: kind(confirm.kind),
                    choices: confirm
                        .choices
                        .iter()
                        .map(|choice| Choice {
                            user_name: choice.user_name.expose().to_owned(),
                            display_name: choice.display_name.expose().to_owned(),
                        })
                        .collect(),
                },
                Told::PasskeyRetry {
                    id,
                    reason,
                    retry_after,
                } => Prompt::Retry {
                    id,
                    throttled: reason == RetryReason::Throttled,
                    retry_after,
                },
                Told::PasskeyDone { id } => Prompt::Done { id },
            })
            .collect()
    }

    fn reply(&mut self, reply: Reply) {
        match reply {
            Reply::Confirm {
                id,
                choice,
                password,
            } => self.0.confirm(
                &id,
                choice,
                password.map(|password| Secret::new(password.as_str())),
            ),
            Reply::Cancel { id } => self.0.cancel(&id),
        }
    }
}
