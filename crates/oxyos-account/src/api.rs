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
//! [`OxyApi`] is the seam the setup is tested through; [`HttpApi`] is the real one.

use crate::secret::{self, Secret};
use crate::AccountId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;
use ureq::tls::{RootCerts, TlsConfig};
use ureq::Agent;
use zeroize::Zeroizing;

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

/// A sign-in Oxy completed: who signed in, on which device. Its own access token and holder
/// secret are not kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedIn {
    pub user_id: String,
    pub device_id: String,
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
    /// The password checked by Oxy, the sign-in proving `device`.
    fn sign_in_password(
        &self,
        identifier: &str,
        password: &Secret,
        device: &DeviceCredential,
    ) -> Result<SignInStep, ApiError>;
    /// The authenticator's code for a challenge, with the same device's proof.
    fn second_factor(
        &self,
        challenge: &Secret,
        code: &Secret,
        device: &DeviceCredential,
    ) -> Result<SignedIn, ApiError>;
}

/// Oxy's API over HTTPS: `https://api.oxy.so`.
pub const OXY_API: &str = "https://api.oxy.so";

/// The longest answer read; Oxy's are a few kilobytes.
const MAX_ANSWER: u64 = 256 * 1024;

pub struct HttpApi {
    agent: Agent,
    base: String,
}

impl HttpApi {
    /// The API at `base`: HTTPS, or plain HTTP to this machine alone (a development server).
    pub fn new(base: &str) -> Result<Self, String> {
        let url = url::Url::parse(base).map_err(|error| format!("{base}: {error}"))?;
        let loopback = match url.host() {
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            Some(url::Host::Domain(name)) => name == "localhost",
            None => false,
        };
        match url.scheme() {
            "https" => {}
            "http" if loopback => {}
            _ => return Err(format!("{base}: Oxy's API is reached over HTTPS")),
        }
        if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
            return Err(format!("{base}: a base URL has no query, fragment or user"));
        }
        let agent = Agent::config_builder()
            // A refusal is an answer: its status and code say what it was.
            .http_status_as_error(false)
            // None of these calls is redirected; one that is goes nowhere.
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent("oxyos-accountd")
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        Ok(Self {
            agent,
            base: base.trim_end_matches('/').to_owned(),
        })
    }

    fn post(&self, path: &str, body: &impl Serialize) -> Result<Answer, ApiError> {
        let body = secret::json(body).map_err(|error| ApiError::Unexpected(error.to_string()))?;
        let request = self
            .agent
            .post(format!("{}{path}", self.base))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json");
        answer(request.send(&body[..]))
    }

    fn get(&self, path: &str, token: &Secret) -> Result<Answer, ApiError> {
        let authorization = Zeroizing::new(format!("Bearer {}", token.expose()));
        let request = self
            .agent
            .get(format!("{}{path}", self.base))
            .header("Accept", "application/json")
            .header("Authorization", authorization.as_str());
        answer(request.call())
    }
}

/// An answer's status, `Retry-After` and body (overwritten when dropped: it holds tokens).
pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) retry_after: Option<u64>,
    pub(crate) body: Zeroizing<Vec<u8>>,
}

fn answer(
    result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<Answer, ApiError> {
    let mut response = result.map_err(transport)?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok());
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_ANSWER)
        .read_to_vec()
        .map_err(transport)?;
    Ok(Answer {
        status,
        retry_after,
        body: Zeroizing::new(body),
    })
}

/// What a failed request was: unreachable, or a connection that could not be trusted.
fn transport(error: ureq::Error) -> ApiError {
    match error {
        ureq::Error::Io(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::Timeout(_) => ApiError::Offline(error.to_string()),
        other => ApiError::Unexpected(other.to_string()),
    }
}

/// An error answer: `{error: code, message}` (an `ApiError`'s `toJSON`), or `{error: {code}}`.
#[derive(Deserialize)]
struct ErrorBody {
    error: Option<serde_json::Value>,
}

fn refusal(answer: &Answer) -> ApiError {
    let code = serde_json::from_slice::<ErrorBody>(&answer.body)
        .ok()
        .and_then(|body| body.error)
        .and_then(|error| match error {
            serde_json::Value::String(code) => Some(code),
            serde_json::Value::Object(fields) => fields
                .get("code")
                .and_then(|code| code.as_str())
                .map(str::to_owned),
            _ => None,
        });
    match (answer.status, code.as_deref()) {
        (429, _) | (_, Some("SIGNIN_LOCKED")) => ApiError::Locked {
            retry_after: answer.retry_after,
        },
        (401, Some("invalid_device_secret")) => ApiError::InvalidDeviceSecret,
        (401, Some("account_not_on_device" | "no_active_session")) => ApiError::AccountNotOnDevice,
        (401, Some("SIGNIN_INVALID_CREDENTIALS")) => ApiError::InvalidCredentials,
        (401, Some("SECOND_FACTOR_INVALID")) => ApiError::SecondFactorInvalid,
        (status, code) => ApiError::Refused {
            status,
            code: code.map(str::to_owned),
        },
    }
}

fn parse<'a, T: Deserialize<'a>>(answer: &'a Answer) -> Result<T, ApiError> {
    if answer.status != 200 {
        return Err(refusal(answer));
    }
    serde_json::from_slice(&answer.body).map_err(|error| ApiError::Unexpected(error.to_string()))
}

/// `sendSuccess`'s envelope, which the device mint and `/users/me` answer in.
#[derive(Deserialize)]
struct Data<T> {
    data: T,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MintRequest<'a> {
    device_id: &'a str,
    device_secret: &'a Secret,
    account_id: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Minted {
    access_token: Secret,
}

#[derive(Deserialize)]
struct Me {
    id: String,
    username: Option<String>,
    name: Option<serde_json::Value>,
    avatar: Option<String>,
}

/// The account's name: a string, or `{full}` or `{first, last}`.
fn display_name(name: Option<serde_json::Value>) -> Option<String> {
    let text = match name? {
        serde_json::Value::String(text) => text,
        serde_json::Value::Object(fields) => {
            let field = |key: &str| fields.get(key).and_then(|v| v.as_str()).map(str::trim);
            match field("full").filter(|full| !full.is_empty()) {
                Some(full) => full.to_owned(),
                None => [field("first"), field("last")]
                    .into_iter()
                    .flatten()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            }
        }
        _ => return None,
    };
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Proof<'a> {
    device_id: &'a str,
    device_secret: &'a Secret,
}

impl<'a> From<&'a DeviceCredential> for Proof<'a> {
    fn from(credential: &'a DeviceCredential) -> Self {
        Self {
            device_id: &credential.device_id,
            device_secret: &credential.device_secret,
        }
    }
}

#[derive(Serialize)]
struct PasswordRequest<'a> {
    identifier: &'a str,
    password: &'a Secret,
    device: Proof<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecondFactorRequest<'a> {
    challenge_id: &'a Secret,
    code: &'a Secret,
    device: Proof<'a>,
}

/// A sign-in's answer: a session (`LoginResult`) or a second factor's challenge
/// (`SecondFactorRequired`). The session's own token and secret are skipped, never read.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignInAnswer {
    second_factor_required: Option<bool>,
    challenge_id: Option<Secret>,
    device_id: Option<String>,
    user: Option<SignInUser>,
}

#[derive(Deserialize)]
struct SignInUser {
    id: String,
}

fn signed_in(answer: SignInAnswer) -> Result<SignedIn, ApiError> {
    match (answer.device_id, answer.user) {
        (Some(device_id), Some(user)) => Ok(SignedIn {
            user_id: user.id,
            device_id,
        }),
        _ => Err(ApiError::Unexpected(
            "a sign-in without its account or device".into(),
        )),
    }
}

impl OxyApi for HttpApi {
    fn mint_token(
        &self,
        credential: &DeviceCredential,
        account: &AccountId,
    ) -> Result<Secret, ApiError> {
        let answer = self.post(
            "/session/device/token",
            &MintRequest {
                device_id: &credential.device_id,
                device_secret: &credential.device_secret,
                account_id: account.as_str(),
            },
        )?;
        Ok(parse::<Data<Minted>>(&answer)?.data.access_token)
    }

    fn profile(&self, token: &Secret) -> Result<Profile, ApiError> {
        let answer = self.get("/users/me", token)?;
        let me = parse::<Data<Me>>(&answer)?.data;
        Ok(Profile {
            id: me.id,
            username: me.username.filter(|name| !name.trim().is_empty()),
            display_name: display_name(me.name),
            avatar: me.avatar.filter(|avatar| !avatar.is_empty()),
        })
    }

    fn sign_in_password(
        &self,
        identifier: &str,
        password: &Secret,
        device: &DeviceCredential,
    ) -> Result<SignInStep, ApiError> {
        let answer = self.post(
            "/auth/signin/password",
            &PasswordRequest {
                identifier,
                password,
                device: device.into(),
            },
        )?;
        let step: SignInAnswer = parse(&answer)?;
        if step.second_factor_required == Some(true) {
            return match step.challenge_id {
                Some(challenge) => Ok(SignInStep::SecondFactor { challenge }),
                None => Err(ApiError::Unexpected(
                    "a second factor without its challenge".into(),
                )),
            };
        }
        signed_in(step).map(SignInStep::SignedIn)
    }

    fn second_factor(
        &self,
        challenge: &Secret,
        code: &Secret,
        device: &DeviceCredential,
    ) -> Result<SignedIn, ApiError> {
        let answer = self.post(
            "/auth/signin/second-factor",
            &SecondFactorRequest {
                challenge_id: challenge,
                code,
                device: device.into(),
            },
        )?;
        signed_in(parse(&answer)?)
    }
}
