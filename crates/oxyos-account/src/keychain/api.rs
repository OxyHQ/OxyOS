//! The keychain's routes at Oxy (`docs/identity/keychain.md`; Oxy ADR 0063), called with OxyOS's
//! own application token (`self`, ADR 0020 step 5): [`VaultApi`], the seam the service is tested
//! through, and [`HttpApi`]'s calls. Answers are read in `sendSuccess`'s envelope or bare; a
//! refusal is `{error: KEYCHAIN_…}`. Nothing here trusts an answer: `oxyos_keychain::vault`
//! checks each before it is kept.

use crate::api::http::{answer, refusal, Enveloped};
use crate::api::{ApiError, HttpApi};
use crate::secret::{self, Secret};
use oxyos_keychain::item::SealedItem;
use oxyos_keychain::roster::{Entry, Member};
use oxyos_keychain::server::{Enrollment, ItemsPage, ServerItem, State};
use oxyos_keychain::wrap::Wrap;
use oxyos_keychain::{EnrollmentId, ItemId, MemberId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroizing;

/// What a call to the keychain's routes came to, when not its answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaultError {
    /// Oxy cannot be reached.
    Offline(String),
    /// Oxy refused: its status and `KEYCHAIN_…` code.
    Refused { status: u16, code: Option<String> },
    /// An answer that is not the contract's.
    Unexpected(String),
}

impl VaultError {
    pub fn code(&self) -> Option<&str> {
        match self {
            VaultError::Refused { code, .. } => code.as_deref(),
            _ => None,
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            VaultError::Refused { status, .. } => Some(*status),
            _ => None,
        }
    }
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VaultError::Offline(why) => write!(f, "Oxy cannot be reached: {why}"),
            VaultError::Refused { status, code } => write!(
                f,
                "Oxy refused ({status}, {})",
                code.as_deref().unwrap_or("no code")
            ),
            VaultError::Unexpected(why) => write!(f, "an unexpected answer from Oxy: {why}"),
        }
    }
}

impl From<ApiError> for VaultError {
    fn from(error: ApiError) -> Self {
        match error {
            ApiError::Offline(why) => VaultError::Offline(why),
            ApiError::Refused { status, code } => VaultError::Refused { status, code },
            ApiError::Unexpected(why) => VaultError::Unexpected(why),
            other => VaultError::Refused {
                status: 0,
                code: Some(format!("{other:?}")),
            },
        }
    }
}

pub type VaultResult<T> = Result<T, VaultError>;

/// Oxy's keychain routes; `token` is OxyOS's own application token for the account.
pub trait VaultApi: Send + Sync {
    /// `GET /keychain`.
    fn state(&self, token: &Secret) -> VaultResult<State>;
    /// `POST /keychain`.
    fn create(&self, token: &Secret, genesis: &Entry, wraps: &[Wrap]) -> VaultResult<State>;
    /// `POST /keychain/roster`.
    fn append(&self, token: &Secret, entries: &[Entry], wraps: &[Wrap]) -> VaultResult<State>;
    /// `GET /keychain/wrap`: the caller's own member's.
    fn own_wrap(&self, token: &Secret) -> VaultResult<Wrap>;
    /// `GET /keychain/recovery`.
    fn recovery_wrap(&self, token: &Secret) -> VaultResult<Wrap>;
    /// `POST /keychain/enrollments`.
    fn create_enrollment(
        &self,
        token: &Secret,
        member: &Member,
        commitment: &[u8; 32],
    ) -> VaultResult<Enrollment>;
    /// `GET /keychain/enrollments`.
    fn enrollments(&self, token: &Secret) -> VaultResult<Vec<Enrollment>>;
    /// `GET /keychain/enrollments/:id`.
    fn enrollment(&self, token: &Secret, id: &EnrollmentId) -> VaultResult<Enrollment>;
    /// `POST /keychain/enrollments/:id/approver-nonce`.
    fn approver_nonce(
        &self,
        token: &Secret,
        id: &EnrollmentId,
        member: &MemberId,
        nonce: &[u8; 32],
    ) -> VaultResult<Enrollment>;
    /// `POST /keychain/enrollments/:id/reveal`.
    fn reveal(
        &self,
        token: &Secret,
        id: &EnrollmentId,
        nonce: &[u8; 32],
    ) -> VaultResult<Enrollment>;
    /// `POST /keychain/enrollments/:id/approve`.
    fn approve(
        &self,
        token: &Secret,
        id: &EnrollmentId,
        entry: &Entry,
        wrap: &Wrap,
    ) -> VaultResult<State>;
    /// `DELETE /keychain/enrollments/:id`.
    fn delete_enrollment(&self, token: &Secret, id: &EnrollmentId) -> VaultResult<()>;
    /// `GET /keychain/items?since=`.
    fn items(&self, token: &Secret, since: u64) -> VaultResult<ItemsPage>;
    /// `PUT /keychain/items/:id`.
    fn put_item(
        &self,
        token: &Secret,
        id: &ItemId,
        base: Option<u64>,
        sealed: &SealedItem,
    ) -> VaultResult<ServerItem>;
    /// `DELETE /keychain/items/:id`.
    fn delete_item(&self, token: &Secret, id: &ItemId, base: u64) -> VaultResult<ServerItem>;
}

use oxyos_keychain::wire::b64;

#[derive(Serialize)]
struct CreateBody<'a> {
    genesis: &'a Entry,
    wraps: &'a [Wrap],
}

#[derive(Serialize)]
struct AppendBody<'a> {
    entries: &'a [Entry],
    wraps: &'a [Wrap],
}

#[derive(Serialize)]
struct EnrollBody<'a> {
    member: &'a Member,
    commitment: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApproverNonceBody<'a> {
    member_id: &'a MemberId,
    approver_nonce: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevealBody {
    device_nonce: String,
}

#[derive(Serialize)]
struct ApproveBody<'a> {
    entry: &'a Entry,
    wrap: &'a Wrap,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PutBody {
    base_revision: Option<u64>,
    generation: u32,
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteBody {
    base_revision: u64,
}

#[derive(Deserialize)]
struct WrapAnswer {
    wrap: Wrap,
}

#[derive(Deserialize)]
struct EnrollmentAnswer {
    enrollment: Enrollment,
}

#[derive(Deserialize)]
struct EnrollmentsAnswer {
    enrollments: Vec<Enrollment>,
}

#[derive(Deserialize)]
struct ItemAnswer {
    item: ServerItem,
}

impl HttpApi {
    /// One call: `method` on `path` with the bearer `token` and a JSON `body` (none for a GET).
    fn keychain_call(
        &self,
        method: &str,
        path: &str,
        token: &Secret,
        body: Option<&dyn erased::Body>,
    ) -> VaultResult<crate::api::http::Answer> {
        let url = format!("{}{path}", self.base);
        let authorization = Zeroizing::new(format!("Bearer {}", token.expose()));
        let result = match (method, body) {
            ("GET", _) => self
                .agent
                .get(&url)
                .header("Accept", "application/json")
                .header("Authorization", authorization.as_str())
                .call(),
            (method, body) => {
                let bytes = match body {
                    Some(body) => body.json()?,
                    None => Zeroizing::new(b"{}".to_vec()),
                };
                let request = match method {
                    "POST" => self.agent.post(&url),
                    "PUT" => self.agent.put(&url),
                    _ => self.agent.delete(&url).force_send_body(),
                };
                request
                    .header("Accept", "application/json")
                    .header("Content-Type", "application/json")
                    .header("Authorization", authorization.as_str())
                    .send(&bytes[..])
            }
        };
        Ok(answer(result)?)
    }

    fn keychain<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        token: &Secret,
        body: Option<&dyn erased::Body>,
    ) -> VaultResult<T> {
        let answer = self.keychain_call(method, path, token, body)?;
        if !(200..300).contains(&answer.status) {
            return Err(refusal(&answer).into());
        }
        serde_json::from_slice::<Enveloped<T>>(&answer.body)
            .map(Enveloped::into_inner)
            .map_err(|error| VaultError::Unexpected(error.to_string()))
    }
}

/// A request body of any type, written into a buffer overwritten when dropped.
mod erased {
    use super::*;

    pub trait Body {
        fn json(&self) -> VaultResult<Zeroizing<Vec<u8>>>;
    }

    impl<T: Serialize> Body for T {
        fn json(&self) -> VaultResult<Zeroizing<Vec<u8>>> {
            secret::json(self).map_err(|error| VaultError::Unexpected(error.to_string()))
        }
    }
}

impl VaultApi for HttpApi {
    fn state(&self, token: &Secret) -> VaultResult<State> {
        self.keychain("GET", "/keychain", token, None)
    }

    fn create(&self, token: &Secret, genesis: &Entry, wraps: &[Wrap]) -> VaultResult<State> {
        self.keychain(
            "POST",
            "/keychain",
            token,
            Some(&CreateBody { genesis, wraps }),
        )
    }

    fn append(&self, token: &Secret, entries: &[Entry], wraps: &[Wrap]) -> VaultResult<State> {
        self.keychain(
            "POST",
            "/keychain/roster",
            token,
            Some(&AppendBody { entries, wraps }),
        )
    }

    fn own_wrap(&self, token: &Secret) -> VaultResult<Wrap> {
        self.keychain::<WrapAnswer>("GET", "/keychain/wrap", token, None)
            .map(|answer| answer.wrap)
    }

    fn recovery_wrap(&self, token: &Secret) -> VaultResult<Wrap> {
        self.keychain::<WrapAnswer>("GET", "/keychain/recovery", token, None)
            .map(|answer| answer.wrap)
    }

    fn create_enrollment(
        &self,
        token: &Secret,
        member: &Member,
        commitment: &[u8; 32],
    ) -> VaultResult<Enrollment> {
        self.keychain::<EnrollmentAnswer>(
            "POST",
            "/keychain/enrollments",
            token,
            Some(&EnrollBody {
                member,
                commitment: b64(commitment),
            }),
        )
        .map(|answer| answer.enrollment)
    }

    fn enrollments(&self, token: &Secret) -> VaultResult<Vec<Enrollment>> {
        self.keychain::<EnrollmentsAnswer>("GET", "/keychain/enrollments", token, None)
            .map(|answer| answer.enrollments)
    }

    fn enrollment(&self, token: &Secret, id: &EnrollmentId) -> VaultResult<Enrollment> {
        self.keychain::<EnrollmentAnswer>(
            "GET",
            &format!("/keychain/enrollments/{id}"),
            token,
            None,
        )
        .map(|answer| answer.enrollment)
    }

    fn approver_nonce(
        &self,
        token: &Secret,
        id: &EnrollmentId,
        member: &MemberId,
        nonce: &[u8; 32],
    ) -> VaultResult<Enrollment> {
        self.keychain::<EnrollmentAnswer>(
            "POST",
            &format!("/keychain/enrollments/{id}/approver-nonce"),
            token,
            Some(&ApproverNonceBody {
                member_id: member,
                approver_nonce: b64(nonce),
            }),
        )
        .map(|answer| answer.enrollment)
    }

    fn reveal(
        &self,
        token: &Secret,
        id: &EnrollmentId,
        nonce: &[u8; 32],
    ) -> VaultResult<Enrollment> {
        self.keychain::<EnrollmentAnswer>(
            "POST",
            &format!("/keychain/enrollments/{id}/reveal"),
            token,
            Some(&RevealBody {
                device_nonce: b64(nonce),
            }),
        )
        .map(|answer| answer.enrollment)
    }

    fn approve(
        &self,
        token: &Secret,
        id: &EnrollmentId,
        entry: &Entry,
        wrap: &Wrap,
    ) -> VaultResult<State> {
        self.keychain(
            "POST",
            &format!("/keychain/enrollments/{id}/approve"),
            token,
            Some(&ApproveBody { entry, wrap }),
        )
    }

    fn delete_enrollment(&self, token: &Secret, id: &EnrollmentId) -> VaultResult<()> {
        let answer = self.keychain_call(
            "DELETE",
            &format!("/keychain/enrollments/{id}"),
            token,
            None,
        )?;
        if !(200..300).contains(&answer.status) {
            return Err(refusal(&answer).into());
        }
        Ok(())
    }

    fn items(&self, token: &Secret, since: u64) -> VaultResult<ItemsPage> {
        self.keychain(
            "GET",
            &format!("/keychain/items?since={since}"),
            token,
            None,
        )
    }

    fn put_item(
        &self,
        token: &Secret,
        id: &ItemId,
        base: Option<u64>,
        sealed: &SealedItem,
    ) -> VaultResult<ServerItem> {
        self.keychain::<ItemAnswer>(
            "PUT",
            &format!("/keychain/items/{id}"),
            token,
            Some(&PutBody {
                base_revision: base,
                generation: sealed.generation,
                nonce: b64(&sealed.nonce),
                ciphertext: b64(&sealed.ciphertext),
            }),
        )
        .map(|answer| answer.item)
    }

    fn delete_item(&self, token: &Secret, id: &ItemId, base: u64) -> VaultResult<ServerItem> {
        self.keychain::<ItemAnswer>(
            "DELETE",
            &format!("/keychain/items/{id}"),
            token,
            Some(&DeleteBody {
                base_revision: base,
            }),
        )
        .map(|answer| answer.item)
    }
}
