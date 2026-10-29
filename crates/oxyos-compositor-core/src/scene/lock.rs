//! The lock's password (ADR 0010): a lock surface gives it while the session is locked, the
//! session takes it to check, and the surface is told what came of it. The scene holds the
//! password only until the session takes it, in memory overwritten when it goes; it never
//! decides whether a password is right, and nothing here unlocks.

use super::{ClientId, Role, Scene, SurfaceKey};
use oxyos_protocol::{AuthOutcome, ErrorCode, Event, SecretChunk, MAX_SECRET_BYTES};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

/// A password, overwritten in memory when dropped. Its room is reserved whole when it begins,
/// so it never moves and leaves no copy behind as it grows.
pub type Secret = Zeroizing<Vec<u8>>;

#[derive(Debug, Default)]
pub(super) struct Authentication {
    /// The password each lock surface is giving, chunk by chunk.
    receiving: BTreeMap<SurfaceKey, Secret>,
    /// A password submitted and not yet taken by the session.
    submitted: Option<(SurfaceKey, Secret)>,
    /// The lock surface whose password the session is checking, until it answers.
    checking: Option<SurfaceKey>,
}

impl Scene {
    pub(super) fn lock_request(
        &mut self,
        client: ClientId,
        request: oxyos_protocol::Request,
    ) -> Result<(), ErrorCode> {
        use oxyos_protocol::Request;
        let grants = self.clients[&client];
        let key = |surface| SurfaceKey {
            client,
            id: surface,
        };
        match request {
            Request::LockSession {} => {
                if !grants.shell {
                    return Err(ErrorCode::Unauthorized);
                }
                self.lock();
                Ok(())
            }
            Request::AuthenticateSecret { surface, secret } => {
                self.lock_surface(key(surface))?;
                self.receive_secret(key(surface), secret)
            }
            Request::Authenticate { surface } => {
                self.lock_surface(key(surface))?;
                self.submit_secret(key(surface));
                Ok(())
            }
            _ => unreachable!("not a lock request"),
        }
    }

    /// `key` is one of its client's live lock surfaces.
    fn lock_surface(&self, key: SurfaceKey) -> Result<(), ErrorCode> {
        match self.live(key) {
            Some(node) if matches!(node.role, Role::Lock { .. }) => Ok(()),
            Some(_) => Err(ErrorCode::InvalidRole),
            None => Err(ErrorCode::UnknownSurface),
        }
    }

    fn receive_secret(&mut self, key: SurfaceKey, chunk: SecretChunk) -> Result<(), ErrorCode> {
        // A password given while unlocked (the lock ended as it was typed) unlocks nothing.
        if !self.locked {
            return Ok(());
        }
        let secret = self
            .authentication
            .receiving
            .entry(key)
            .or_insert_with(|| Zeroizing::new(Vec::with_capacity(MAX_SECRET_BYTES)));
        if secret.len() + chunk.as_str().len() > MAX_SECRET_BYTES {
            self.authentication.receiving.remove(&key);
            return Err(ErrorCode::QuotaExceeded);
        }
        secret.extend_from_slice(chunk.as_str().as_bytes());
        Ok(())
    }

    fn submit_secret(&mut self, key: SurfaceKey) {
        let secret = self
            .authentication
            .receiving
            .remove(&key)
            .unwrap_or_default();
        if !self.locked {
            return;
        }
        // One check at a time, whichever lock surface asked for it.
        if self.authentication.checking.is_some() {
            self.emit_result(key, AuthOutcome::Busy, 0);
            return;
        }
        self.authentication.checking = Some(key);
        self.authentication.submitted = Some((key, secret));
    }

    /// The password a lock surface submitted, for the session to check off the compositor's
    /// thread, then answer with [`Scene::authenticated`]. Until it does, others are `Busy`.
    pub fn take_authentication(&mut self) -> Option<(SurfaceKey, Secret)> {
        self.authentication.submitted.take()
    }

    /// The session's answer for the password `key` submitted. It only tells the lock surface:
    /// the session itself unlocks, with [`Scene::unlock`], when the answer is `Accepted`.
    pub fn authenticated(&mut self, key: SurfaceKey, outcome: AuthOutcome, wait_s: u32) {
        if self.authentication.checking == Some(key) {
            self.authentication.checking = None;
        }
        if self.live(key).is_some() {
            self.emit_result(key, outcome, wait_s);
        }
    }

    fn emit_result(&mut self, key: SurfaceKey, outcome: AuthOutcome, wait_s: u32) {
        self.emit(
            key.client,
            Event::AuthenticationResult {
                surface: key.id,
                outcome,
                wait_s,
            },
        );
    }

    /// Tells every client with the `lock` grant whether the session is locked.
    pub(super) fn tell_lock_state(&mut self) {
        let locked = self.locked;
        let lockers: Vec<ClientId> = self
            .clients
            .iter()
            .filter(|(_, grants)| grants.lock)
            .map(|(client, _)| *client)
            .collect();
        for client in lockers {
            self.emit(client, Event::LockState { locked });
        }
    }

    /// Passwords being given are dropped (and overwritten) with the lock, or with their
    /// surface. A password the session is checking is the session's to answer.
    pub(super) fn forget_secrets(&mut self, key: Option<SurfaceKey>) {
        match key {
            Some(key) => {
                self.authentication.receiving.remove(&key);
                if self
                    .authentication
                    .submitted
                    .as_ref()
                    .is_some_and(|(submitted, _)| *submitted == key)
                {
                    self.authentication.submitted = None;
                    self.authentication.checking = None;
                }
            }
            None => {
                self.authentication.receiving.clear();
                if self.authentication.submitted.take().is_some() {
                    self.authentication.checking = None;
                }
            }
        }
    }
}
