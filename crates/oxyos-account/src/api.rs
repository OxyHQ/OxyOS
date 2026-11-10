//! The calls to Oxy's API setting up an account needs, with the SDK's own contracts
//! (`@oxy.so/contracts`; the routes are the API's `routes/sessionDevice.ts`, `users.ts` and
//! `signIn.ts`):
//!
//! - `POST /session/device/token {deviceId, deviceSecret, accountId}`: a short access token for
//!   the account, pinned so the device's active account is never followed or changed. It proves
//!   the device credential is live and the account is on that device.
//! - `GET /users/me` with that token: the account's username, name and picture.
//! - `POST /auth/signin/password {identifier, password, device}`: the password checked by Oxy, on
//!   the same device (the proof puts the session there, not on a device of its own). An account
//!   with an authenticator answers with a challenge instead of a session:
//! - `POST /auth/signin/second-factor {challengeId, code, device}`.
//!
//! - `POST /session/device/platform {device, platformClient, redemptionKey}`: the QR's device proof
//!   redeemed for the platform credential (ADR 0058 R6), repeatable with the same key (R7);
//! - `POST /session/device/platform/confirm {deviceId, deviceSecret}`: a platform credential
//!   confirmed once the machine holds it (R8).
//!
//! A sign-in of the platform's own (Oxy ADR 0058 2.1, as accepted with OxyOS's refinements) names
//! OxyOS's public client (`platformClient: {platform: "oxyos", clientId}`), sent with no web
//! `Origin`: its answer's holder secret is then the **platform credential**, and the device's proof
//! it carried is retired by Oxy in the same transaction. The service keeps that credential, and
//! every later online sign-in with it rotates it so.
//!
//! The platform's own calls (ADR 0058 2.2, R4):
//!
//! - `POST /session/device/app-token {deviceId, deviceSecret, accountId, attestation}`: a short
//!   token for one application, named by its desktop entry id or, for OxyOS itself, `self`;
//! - `POST /session/device/signout {all: true}` with the device's own token: the device's session
//!   ended online (removing an account).
//!
//! [`OxyApi`] is the seam the setup is tested through; [`HttpApi`] is the real one.

use crate::secret::Secret;
use crate::AccountId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{Duration, Instant};

#[cfg(feature = "http")]
pub(crate) mod http;
#[cfg(feature = "http")]
pub use http::{HttpApi, OXY_CLOUD};
// The messages, the protocol's own crate's (`oxyos-account-proto`): clients link them alone.
pub use oxyos_account_proto::api::Profile;

/// The SDK's device credential (Oxy ADR 0029): the device's id and a holder secret for it.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCredential {
    pub device_id: String,
    pub device_secret: Secret,
}

/// The key a redemption of the device's proof is made with (Oxy ADR 0058 R7): 256 random bits,
/// made by the service for one setup and kept with it, sent with every redemption of that setup's
/// proof and never to anyone else. The setup client handed the proof over, and its JavaScript saw
/// it; it never sees the key, so a proof alone redeems nothing a second time.
pub struct RedemptionKey(Secret);

impl RedemptionKey {
    /// A new key from the kernel's random numbers, as 64 hexadecimal digits.
    pub fn new() -> std::io::Result<Self> {
        let mut bytes = zeroize::Zeroizing::new([0u8; 32]);
        let mut filled = 0;
        while filled < bytes.len() {
            filled += rustix::rand::getrandom(
                &mut bytes[filled..],
                rustix::rand::GetRandomFlags::empty(),
            )?;
        }
        let mut text = zeroize::Zeroizing::new(String::with_capacity(64));
        for byte in bytes.iter() {
            text.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
            text.push(char::from(b"0123456789abcdef"[usize::from(byte & 0x0f)]));
        }
        Ok(Self(Secret::new(text.as_str())))
    }

    /// The key itself, for the one request body that carries it.
    pub fn expose(&self) -> &str {
        self.0.expose()
    }
}

impl fmt::Debug for RedemptionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RedemptionKey(…)")
    }
}

impl Serialize for RedemptionKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

/// The most of an account's picture that is kept: a thumbnail is a few kilobytes; an avatar
/// without one may be a photograph.
pub const MAX_PICTURE_BYTES: u64 = 4 * 1024 * 1024;

/// `avatar` when it is an Oxy file id the machine can name a picture by (a uuid or a legacy hex
/// id: letters, digits, `-` and `_`, at most 128); anything else is no picture.
pub fn avatar_id(avatar: &str) -> Option<&str> {
    (!avatar.is_empty()
        && avatar.len() <= 128
        && avatar
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
    .then_some(avatar)
}

/// What kind of image `bytes` are, by their signature: PNG, JPEG, WebP or GIF, the images the
/// Bloom host decodes (`oxyos-image`); none for anything else, which is never kept.
pub fn picture_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else {
        None
    }
}

/// A sign-in Oxy completed: who signed in, on which device, and the holder secret it issued (the
/// platform credential, when the sign-in named the platform's client). Its own access token is
/// never read.
#[derive(Debug, PartialEq, Eq)]
pub struct SignedIn {
    pub user_id: String,
    pub device_id: String,
    pub device_secret: Option<Secret>,
    /// The windows the QR's redemption told (none for another sign-in).
    pub windows: Windows,
}

/// When the QR's windows end, as Oxy told them (ADR 0058 R9) and this machine measures them: on
/// its own monotonic clock, from when the answer arrived, the seconds Oxy counted less
/// [`WINDOW_MARGIN`] for the way back. Neither machine's clock setting enters into it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Windows {
    /// The claim's: until then the proof may be redeemed, and redeemed again with its key.
    pub redemption: Option<Instant>,
    /// The pending credential's confirmation's (R8).
    pub confirm: Option<Instant>,
}

/// What a window is shortened by for the answer's way back.
pub const WINDOW_MARGIN: Duration = Duration::from_secs(2);

/// A window's end: `seconds` from `arrived`, less [`WINDOW_MARGIN`] (never before `arrived`).
pub fn window_end(arrived: Instant, seconds: u64) -> Instant {
    arrived + Duration::from_secs(seconds).saturating_sub(WINDOW_MARGIN)
}

/// OxyOS's public client, named by the platform's own sign-ins (`platformClient`). It says that
/// it confirms each platform credential it is issued (`confirms: true`, Oxy ADR 0058 R8): the
/// credential is pending, and mints nothing, until [`OxyApi::confirm_platform_credential`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformClient {
    pub client_id: String,
}

impl Serialize for PlatformClient {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut client = serializer.serialize_struct("PlatformClient", 3)?;
        client.serialize_field("platform", PLATFORM)?;
        client.serialize_field("clientId", &self.client_id)?;
        client.serialize_field("confirms", &true)?;
        client.end()
    }
}

/// The platform OxyOS is to Oxy (`platformHost`, the attestation's `platform`).
pub const PLATFORM: &str = "oxyos";

/// Which application a token is for, as the platform attests it (ADR 0058 2.2, R2).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Attestation {
    /// An application by its desktop entry id (`so.oxy.dropsea`): Oxy resolves the application
    /// from its registry.
    Entry(String),
    /// OxyOS itself (Settings, the shell): the platform credential's own application.
    OwnPlatform,
}

impl Serialize for Attestation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut attestation = serializer.serialize_struct("Attestation", 2)?;
        attestation.serialize_field("platform", PLATFORM)?;
        match self {
            Attestation::Entry(id) => attestation.serialize_field("desktopEntryId", id)?,
            Attestation::OwnPlatform => attestation.serialize_field("self", &true)?,
        }
        attestation.end()
    }
}

/// A token Oxy minted for one application.
#[derive(Debug, PartialEq, Eq)]
pub struct AppToken {
    pub access_token: Secret,
    /// The token's own expiry (its JWT's `exp`), ISO 8601.
    pub expires_at: String,
    pub application_id: String,
    pub account_id: String,
}

#[derive(Debug)]
pub enum SignInStep {
    SignedIn(SignedIn),
    /// The account has an authenticator: its code answers this challenge.
    SecondFactor {
        challenge: Secret,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// Oxy could not be reached: no network, no name, refused, timed out.
    Offline(String),
    /// The device credential is not one Oxy knows (`invalid_device_secret`).
    InvalidDeviceSecret,
    /// The account is not signed in on that device, or its session there ended.
    AccountNotOnDevice,
    /// The identifier or the password is not right: Oxy never says which.
    InvalidCredentials,
    /// The authenticator's code is wrong, or the challenge expired or was spent.
    SecondFactorInvalid,
    /// Too many attempts: try again after this many seconds, if Oxy said; and, once Oxy knew the
    /// QR's proof (R9), when its window ends.
    Locked {
        retry_after: Option<u64>,
        window: Option<Instant>,
    },
    /// The platform does not name an application Oxy recognises for it
    /// (`application_not_recognised`).
    NotRecognised,
    /// Any other refusal.
    Refused { status: u16, code: Option<String> },
    /// An answer that is not the contract's, or a connection that is not to be trusted (TLS).
    Unexpected(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Offline(why) => write!(f, "Oxy cannot be reached: {why}"),
            ApiError::InvalidDeviceSecret => f.write_str("Oxy does not know the device credential"),
            ApiError::AccountNotOnDevice => {
                f.write_str("the account is not signed in on the device")
            }
            ApiError::InvalidCredentials => f.write_str("the username or password is not right"),
            ApiError::SecondFactorInvalid => {
                f.write_str("the authenticator's code is not right, or the sign-in expired")
            }
            ApiError::Locked {
                retry_after: Some(seconds),
                ..
            } => {
                write!(f, "too many attempts: try again in {seconds} s")
            }
            ApiError::Locked {
                retry_after: None, ..
            } => f.write_str("too many attempts"),
            ApiError::NotRecognised => f.write_str("Oxy does not recognise the application"),
            ApiError::Refused { status, code } => {
                write!(
                    f,
                    "Oxy refused ({status}, {})",
                    code.as_deref().unwrap_or("no code")
                )
            }
            ApiError::Unexpected(why) => write!(f, "an unexpected answer from Oxy: {why}"),
        }
    }
}

impl std::error::Error for ApiError {}

pub trait OxyApi {
    /// A short access token for `account`, minted with the device credential.
    fn mint_token(
        &self,
        credential: &DeviceCredential,
        account: &AccountId,
    ) -> Result<Secret, ApiError>;
    /// The account the token is for.
    fn profile(&self, token: &Secret) -> Result<Profile, ApiError>;
    /// The password checked by Oxy, the sign-in proving `device`; naming `platform`, it is the
    /// platform's own sign-in, whose holder secret is the platform credential.
    fn sign_in_password(
        &self,
        identifier: &str,
        password: &Secret,
        device: &DeviceCredential,
        platform: Option<&PlatformClient>,
    ) -> Result<SignInStep, ApiError>;
    /// The authenticator's code for a challenge, with the same device's proof and the same
    /// platform client the sign-in named.
    fn second_factor(
        &self,
        challenge: &Secret,
        code: &Secret,
        device: &DeviceCredential,
        platform: Option<&PlatformClient>,
    ) -> Result<SignedIn, ApiError>;
    /// The platform's own sign-in from the device's proof alone (Oxy ADR 0058, the setup's QR
    /// route): the holder credential a QR's claim issued the setup client, redeemed once, within
    /// ten minutes of the claim, for the platform credential, the proof retired in the same
    /// transaction. Made with `key` (R7), the same redemption may be asked again within those ten
    /// minutes with the same proof and the same key, and Oxy answers it afresh: a new platform
    /// credential, the one it issued before revoked. So an answer lost on its way (the request
    /// reached Oxy, the response never came back) is asked for again, never a second scan. Every
    /// proof that cannot be redeemed, or redeemed again with that key, is one refusal,
    /// `InvalidDeviceSecret`.
    fn platform_device_sign_in(
        &self,
        device: &DeviceCredential,
        platform: &PlatformClient,
        key: &RedemptionKey,
    ) -> Result<SignedIn, ApiError>;
    /// Confirms a platform credential a platform sign-in issued (Oxy ADR 0058 R8): until then it
    /// is pending, mints nothing and expires with the QR's ten minutes (a password's sign-in's,
    /// ten minutes after it), and the credential it replaces still works. The machine confirms it
    /// once it holds it: an answer lost on its way leaves nothing live that nobody holds. Again
    /// for a confirmed credential, the same answer. Every credential that cannot be confirmed
    /// (unknown, expired) is one refusal, `InvalidDeviceSecret`.
    fn confirm_platform_credential(&self, credential: &DeviceCredential) -> Result<(), ApiError>;
    /// A short token for one application, minted with the platform credential, pinned to
    /// `account`.
    fn mint_app_token(
        &self,
        credential: &DeviceCredential,
        account: &AccountId,
        attestation: &Attestation,
    ) -> Result<AppToken, ApiError>;
    /// Ends the device's session online, every account of it (`token` the device's own).
    fn sign_out_device(&self, token: &Secret) -> Result<(), ApiError>;
    /// The account's picture: its avatar's image (`avatar`, an Oxy file id, [`avatar_id`]) as Oxy's
    /// cloud serves it publicly, its thumbnail (else the image itself), at most
    /// [`MAX_PICTURE_BYTES`], and only a PNG, JPEG, WebP or GIF image ([`picture_kind`]).
    fn picture(&self, avatar: &str) -> Result<Vec<u8>, ApiError>;
}

/// Oxy's API over HTTPS: `https://api.oxy.so`.
pub const OXY_API: &str = "https://api.oxy.so";
