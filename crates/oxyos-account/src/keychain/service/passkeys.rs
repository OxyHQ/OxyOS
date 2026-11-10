//! Passkeys in the service (ADR 0050 decision 7, step 5): the platform authenticator's ceremonies
//! for registered applications, each verified by the person on the session's sheet.
//!
//! - **Which relying parties** ([`RelyingParties`], then the vault): whether the keychain is open
//!   is asked first, of every application alike (`locked`); then an application may use an
//!   `rpId` associated with it (decision 5): equal to, or a registrable domain suffix of, one of
//!   its web origins' hosts (WebAuthn's rule, a public suffix never), the origins being Oxy's
//!   registry's (`webOrigins`) for its application, or the relying party of a passkey of its own
//!   (an item whose `apps` name it, made when it was associated or synced from another device).
//!   A login's domain associates nothing: an application names it when it asks to save, and the
//!   person's yes to a save is no grant of a relying party. The browser may use any: the browser
//!   is what the system's own configuration file (`/etc/oxyos/browsers`, root's, writable by no
//!   one else, one desktop entry id a line) names, never what an application declares. A relying
//!   party the application may not use, a keychain that is off, or one that needs its previous
//!   password, is refused `notAllowed` before anything is shown, so the application learns
//!   nothing of the keychain.
//! - **The person verifies** every create and get on the session's sheet, with the account's
//!   password, which the service checks itself (the keybag opened with it, throttled with
//!   Settings' verifications). Only then is a key made or a signature made. An excluded
//!   credential, or no passkey for the relying party, is told on the sheet too before its error,
//!   as CTAP2 asks for the person's presence before either (no silent probing).
//! - **One sheet at a time** per account: a second ceremony waits for the first. An application
//!   has one ceremony at a time: another while it waits is refused `notAllowed`.
//! - **What the sheet shows** of the relying party's names is drawn as itself: control characters
//!   and the marks that reorder text are taken out of the copy shown (the item keeps them).
//! - **A new passkey** is a `passkey` item written to the vault (synced) before anything is
//!   answered: a passkey Oxy did not take is never given to the application.

use super::*;
use crate::ipc::Connection;
use crate::keychain::passkey::*;
use crate::keychain::Asker;
use oxyos_keychain::passkey::{self as authenticator, ClientDataHash, NewPasskey};
use oxyos_keychain::wire::b64;
use std::collections::VecDeque;
use std::io;
use std::path::Path;

/// The system's list of browsers (ADR 0050 decision 7): desktop entry ids, one a line.
pub const BROWSERS: &str = "/etc/oxyos/browsers";
/// The longest browsers file read.
const MAX_BROWSERS: u64 = 64 * 1024;
/// How often a waiting ceremony looks whether its application went away.
const SLICE: Duration = Duration::from_millis(100);
/// How long the session's sheet connection may take to take a message.
const SEND_WAIT: Duration = Duration::from_secs(5);

/// The web origins of an application's Oxy registration (`https://example.com`), by its desktop
/// entry id.
pub type WebOrigins = Box<dyn Fn(&str) -> Vec<String> + Send + Sync>;

/// Which relying parties each application may use.
pub struct RelyingParties {
    /// The system's browsers file.
    pub browsers: PathBuf,
    /// Its owner, and its directory's: root (the tests', their own user).
    pub owner: u32,
    /// The registry's web origins of an application. Oxy does not answer them to the platform
    /// yet: the service's own has none, so only the browser may use passkeys until it does.
    pub web_origins: WebOrigins,
}

impl RelyingParties {
    /// The system's: `/etc/oxyos/browsers`, root's.
    pub fn system() -> Self {
        Self {
            browsers: PathBuf::from(BROWSERS),
            owner: 0,
            web_origins: Box::new(|_| Vec::new()),
        }
    }

    /// Whether `desktop_entry_id` is one of the system's browsers: the file must be a regular
    /// file (not a link), it and its directory the owner's and writable by no one else.
    pub fn is_browser(&self, desktop_entry_id: &str) -> bool {
        match read_browsers(&self.browsers, self.owner) {
            Ok(text) => text
                .lines()
                .map(str::trim)
                .any(|line| !line.starts_with('#') && line == desktop_entry_id),
            Err(error) => {
                if error.kind() != io::ErrorKind::NotFound {
                    eprintln!(
                        "oxyos-accountd: {} is not taken: {error}",
                        self.browsers.display()
                    );
                }
                false
            }
        }
    }

    /// Whether `desktop_entry_id` may use `rp_id`.
    pub fn allows(&self, desktop_entry_id: &str, rp_id: &str) -> bool {
        if !authenticator::valid_rp_id(rp_id) {
            return false;
        }
        if self.is_browser(desktop_entry_id) {
            return true;
        }
        // A public suffix is no relying party (WebAuthn: a registrable domain suffix).
        if psl::domain_str(rp_id).is_none() {
            return false;
        }
        (self.web_origins)(desktop_entry_id).iter().any(|origin| {
            url::Url::parse(origin)
                .ok()
                .filter(|url| url.scheme() == "https")
                .and_then(|url| url.host_str().map(str::to_owned))
                .is_some_and(|host| {
                    host == rp_id
                        || host
                            .strip_suffix(rp_id)
                            .is_some_and(|rest| rest.ends_with('.'))
                })
        })
    }
}

fn read_browsers(path: &Path, owner: u32) -> io::Result<String> {
    use rustix::fs::{fstat, open, FileType, Mode, OFlags};
    let unsafe_file = |what: &str| io::Error::new(io::ErrorKind::PermissionDenied, what.to_owned());
    let directory = path.parent().ok_or_else(|| unsafe_file("no directory"))?;
    let stat = rustix::fs::stat(directory)?;
    if stat.st_uid != owner || stat.st_mode & 0o022 != 0 {
        return Err(unsafe_file("its directory is not the system's alone"));
    }
    let file = open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )?;
    let stat = fstat(&file)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != owner
        || stat.st_mode & 0o022 != 0
    {
        return Err(unsafe_file("not a file the system's alone"));
    }
    let mut text = String::new();
    io::Read::read_to_string(
        &mut io::Read::take(std::fs::File::from(file), MAX_BROWSERS),
        &mut text,
    )?;
    Ok(text)
}

/// An account's sheets: the session's prompts connection and the ceremony on it.
#[derive(Default)]
pub(super) struct Prompts {
    host: Option<(u64, Arc<Connection>)>,
    hosts: u64,
    current: Option<Current>,
    ids: u64,
    /// The applications (desktop entry id, process) with a ceremony under way.
    asking: std::collections::HashSet<(String, i32)>,
}

struct Current {
    id: String,
    replies: VecDeque<Reply>,
}

enum Reply {
    Confirm {
        choice: Option<usize>,
        password: Option<Secret>,
    },
    Cancel,
}

/// What a confirmation came to.
enum Step<T> {
    Done(T),
    Retry(RetryReason, Option<u64>),
    Failed(Reason),
}

/// The sheet's place, given back however the ceremony ends.
struct Showing<'a> {
    user: &'a User,
    id: String,
    host: Arc<Connection>,
}

impl Drop for Showing<'_> {
    fn drop(&mut self) {
        let _ = self.host.send(&Prompt::PasskeyDone {
            id: self.id.clone(),
        });
        let mut prompts = lock(&self.user.prompts);
        if prompts
            .current
            .as_ref()
            .is_some_and(|current| current.id == self.id)
        {
            prompts.current = None;
        }
        drop(prompts);
        self.user.prompted.notify_all();
    }
}

fn client_data_hash(text: &str) -> Result<ClientDataHash, Reason> {
    let bytes = oxyos_keychain::wire::unb64(text, "client data hash")
        .map_err(|_| Reason::InvalidRequest)?;
    bytes.try_into().map_err(|_| Reason::InvalidRequest)
}

fn credential_list(ids: &[String]) -> Result<Vec<Vec<u8>>, Reason> {
    if ids.len() > authenticator::MAX_CREDENTIAL_LIST {
        return Err(Reason::InvalidRequest);
    }
    ids.iter()
        .map(|id| {
            oxyos_keychain::wire::unb64(id, "credential id")
                .ok()
                .filter(|bytes| !bytes.is_empty() && bytes.len() <= 1023)
                .ok_or(Reason::InvalidRequest)
        })
        .collect()
}

fn passkey_of(item: &Item) -> Option<&oxyos_keychain::item::Passkey> {
    match &item.content {
        Content::Passkey(passkey) => Some(passkey),
        _ => None,
    }
}

/// What the sheet shows of a name a relying party gave: drawn as itself, without the control
/// characters and the marks that reorder text, which could make the system's sheet read as
/// something else.
fn shown(text: &str) -> Secret {
    let kept: zeroize::Zeroizing<String> = zeroize::Zeroizing::new(
        text.chars()
            .filter(|c| {
                !(c.is_control()
                    || matches!(c, '\u{200e}' | '\u{200f}' | '\u{061c}')
                    || ('\u{202a}'..='\u{202e}').contains(c)
                    || ('\u{2066}'..='\u{2069}').contains(c))
            })
            .collect(),
    );
    Secret::new(kept.as_str())
}

/// What an application is told when its relying party is not to be served: the keychain off, or
/// waiting for its previous password, is no different from a relying party it may not use.
fn for_application(reason: Reason) -> Reason {
    match reason {
        Reason::Locked => Reason::Locked,
        _ => Reason::NotAllowed,
    }
}

/// An application's ceremony under way, let go however it ends.
struct Asking<'a> {
    user: &'a User,
    asker: (String, i32),
}

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        lock(&self.user.prompts).asking.remove(&self.asker);
    }
}

impl<V: VaultApi + 'static> KeychainService<V> {
    /// The relying parties' rule the service follows (the system's unless the tests set theirs).
    pub fn set_relying_parties(&self, parties: RelyingParties) {
        *lock(&self.parties) = Arc::new(parties);
    }

    /// Whether `uid`'s session's prompts connection is being served (the tests wait for it).
    #[cfg(test)]
    pub(crate) fn shows_sheets(&self, uid: u32) -> bool {
        lock(&self.users)
            .get(&uid)
            .is_some_and(|user| lock(&user.prompts).host.is_some())
    }

    fn parties(&self) -> Arc<RelyingParties> {
        lock(&self.parties).clone()
    }

    /// Whether `asker` may use `rp_id` now (whether it is a browser): refused `locked` while the
    /// keychain is, whatever the relying party, then `notAllowed` unless the keychain is a vault's
    /// member and the relying party is associated with the application ([`RelyingParties`], or a
    /// passkey of its own) or the application is the system's browser.
    fn admit(&self, user: &User, asker: &Asker<'_>, rp_id: &str) -> Result<bool, Reason> {
        let parties = self.parties();
        let id = asker.desktop_entry_id;
        let own = self
            .with_open(user, None, |open| {
                let vault = member_of(&mut open.vault, open.standing)?;
                Ok(vault.open_items().any(|(_, item)| {
                    item.ok().is_some_and(|item| {
                        passkey_of(&item).is_some_and(|passkey| {
                            passkey.rp_id.expose() == rp_id
                                && passkey.apps.iter().any(|app| app.expose() == id)
                        })
                    })
                }))
            })
            .map_err(for_application)?
            .1;
        if !authenticator::valid_rp_id(rp_id) {
            return Err(Reason::NotAllowed);
        }
        let browser = parties.is_browser(id);
        if browser || own || parties.allows(id, rp_id) {
            Ok(browser)
        } else {
            Err(Reason::NotAllowed)
        }
    }

    /// The account's password, checked as Settings' verification is (the keybag opened with it,
    /// throttled): the epoch it was checked in.
    pub(super) fn check_password(&self, user: &User, password: &Secret) -> Result<u64, Reason> {
        let epoch = {
            let data = lock(&user.data);
            if !matches!(data.phase, Phase::Open(_)) {
                return Err(Reason::Locked);
            }
            if data.throttle.waiting() {
                return Err(Reason::Throttled);
            }
            data.epoch
        };
        let store = self.store(user)?;
        let opened = store.unlock(password.expose());
        let mut data = lock(&user.data);
        let accepted = match opened {
            Ok(_) => true,
            Err(oxyos_keychain::Error::WrongPassword) => false,
            Err(error) => return Err(self.failed(user, "verifying", error)),
        };
        data.throttle.record(accepted);
        if !accepted {
            return Err(Reason::WrongPassword);
        }
        if data.epoch != epoch {
            return Err(Reason::Locked);
        }
        Ok(epoch)
    }

    /// Serves the session's prompts connection to its end: the person's answers routed to the
    /// ceremony they answer.
    pub(super) fn serve_prompts_of(&self, user: &User, connection: &Connection) -> io::Result<()> {
        let sender = connection.try_clone()?;
        sender.set_send_timeout(Some(SEND_WAIT))?;
        let host = {
            let mut prompts = lock(&user.prompts);
            prompts.hosts += 1;
            let host = prompts.hosts;
            prompts.host = Some((host, Arc::new(sender)));
            host
        };
        user.prompted.notify_all();
        // Told it is open only once it is: a ceremony asked from then on shows its sheet here.
        let opened = connection
            .send(&Answer::PasskeyPromptsOpen)
            .and_then(|()| connection.set_receive_timeout(None));
        let served = opened.and_then(|()| loop {
            let request = match connection.receive::<Request>() {
                Ok(Some(request)) => request,
                Ok(None) => break Ok(()),
                Err(error) => break Err(error),
            };
            let (id, reply) = match request {
                Some(Request::PasskeyConfirm {
                    id,
                    choice,
                    password,
                }) => (id, Reply::Confirm { choice, password }),
                Some(Request::PasskeyCancel { id }) => (id, Reply::Cancel),
                // Nothing else is asked here, and nothing else is answered.
                _ => continue,
            };
            let mut prompts = lock(&user.prompts);
            if let Some(current) = prompts.current.as_mut().filter(|current| current.id == id) {
                current.replies.push_back(reply);
                drop(prompts);
                user.prompted.notify_all();
            }
        });
        let mut prompts = lock(&user.prompts);
        if prompts.host.as_ref().is_some_and(|(held, _)| *held == host) {
            prompts.host = None;
        }
        drop(prompts);
        user.prompted.notify_all();
        served
    }

    /// Shows `confirm` on the session's sheet and runs `confirmed` on each confirmation, until it
    /// is done, the person cancels, the application goes, the keychain locks (`epoch`) or the
    /// time runs out.
    fn ceremony<T>(
        &self,
        user: &User,
        epoch: u64,
        asker: &Asker<'_>,
        mut confirm: ConfirmPasskey,
        mut confirmed: impl FnMut(Option<usize>, Option<Secret>) -> Step<T>,
    ) -> Result<T, Reason> {
        let deadline = Instant::now() + PASSKEY_WAIT;
        let still = |user: &User| -> Result<(), Reason> {
            if lock(&user.data).epoch != epoch {
                return Err(Reason::Locked);
            }
            if (asker.gone)() || Instant::now() >= deadline {
                return Err(Reason::NotAllowed);
            }
            Ok(())
        };
        // One ceremony at a time for an application: it cannot pile sheets up.
        let asking = (asker.desktop_entry_id.to_owned(), asker.pid);
        if !lock(&user.prompts).asking.insert(asking.clone()) {
            return Err(Reason::NotAllowed);
        }
        let _asking = Asking {
            user,
            asker: asking,
        };
        // The sheet's turn: one ceremony at a time.
        let showing = {
            let mut prompts = lock(&user.prompts);
            loop {
                still(user)?;
                let Some((_, host)) = prompts.host.clone() else {
                    eprintln!(
                        "oxyos-accountd: {} asked for a passkey and no session shows sheets",
                        asker.desktop_entry_id
                    );
                    return Err(Reason::NotAllowed);
                };
                if prompts.current.is_none() {
                    prompts.ids += 1;
                    let random = oxyos_keychain::enroll::nonce().map_err(|_| Reason::Failed)?;
                    let id = format!("{}-{}", prompts.ids, b64(&random[..9]));
                    prompts.current = Some(Current {
                        id: id.clone(),
                        replies: VecDeque::new(),
                    });
                    break Showing { user, id, host };
                }
                prompts = user
                    .prompted
                    .wait_timeout(prompts, SLICE)
                    .unwrap_or_else(|p| p.into_inner())
                    .0;
            }
        };
        confirm.id = showing.id.clone();
        if showing.host.send(&Prompt::ConfirmPasskey(confirm)).is_err() {
            return Err(Reason::NotAllowed);
        }
        loop {
            let reply = {
                let mut prompts = lock(&user.prompts);
                loop {
                    still(user)?;
                    if !prompts
                        .host
                        .as_ref()
                        .is_some_and(|(_, host)| Arc::ptr_eq(host, &showing.host))
                    {
                        // The session's sheets went with its connection.
                        return Err(Reason::NotAllowed);
                    }
                    let current = prompts
                        .current
                        .as_mut()
                        .filter(|current| current.id == showing.id)
                        .ok_or(Reason::NotAllowed)?;
                    if let Some(reply) = current.replies.pop_front() {
                        break reply;
                    }
                    prompts = user
                        .prompted
                        .wait_timeout(prompts, SLICE)
                        .unwrap_or_else(|p| p.into_inner())
                        .0;
                }
            };
            let (choice, password) = match reply {
                Reply::Cancel => return Err(Reason::NotAllowed),
                Reply::Confirm { choice, password } => (choice, password),
            };
            match confirmed(choice, password) {
                Step::Done(value) => return Ok(value),
                Step::Failed(reason) => return Err(reason),
                Step::Retry(reason, retry_after) => {
                    let _ = showing.host.send(&Prompt::PasskeyRetry {
                        id: showing.id.clone(),
                        reason,
                        retry_after,
                    });
                }
            }
        }
    }

    /// A confirmation's password checked: `None` when it was right.
    fn verified_step<T>(&self, user: &User, password: Option<Secret>) -> Option<Step<T>> {
        let Some(password) = password else {
            return Some(Step::Retry(RetryReason::WrongPassword, None));
        };
        match self.check_password(user, &password) {
            Ok(_) => None,
            Err(Reason::WrongPassword) => Some(Step::Retry(RetryReason::WrongPassword, None)),
            Err(Reason::Throttled) => Some(Step::Retry(
                RetryReason::Throttled,
                lock(&user.data).throttle.retry_after(),
            )),
            Err(reason) => Some(Step::Failed(reason)),
        }
    }

    /// The passkeys of `rp_id` the vault holds, by `keep`, each with its id: the epoch they were
    /// read in. Locked, or no member, refused.
    fn passkeys(
        &self,
        user: &User,
        rp_id: &str,
        keep: impl Fn(&oxyos_keychain::item::Passkey) -> bool,
    ) -> Result<(u64, Vec<(ItemId, PasskeyChoice)>), Reason> {
        self.with_open(user, None, |open| {
            let vault = member_of(&mut open.vault, open.standing)?;
            let mut found = Vec::new();
            for (id, item) in vault.open_items() {
                let Ok(item) = item else { continue };
                let Some(passkey) = passkey_of(&item) else {
                    continue;
                };
                if passkey.rp_id.expose() == rp_id && keep(passkey) {
                    found.push((
                        id.clone(),
                        PasskeyChoice {
                            user_name: shown(passkey.user_name.expose()),
                            display_name: shown(passkey.user_display_name.expose()),
                        },
                    ));
                }
            }
            found.sort_by(|a, b| a.1.user_name.expose().cmp(b.1.user_name.expose()));
            Ok(found)
        })
        .map_err(for_application)
    }

    /// Writes a new item to the vault (Oxy holding it before this returns), once more over what
    /// another device wrote first.
    fn write_new(&self, user: &User, uid: u32, item: &Item) -> Result<(), Reason> {
        let _net = lock(&user.net);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let (epoch, prepared) = self.with_open(user, None, |open| {
                let vault = member_of(&mut open.vault, open.standing)?;
                vault
                    .prepare(item)
                    .map_err(|error| self.failed(user, "sealing a passkey", error))
            })?;
            let (id, base, sealed) = (prepared.id.clone(), prepared.base, prepared.sealed.clone());
            match self.ask(user, uid, "writing a passkey", |api, token| {
                api.put_item(token, &id, base, &sealed)
            }) {
                Err(Reason::Conflict) if attempt == 1 => {
                    self.sync_held(user, uid)?;
                    continue;
                }
                Err(reason) => return Err(reason),
                Ok(answer) => {
                    return self
                        .with_open(user, Some(epoch), |open| {
                            let vault = member_of(&mut open.vault, open.standing)?;
                            vault.written(prepared, &answer).map_err(|error| {
                                self.failed(user, "Oxy's answer to a write", error)
                            })?;
                            vault
                                .save_items(&open.store)
                                .map_err(|error| self.failed(user, "keeping the items", error))
                        })
                        .map(drop)
                }
            }
        }
    }

    pub(super) fn passkey_create(
        &self,
        user: &User,
        uid: u32,
        asker: &Asker<'_>,
        request: &PasskeyCreate,
    ) -> Result<Answer, Reason> {
        // Whether it is open, then the relying party: an application that may not use it learns
        // nothing more.
        let browser = self.admit(user, asker, &request.rp_id)?;
        let _hash = client_data_hash(&request.client_data_hash)?;
        let user_handle = oxyos_keychain::wire::unb64(&request.user.id, "user handle")
            .map_err(|_| Reason::InvalidRequest)?;
        if user_handle.is_empty() || user_handle.len() > authenticator::MAX_USER_HANDLE {
            return Err(Reason::InvalidRequest);
        }
        let exclude = credential_list(&request.exclude_credentials)?;
        if request.algorithms.is_empty() {
            return Err(Reason::InvalidRequest);
        }
        if !authenticator::supports(&request.algorithms) {
            return Err(Reason::NotSupported);
        }
        let rp_id = request.rp_id.as_str();
        let (epoch, excluded) = self.passkeys(user, rp_id, |passkey| {
            authenticator::excluded(passkey, rp_id, &exclude)
        })?;
        let confirm = ConfirmPasskey {
            id: String::new(),
            desktop_entry_id: asker.desktop_entry_id.to_owned(),
            pid: asker.pid,
            rp_id: rp_id.to_owned(),
            user_name: Some(shown(request.user.name.expose())),
            kind: if excluded.is_empty() {
                PromptKind::Create
            } else {
                PromptKind::Excluded
            },
            choices: Vec::new(),
        };
        if !excluded.is_empty() {
            // CTAP2: the person's presence before InvalidStateError.
            return self.ceremony(user, epoch, asker, confirm, |_, _| {
                Step::<Answer>::Failed(Reason::InvalidState)
            });
        }
        self.ceremony(user, epoch, asker, confirm, |_, password| {
            if let Some(step) = self.verified_step(user, password) {
                return step;
            }
            let new = NewPasskey {
                rp_id,
                rp_name: request.rp_name.expose(),
                user_handle: &user_handle,
                user_name: request.user.name.expose(),
                user_display_name: request.user.display_name.expose(),
                // Made for the application, unless the browser made it for a page.
                apps: if browser {
                    Vec::new()
                } else {
                    vec![Secret::new(asker.desktop_entry_id)]
                },
            };
            let made = ItemId::random().map_err(|_| Reason::Failed).and_then(|id| {
                authenticator::create(&new, id, &oxyos_keychain::wire::now())
                    .map_err(|error| self.failed(user, "making a passkey", error))
            });
            let (item, registration) = match made {
                Ok(made) => made,
                Err(reason) => return Step::Failed(reason),
            };
            if let Err(reason) = self.write_new(user, uid, &item) {
                return Step::Failed(reason);
            }
            eprintln!(
                "oxyos-accountd: {} made a passkey for {}",
                user.name, asker.desktop_entry_id
            );
            Step::Done(Answer::PasskeyCreated(PasskeyCreated {
                credential_id: b64(&registration.credential_id),
                attestation_object: b64(&registration.attestation_object),
                authenticator_data: b64(&registration.authenticator_data),
                public_key: b64(&registration.public_key),
                public_key_algorithm: oxyos_keychain::item::ES256,
                transports: vec!["internal".into()],
            }))
        })
    }

    pub(super) fn passkey_get(
        &self,
        user: &User,
        asker: &Asker<'_>,
        request: &PasskeyGet,
    ) -> Result<Answer, Reason> {
        self.admit(user, asker, &request.rp_id)?;
        let hash = client_data_hash(&request.client_data_hash)?;
        let allow = credential_list(&request.allow_credentials)?;
        let rp_id = request.rp_id.as_str();
        let (epoch, found) = self.passkeys(user, rp_id, |passkey| {
            authenticator::matches(passkey, rp_id, &allow)
        })?;
        let (ids, choices): (Vec<ItemId>, Vec<PasskeyChoice>) = found.into_iter().unzip();
        let mut confirm = ConfirmPasskey {
            id: String::new(),
            desktop_entry_id: asker.desktop_entry_id.to_owned(),
            pid: asker.pid,
            rp_id: rp_id.to_owned(),
            user_name: choices
                .first()
                .map(|choice| Secret::new(choice.user_name.expose())),
            kind: PromptKind::Get,
            choices,
        };
        if ids.is_empty() {
            // CTAP2: the person's presence before saying there is none.
            confirm.kind = PromptKind::None;
            return self.ceremony(user, epoch, asker, confirm, |_, _| {
                Step::<Answer>::Failed(Reason::NotAllowed)
            });
        }
        self.ceremony(user, epoch, asker, confirm, |choice, password| {
            let Some(id) = ids.get(choice.unwrap_or(0)) else {
                return Step::Failed(Reason::InvalidRequest);
            };
            if let Some(step) = self.verified_step(user, password) {
                return step;
            }
            // Read again: it may have changed or gone while the sheet was shown.
            let signed = self.with_open(user, Some(epoch), |open| {
                let vault = member_of(&mut open.vault, open.standing)?;
                let item = vault
                    .open_item(id)
                    .ok()
                    .flatten()
                    .ok_or(Reason::NotAllowed)?;
                let passkey = passkey_of(&item)
                    .filter(|passkey| authenticator::matches(passkey, rp_id, &allow))
                    .ok_or(Reason::NotAllowed)?;
                authenticator::assert(passkey, rp_id, &hash)
                    .map_err(|error| self.failed(user, "signing with a passkey", error))
            });
            match signed {
                Ok((_, assertion)) => Step::Done(Answer::PasskeyAssertion(PasskeyAssertion {
                    credential_id: b64(&assertion.credential_id),
                    authenticator_data: b64(&assertion.authenticator_data),
                    signature: b64(&assertion.signature),
                    user_handle: b64(&assertion.user_handle),
                })),
                Err(reason) => Step::Failed(reason),
            }
        })
    }
}
