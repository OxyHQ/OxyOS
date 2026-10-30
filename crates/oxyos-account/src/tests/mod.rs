//! Every rule of setting an account up, without root and without the network: Oxy's API is a fake
//! (or, for the HTTP client, a server of the tests' own on this machine), the users are a fake or
//! the system's real tools on a root of the tests' own (`--prefix`), and the store is real, in a
//! directory of the tests' own.

mod api;
mod dev_server;
mod id;
mod ipc;
mod setup;
mod store;
mod users;

use crate::api::{ApiError, DeviceCredential, OxyApi, Profile, SignInStep, SignedIn};
use crate::users::{UnixUser, Users, UsersError};
use crate::{AccountId, Secret, UserName};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub(crate) const ACCOUNT: &str = "0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e";
pub(crate) const OTHER_ACCOUNT: &str = "64f7c2a1b8e9d3f4a1c2b3d4";
pub(crate) const DEVICE: &str = "device-1";
pub(crate) const DEVICE_SECRET: &str = "device-secret-5a1c9e";
/// Another holder secret of the same device (each sign-in issues one; none rotates).
pub(crate) const OTHER_DEVICE_SECRET: &str = "device-secret-77d0b2";
pub(crate) const PASSWORD: &str = "correct horse battery staple";
pub(crate) const USERNAME: &str = "ada";
pub(crate) const CODE: &str = "246810";
pub(crate) const CHALLENGE: &str = "challenge-7f3b";

/// A directory of the test's own, removed when dropped.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "oxyos-account-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn credential() -> DeviceCredential {
    DeviceCredential {
        device_id: DEVICE.into(),
        device_secret: Secret::new(DEVICE_SECRET),
    }
}

pub(crate) fn account() -> AccountId {
    AccountId::parse(ACCOUNT).unwrap()
}

/// The calls a [`FakeApi`] answers, in a setup's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Call {
    Mint,
    Profile,
    Password,
    SecondFactor,
}

/// Oxy, as its contracts describe it, for one account on one device.
pub(crate) struct FakeApi {
    /// From this call on, Oxy cannot be reached.
    pub(crate) offline_from: Option<Call>,
    /// The accounts signed in on the device.
    pub(crate) on_device: Vec<&'static str>,
    pub(crate) password: &'static str,
    /// The account has an authenticator.
    pub(crate) authenticator: bool,
    /// Who `/users/me` says the token is for, if not the account it was minted for.
    pub(crate) profile_id: Option<&'static str>,
    /// Who and where a sign-in says it signed in, if not the account and the device.
    pub(crate) signs_in: Option<(&'static str, &'static str)>,
    pub(crate) locked: bool,
    pub(crate) calls: Mutex<Vec<Call>>,
}

impl Default for FakeApi {
    fn default() -> Self {
        Self {
            offline_from: None,
            on_device: vec![ACCOUNT],
            password: PASSWORD,
            authenticator: false,
            profile_id: None,
            signs_in: None,
            locked: false,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl FakeApi {
    fn call(&self, call: Call) -> Result<(), ApiError> {
        self.calls.lock().unwrap().push(call);
        match self.offline_from {
            Some(from) if call >= from => Err(ApiError::Offline("no route to host".into())),
            _ => Ok(()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn proven(&self, device: &DeviceCredential) -> bool {
        device.device_id == DEVICE
            && [DEVICE_SECRET, OTHER_DEVICE_SECRET].contains(&device.device_secret.expose())
    }

    fn signed_in(&self, device: &DeviceCredential) -> SignedIn {
        let (user, device_id) = self.signs_in.unwrap_or((ACCOUNT, DEVICE));
        assert!(self.proven(device), "a sign-in carries the device's proof");
        SignedIn {
            user_id: user.into(),
            device_id: device_id.into(),
        }
    }
}

impl OxyApi for FakeApi {
    fn mint_token(
        &self,
        credential: &DeviceCredential,
        account: &AccountId,
    ) -> Result<Secret, ApiError> {
        self.call(Call::Mint)?;
        if !self.proven(credential) {
            return Err(ApiError::InvalidDeviceSecret);
        }
        if !self.on_device.contains(&account.as_str()) {
            return Err(ApiError::AccountNotOnDevice);
        }
        Ok(Secret::new(format!("token-for-{account}")))
    }

    fn profile(&self, token: &Secret) -> Result<Profile, ApiError> {
        self.call(Call::Profile)?;
        let id = token
            .expose()
            .strip_prefix("token-for-")
            .ok_or(ApiError::Refused {
                status: 401,
                code: None,
            })?;
        Ok(Profile {
            id: self.profile_id.unwrap_or(id).into(),
            username: Some(USERNAME.into()),
            display_name: Some("Ada Lovelace".into()),
            avatar: Some("avatar-file-1".into()),
        })
    }

    fn sign_in_password(
        &self,
        identifier: &str,
        password: &Secret,
        device: &DeviceCredential,
    ) -> Result<SignInStep, ApiError> {
        self.call(Call::Password)?;
        if self.locked {
            return Err(ApiError::Locked {
                retry_after: Some(900),
            });
        }
        if identifier != USERNAME || password.expose() != self.password {
            return Err(ApiError::InvalidCredentials);
        }
        if self.authenticator {
            return Ok(SignInStep::SecondFactor {
                challenge: Secret::new(CHALLENGE),
            });
        }
        Ok(SignInStep::SignedIn(self.signed_in(device)))
    }

    fn second_factor(
        &self,
        challenge: &Secret,
        code: &Secret,
        device: &DeviceCredential,
    ) -> Result<SignedIn, ApiError> {
        self.call(Call::SecondFactor)?;
        if challenge.expose() != CHALLENGE || code.expose() != CODE {
            return Err(ApiError::SecondFactorInvalid);
        }
        Ok(self.signed_in(device))
    }
}

/// What a [`FakeUsers`] may be told to fail at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum UserStep {
    Lookup,
    Create,
    SetVerifier,
    Remove,
}

/// The machine's users, in memory: each user's entry and verifier (the password itself, as a
/// fake may keep it).
#[derive(Default)]
pub(crate) struct FakeUsers {
    pub(crate) users: Mutex<BTreeMap<String, (UnixUser, Option<String>)>>,
    pub(crate) failing: Mutex<BTreeSet<UserStep>>,
    pub(crate) steps: Mutex<Vec<UserStep>>,
}

impl FakeUsers {
    pub(crate) fn fail(&self, step: UserStep) {
        self.failing.lock().unwrap().insert(step);
    }

    fn step(&self, step: UserStep) -> Result<(), UsersError> {
        self.steps.lock().unwrap().push(step);
        if self.failing.lock().unwrap().contains(&step) {
            return Err(UsersError::Failed(format!("{step:?} failed")));
        }
        Ok(())
    }

    pub(crate) fn add(&self, name: &str, uid: u32) {
        self.users.lock().unwrap().insert(
            name.into(),
            (
                UnixUser {
                    uid,
                    gid: uid,
                    home: PathBuf::from(format!("/home/{name}")),
                },
                None,
            ),
        );
    }

    pub(crate) fn verifier(&self, name: &UserName) -> Option<String> {
        self.users
            .lock()
            .unwrap()
            .get(name.as_str())
            .and_then(|(_, verifier)| verifier.clone())
    }

    pub(crate) fn names(&self) -> Vec<String> {
        self.users.lock().unwrap().keys().cloned().collect()
    }

    pub(crate) fn steps(&self) -> Vec<UserStep> {
        self.steps.lock().unwrap().clone()
    }
}

impl Users for FakeUsers {
    fn lookup(&self, name: &UserName) -> Result<Option<UnixUser>, UsersError> {
        self.step(UserStep::Lookup)?;
        Ok(self
            .users
            .lock()
            .unwrap()
            .get(name.as_str())
            .map(|(user, _)| user.clone()))
    }

    fn create(&self, name: &UserName) -> Result<UnixUser, UsersError> {
        self.step(UserStep::Create)?;
        let mut users = self.users.lock().unwrap();
        if users.contains_key(name.as_str()) {
            return Err(UsersError::Exists);
        }
        let uid = 1000 + users.len() as u32;
        let user = UnixUser {
            uid,
            gid: uid,
            home: PathBuf::from(format!("/home/{name}")),
        };
        users.insert(name.as_str().into(), (user.clone(), None));
        Ok(user)
    }

    fn set_verifier(&self, name: &UserName, password: &Secret) -> Result<(), UsersError> {
        self.step(UserStep::SetVerifier)?;
        match self.users.lock().unwrap().get_mut(name.as_str()) {
            Some((_, verifier)) => {
                *verifier = Some(password.expose().into());
                Ok(())
            }
            None => Err(UsersError::Missing),
        }
    }

    fn remove(&self, name: &UserName) -> Result<(), UsersError> {
        self.step(UserStep::Remove)?;
        match self.users.lock().unwrap().remove(name.as_str()) {
            Some(_) => Ok(()),
            None => Err(UsersError::Missing),
        }
    }
}

/// A system root of the test's own for the real passwd tools (`--prefix`): its own account
/// database and login.defs, hashing with yescrypt as Debian trixie's does.
pub(crate) struct Root(TempDir);

impl Root {
    pub(crate) fn new(name: &str) -> Self {
        let dir = TempDir::new(name);
        let etc = dir.path().join("etc");
        fs::create_dir_all(&etc).unwrap();
        fs::create_dir_all(dir.path().join("home")).unwrap();
        fs::write(etc.join("passwd"), "root:x:0:0:root:/root:/bin/sh\n").unwrap();
        fs::write(etc.join("shadow"), "root:*:20000:0:99999:7:::\n").unwrap();
        fs::write(etc.join("group"), "root:x:0:\n").unwrap();
        fs::write(etc.join("gshadow"), "root:*::\n").unwrap();
        fs::write(
            etc.join("login.defs"),
            "UID_MIN 1000\nUID_MAX 60000\nGID_MIN 1000\nGID_MAX 60000\nHOME_MODE 0700\n\
             USERGROUPS_ENAB yes\nENCRYPT_METHOD YESCRYPT\n",
        )
        .unwrap();
        Self(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        self.0.path()
    }

    pub(crate) fn read(&self, file: &str) -> String {
        fs::read_to_string(self.path().join("etc").join(file)).unwrap()
    }

    /// `name`'s field in the shadow file: its verifier.
    pub(crate) fn shadow(&self, name: &str) -> Option<String> {
        self.read("shadow").lines().find_map(|line| {
            let mut fields = line.split(':');
            (fields.next() == Some(name)).then(|| fields.next().unwrap_or("").to_owned())
        })
    }
}

/// Set to `required` (as CI's Debian job does) to fail, not skip, where [`passwd_tools`] finds the
/// system's passwd tools unable to work on a root of the tests' own.
const PASSWD_TOOLS: &str = "OXYOS_ACCOUNT_PASSWD_TOOLS";

/// Whether the system's `useradd`, `chpasswd` and `userdel` can make, verify and remove a user on a
/// root of the tests' own without privileges, as Debian trixie's (shadow 4.17) do; the tests that
/// run them ask first, and are skipped, saying why, where they cannot. Ubuntu 24.04's (shadow
/// 4.13) cannot: its `chpasswd` has no `--prefix`, its `useradd` opens the host's lastlog despite
/// one, and its `userdel` keeps a home it could not give the user. A `chpasswd` without
/// `--prefix` is found before `useradd` runs at all, so a probe never reaches the host's files.
pub(crate) fn passwd_tools() -> bool {
    static PROBE: OnceLock<Result<(), String>> = OnceLock::new();
    match PROBE.get_or_init(probe_passwd_tools) {
        Ok(()) => true,
        Err(why) if std::env::var(PASSWD_TOOLS).as_deref() == Ok("required") => {
            panic!("{PASSWD_TOOLS}=required, and the passwd tools cannot run on a root of the tests' own: {why}")
        }
        Err(why) => {
            eprintln!("skipped: the passwd tools cannot run on a root of the tests' own: {why}");
            false
        }
    }
}

fn probe_passwd_tools() -> Result<(), String> {
    let users = crate::users::SystemUsers::new();
    let help = Command::new(&users.chpasswd)
        .arg("--help")
        .output()
        .map_err(|error| format!("chpasswd: {error}"))?;
    let help = String::from_utf8_lossy(&help.stdout) + String::from_utf8_lossy(&help.stderr);
    if !help.contains("--prefix") {
        return Err("chpasswd has no --prefix".into());
    }
    let root = Root::new("probe");
    let prefix = |tool: &Path| {
        let mut command = Command::new(tool);
        command.env_clear().arg("--prefix").arg(root.path());
        command
    };
    let name = "oxy-probe";
    let ran = |what: &str, output: std::io::Result<std::process::Output>| {
        let output = output.map_err(|error| format!("{what}: {error}"))?;
        let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if !output.status.success() {
            return Err(format!("{what} failed ({}): {said}", output.status));
        }
        Ok(said)
    };
    let said = ran(
        "useradd",
        prefix(&users.useradd)
            .args(["--create-home", "--user-group", "--", name])
            .output(),
    )?;
    if said.contains("lastlog") || said.contains("faillog") {
        return Err(format!("useradd reaches outside its --prefix: {said}"));
    }
    let mut chpasswd = prefix(&users.chpasswd)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("chpasswd: {error}"))?;
    chpasswd
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{name}:probe password\n").as_bytes())
        .map_err(|error| format!("chpasswd: {error}"))?;
    ran("chpasswd", chpasswd.wait_with_output())?;
    ran(
        "userdel",
        prefix(&users.userdel)
            .args(["--remove", "--", name])
            .output(),
    )?;
    if root.path().join("home").join(name).exists() {
        return Err("userdel left the home".into());
    }
    Ok(())
}

/// Whether `password` is the one `hash` verifies, by the system's crypt(3) (libxcrypt), as
/// pam_unix checks it.
pub(crate) fn crypt_verifies(password: &str, hash: &str) -> bool {
    let script = "import ctypes, sys\n\
        lib = ctypes.CDLL('libcrypt.so.1')\n\
        lib.crypt.restype = ctypes.c_char_p\n\
        lib.crypt.argtypes = [ctypes.c_char_p, ctypes.c_char_p]\n\
        password, setting = sys.stdin.buffer.read().split(b'\\n', 1)\n\
        out = lib.crypt(password, setting)\n\
        sys.exit(0 if out is not None and out == setting else 1)\n";
    let mut child = Command::new("python3")
        .args(["-I", "-c", script])
        .stdin(Stdio::piped())
        .spawn()
        .expect("python3 runs crypt(3)");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{password}\n{hash}").as_bytes())
        .unwrap();
    child.wait().unwrap().success()
}
