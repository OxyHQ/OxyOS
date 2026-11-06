//! [`HttpApi`]: the calls of [`OxyApi`] over HTTPS, with `ureq` and rustls verified by the
//! platform's certificates, as `oxyos-net` makes them; without cookies or redirects.

use super::*;
use crate::secret;
use std::time::Duration;
use ureq::tls::{RootCerts, TlsConfig};
use ureq::Agent;
use zeroize::Zeroizing;

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
            // Offline is found out soon: an unreachable API, not a slow one.
            .timeout_resolve(Some(Duration::from_secs(10)))
            .timeout_connect(Some(Duration::from_secs(10)))
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

    fn post_with(
        &self,
        path: &str,
        token: &Secret,
        body: &impl Serialize,
    ) -> Result<Answer, ApiError> {
        let body = secret::json(body).map_err(|error| ApiError::Unexpected(error.to_string()))?;
        let authorization = Zeroizing::new(format!("Bearer {}", token.expose()));
        let request = self
            .agent
            .post(format!("{}{path}", self.base))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("Authorization", authorization.as_str());
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
        (_, Some("invalid_device_secret")) => ApiError::InvalidDeviceSecret,
        (_, Some("account_not_on_device" | "no_active_session")) => ApiError::AccountNotOnDevice,
        (401, Some("SIGNIN_INVALID_CREDENTIALS")) => ApiError::InvalidCredentials,
        (401, Some("SECOND_FACTOR_INVALID")) => ApiError::SecondFactorInvalid,
        (_, Some("application_not_recognised")) => ApiError::NotRecognised,
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

/// An answer in `sendSuccess`'s envelope or bare: the application mint's is read either way.
#[derive(Deserialize)]
#[serde(untagged)]
enum Enveloped<T> {
    Data { data: T },
    Bare(T),
}

impl<T> Enveloped<T> {
    fn into_inner(self) -> T {
        match self {
            Enveloped::Data { data } | Enveloped::Bare(data) => data,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppTokenRequest<'a> {
    device_id: &'a str,
    device_secret: &'a Secret,
    account_id: &'a str,
    attestation: &'a Attestation,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppTokenAnswer {
    access_token: Secret,
    expires_at: String,
    application_id: String,
    account_id: String,
}

#[derive(Serialize)]
struct SignOutRequest {
    all: bool,
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
#[serde(rename_all = "camelCase")]
struct PasswordRequest<'a> {
    identifier: &'a str,
    password: &'a Secret,
    device: Proof<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform_client: Option<&'a PlatformClient>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecondFactorRequest<'a> {
    challenge_id: &'a Secret,
    code: &'a Secret,
    device: Proof<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform_client: Option<&'a PlatformClient>,
}

/// A sign-in's answer: a session (`LoginResult`) or a second factor's challenge
/// (`SecondFactorRequired`). The session's own token is skipped, never read; its holder secret is
/// read (a [`Secret`]) and kept only by a platform sign-in.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignInAnswer {
    second_factor_required: Option<bool>,
    challenge_id: Option<Secret>,
    device_id: Option<String>,
    device_secret: Option<Secret>,
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
            device_secret: answer.device_secret.filter(|secret| !secret.is_empty()),
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
        platform: Option<&PlatformClient>,
    ) -> Result<SignInStep, ApiError> {
        let answer = self.post(
            "/auth/signin/password",
            &PasswordRequest {
                identifier,
                password,
                device: device.into(),
                platform_client: platform,
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
        platform: Option<&PlatformClient>,
    ) -> Result<SignedIn, ApiError> {
        let answer = self.post(
            "/auth/signin/second-factor",
            &SecondFactorRequest {
                challenge_id: challenge,
                code,
                device: device.into(),
                platform_client: platform,
            },
        )?;
        signed_in(parse(&answer)?)
    }

    fn mint_app_token(
        &self,
        credential: &DeviceCredential,
        account: &AccountId,
        attestation: &Attestation,
    ) -> Result<AppToken, ApiError> {
        let answer = self.post(
            "/session/device/app-token",
            &AppTokenRequest {
                device_id: &credential.device_id,
                device_secret: &credential.device_secret,
                account_id: account.as_str(),
                attestation,
            },
        )?;
        let minted = parse::<Enveloped<AppTokenAnswer>>(&answer)?.into_inner();
        Ok(AppToken {
            access_token: minted.access_token,
            expires_at: minted.expires_at,
            application_id: minted.application_id,
            account_id: minted.account_id,
        })
    }

    fn sign_out_device(&self, token: &Secret) -> Result<(), ApiError> {
        let answer = self.post_with(
            "/session/device/signout",
            token,
            &SignOutRequest { all: true },
        )?;
        if answer.status != 200 {
            return Err(refusal(&answer));
        }
        Ok(())
    }
}
