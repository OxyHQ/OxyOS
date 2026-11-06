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

#[cfg(feature = "http")]
mod http;
#[cfg(feature = "http")]
pub use http::HttpApi;

/// The SDK's device credential (Oxy ADR 0029): the device's id and a holder secret for it.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCredential {
    pub device_id: String,
    pub device_secret: Secret,
}

/// The account as Oxy describes it to its own holder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar: Option<String>,
}

/// A sign-in Oxy completed: who signed in, on which device, and the holder secret it issued (the
/// platform credential, when the sign-in named the platform's client). Its own access token is
/// never read.
#[derive(Debug, PartialEq, Eq)]
pub struct SignedIn {
    pub user_id: String,
    pub device_id: String,
    pub device_secret: Option<Secret>,
}

/// OxyOS's public client, named by the platform's own sign-ins (`platformClient`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformClient {
    pub client_id: String,
}

impl Serialize for PlatformClient {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut client = serializer.serialize_struct("PlatformClient", 2)?;
        client.serialize_field("platform", PLATFORM)?;
        client.serialize_field("clientId", &self.client_id)?;
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
    /// Too many attempts: try again after this many seconds, if Oxy said.
    Locked { retry_after: Option<u64> },
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
            } => {
                write!(f, "too many attempts: try again in {seconds} s")
            }
            ApiError::Locked { retry_after: None } => f.write_str("too many attempts"),
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
}

/// Oxy's API over HTTPS: `https://api.oxy.so`.
pub const OXY_API: &str = "https://api.oxy.so";
