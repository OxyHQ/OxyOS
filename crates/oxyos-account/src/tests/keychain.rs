//! The keychain (ADR 0050 step 2) against a fake of Oxy's keychain routes: an HTTP server of the
//! tests' own on this machine, holding what Oxy holds (`docs/identity/keychain.md`), checking
//! rosters with `oxyos-keychain`'s verifier and answering in the API's shapes. Two machines are
//! two services with stores of their own, each its own device at the fake; Settings' and the
//! registrar's requests are made as the sockets make them (`Keychains`), and on real sockets with
//! registered processes at the end.

use super::session::{self as sockets, Application};
use super::*;
use crate::api::HttpApi;
use crate::keychain::service::{KeychainService, Memory};
use crate::keychain::*;
use crate::session::{Answer, Reason, Registrar, Request};
use oxyos_keychain::roster::{Entry, EntryBody, Op, Principal, VerifiedRoster};
use oxyos_keychain::wrap::Wrap;
use oxyos_keychain::{ItemId, MemberId, VaultId};
use rustix::fd::AsFd;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

mod passkeys;

const PASSWORD_A: &str = "correct horse battery staple";
const PASSWORD_B: &str = "tr0ub4dor & three";
const AT: &str = "2026-11-07T12:00:00.000Z";

// The fake Oxy.

struct HeldItem {
    revision: u64,
    generation: u32,
    nonce: Option<String>,
    ciphertext: Option<String>,
}

struct HeldVault {
    id: VaultId,
    entries: Vec<Entry>,
    roster: VerifiedRoster,
    added: Vec<MemberId>,
    wraps: HashMap<String, Wrap>,
    items: BTreeMap<String, HeldItem>,
    revision: u64,
}

struct HeldEnrollment {
    id: String,
    device: String,
    member: Value,
    member_id: MemberId,
    commitment: String,
    approver_nonce: Option<String>,
    approver: Option<String>,
    device_nonce: Option<String>,
    status: &'static str,
}

#[derive(Default)]
struct OxyState {
    vault: Option<HeldVault>,
    /// Each device's live member.
    bound: HashMap<String, MemberId>,
    orphaned: HashSet<MemberId>,
    enrollments: Vec<HeldEnrollment>,
    /// A roster served in place of the vault's (a fork, a rewrite, a shorter one).
    served_roster: Option<Vec<Entry>>,
    /// Every page of items empty, yet saying more follow.
    empty_pages: bool,
    requests: Vec<String>,
}

/// Oxy's keychain routes, as `docs/identity/keychain.md` describes them.
#[derive(Default)]
struct Oxy {
    state: Mutex<OxyState>,
}

type Response = (u16, Value);

fn ok(data: Value) -> Response {
    (200, json!({ "data": data }))
}

fn error(status: u16, code: &str) -> Response {
    (status, json!({ "error": code, "message": code }))
}

impl Oxy {
    fn start() -> (Arc<Self>, String) {
        let oxy = Arc::new(Self::default());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let serving = oxy.clone();
        std::thread::spawn(move || loop {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let oxy = serving.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let mut length = 0;
                let mut bearer = None;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    let (key, value) = header.split_once(':').unwrap();
                    match key.trim().to_ascii_lowercase().as_str() {
                        "content-length" => length = value.trim().parse().unwrap(),
                        "authorization" => {
                            bearer = value.trim().strip_prefix("Bearer ").map(str::to_owned)
                        }
                        _ => {}
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let mut parts = line.split_whitespace();
                let (method, target) = (parts.next().unwrap(), parts.next().unwrap());
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let (status, answer) =
                    match bearer.as_deref().and_then(|b| b.strip_prefix("device:")) {
                        Some(device) => oxy.handle(method, target, device, &body),
                        None => error(401, "UNAUTHORIZED"),
                    };
                let answer = answer.to_string();
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{answer}",
                    answer.len()
                );
                let _ = (&stream).write_all(response.as_bytes());
            });
        });
        (oxy, base)
    }

    fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }

    fn orphan(&self, member: &MemberId) {
        self.state.lock().unwrap().orphaned.insert(member.clone());
    }

    fn serve_roster(&self, entries: Option<Vec<Entry>>) {
        self.state.lock().unwrap().served_roster = entries;
    }

    fn generation(&self) -> u32 {
        self.state
            .lock()
            .unwrap()
            .vault
            .as_ref()
            .unwrap()
            .roster
            .generation()
    }

    fn roster(&self) -> Vec<Entry> {
        self.state
            .lock()
            .unwrap()
            .vault
            .as_ref()
            .unwrap()
            .entries
            .clone()
    }

    fn member_of(&self, device: &str) -> Option<MemberId> {
        self.state.lock().unwrap().bound.get(device).cloned()
    }

    fn has_vault(&self) -> bool {
        self.state.lock().unwrap().vault.is_some()
    }

    /// The item's ciphertext as held, to be served again later (a rollback).
    fn item(&self, id: &str) -> (u32, String, String) {
        let state = self.state.lock().unwrap();
        let item = &state.vault.as_ref().unwrap().items[id];
        (
            item.generation,
            item.nonce.clone().unwrap(),
            item.ciphertext.clone().unwrap(),
        )
    }

    /// Serves an older ciphertext of an item at a new revision.
    fn replay(&self, id: &str, (generation, nonce, ciphertext): (u32, String, String)) {
        let mut state = self.state.lock().unwrap();
        let vault = state.vault.as_mut().unwrap();
        vault.revision += 1;
        let revision = vault.revision;
        vault.items.insert(
            id.into(),
            HeldItem {
                revision,
                generation,
                nonce: Some(nonce),
                ciphertext: Some(ciphertext),
            },
        );
    }

    fn handle(&self, method: &str, target: &str, device: &str, body: &Value) -> Response {
        let mut state = self.state.lock().unwrap();
        state.requests.push(format!("{method} {target} {device}"));
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        match (method, segments.as_slice()) {
            ("GET", ["keychain"]) => ok(state.answer(device)),
            ("POST", ["keychain"]) => state.create(device, body),
            ("POST", ["keychain", "roster"]) => state.append(device, body),
            ("GET", ["keychain", "wrap"]) => {
                let Some(me) = state.live(device) else {
                    return error(404, "KEYCHAIN_NOT_MEMBER");
                };
                let wrap = state
                    .vault
                    .as_ref()
                    .unwrap()
                    .wraps
                    .get(me.as_str())
                    .cloned();
                match wrap {
                    Some(wrap) => ok(json!({ "wrap": wrap })),
                    None => error(404, "KEYCHAIN_NOT_FOUND"),
                }
            }
            ("GET", ["keychain", "recovery"]) => match &state.vault {
                Some(vault) => ok(json!({ "wrap": vault.wraps["recovery"] })),
                None => error(404, "KEYCHAIN_NOT_FOUND"),
            },
            ("POST", ["keychain", "enrollments"]) => state.enroll(device, body),
            ("GET", ["keychain", "enrollments"]) => {
                if state.live(device).is_none() {
                    return error(404, "KEYCHAIN_NOT_MEMBER");
                }
                let pending: Vec<Value> = state
                    .enrollments
                    .iter()
                    .filter(|e| e.status == "pending")
                    .map(|e| state.enrollment_json(e))
                    .collect();
                ok(json!({ "enrollments": pending }))
            }
            ("GET", ["keychain", "enrollments", id]) => match state.find(id) {
                Some(e) => ok(json!({ "enrollment": state.enrollment_json(e) })),
                None => error(404, "KEYCHAIN_ENROLLMENT_NOT_FOUND"),
            },
            ("POST", ["keychain", "enrollments", id, "approver-nonce"]) => {
                let Some(me) = state.live(device) else {
                    return error(403, "KEYCHAIN_NOT_MEMBER");
                };
                let index = state.enrollments.iter().position(|e| e.id == *id).unwrap();
                let enrollment = &mut state.enrollments[index];
                if enrollment.approver_nonce.is_some() || body["memberId"] != me.as_str() {
                    return error(409, "KEYCHAIN_ENROLLMENT_STATE");
                }
                enrollment.approver_nonce = Some(body["approverNonce"].as_str().unwrap().into());
                enrollment.approver = Some(me.to_string());
                let e = &state.enrollments[index];
                ok(json!({ "enrollment": state.enrollment_json(e) }))
            }
            ("POST", ["keychain", "enrollments", id, "reveal"]) => {
                let index = state.enrollments.iter().position(|e| e.id == *id).unwrap();
                let enrollment = &mut state.enrollments[index];
                if enrollment.device != device
                    || enrollment.approver_nonce.is_none()
                    || enrollment.device_nonce.is_some()
                {
                    return error(409, "KEYCHAIN_ENROLLMENT_STATE");
                }
                enrollment.device_nonce = Some(body["deviceNonce"].as_str().unwrap().into());
                let e = &state.enrollments[index];
                ok(json!({ "enrollment": state.enrollment_json(e) }))
            }
            ("POST", ["keychain", "enrollments", id, "approve"]) => state.approve(device, id, body),
            ("DELETE", ["keychain", "enrollments", id]) => {
                match state.enrollments.iter_mut().find(|e| e.id == *id) {
                    Some(e) => {
                        e.status = "denied";
                        (204, Value::Null)
                    }
                    None => error(404, "KEYCHAIN_ENROLLMENT_NOT_FOUND"),
                }
            }
            ("GET", ["keychain", "items"]) => {
                if state.live(device).is_none() {
                    return error(404, "KEYCHAIN_NOT_MEMBER");
                }
                let since: u64 = query
                    .strip_prefix("since=")
                    .and_then(|since| since.parse().ok())
                    .unwrap_or(0);
                let vault = state.vault.as_ref().unwrap();
                let mut changed: Vec<(&String, &HeldItem)> = vault
                    .items
                    .iter()
                    .filter(|(_, item)| item.revision > since)
                    .collect();
                changed.sort_by_key(|(_, item)| item.revision);
                if state.empty_pages {
                    return ok(json!({ "items": [], "revision": vault.revision, "more": true }));
                }
                // Pages of two, so a sync follows `more`.
                let more = changed.len() > 2;
                let page: Vec<Value> = changed
                    .iter()
                    .take(2)
                    .map(|(id, item)| item_json(id, item))
                    .collect();
                ok(json!({ "items": page, "revision": vault.revision, "more": more }))
            }
            ("PUT", ["keychain", "items", id]) => {
                if state.live(device).is_none() {
                    return error(404, "KEYCHAIN_NOT_MEMBER");
                }
                let vault = state.vault.as_mut().unwrap();
                let held = vault.items.get(*id).map(|item| item.revision);
                if body["baseRevision"].as_u64() != held {
                    return error(409, "KEYCHAIN_ITEM_CONFLICT");
                }
                let generation = body["generation"].as_u64().unwrap() as u32;
                if generation < 1 || generation > vault.roster.generation() {
                    return error(400, "KEYCHAIN_GENERATION_INVALID");
                }
                vault.revision += 1;
                let item = HeldItem {
                    revision: vault.revision,
                    generation,
                    nonce: Some(body["nonce"].as_str().unwrap().into()),
                    ciphertext: Some(body["ciphertext"].as_str().unwrap().into()),
                };
                let answer = item_json(id, &item);
                vault.items.insert((*id).into(), item);
                ok(json!({ "item": answer }))
            }
            ("DELETE", ["keychain", "items", id]) => {
                if state.live(device).is_none() {
                    return error(404, "KEYCHAIN_NOT_MEMBER");
                }
                let vault = state.vault.as_mut().unwrap();
                let Some(held) = vault.items.get(*id).filter(|item| item.nonce.is_some()) else {
                    return error(404, "KEYCHAIN_ITEM_NOT_FOUND");
                };
                if body["baseRevision"].as_u64() != Some(held.revision) {
                    return error(409, "KEYCHAIN_ITEM_CONFLICT");
                }
                vault.revision += 1;
                let item = HeldItem {
                    revision: vault.revision,
                    generation: held.generation,
                    nonce: None,
                    ciphertext: None,
                };
                let answer = item_json(id, &item);
                vault.items.insert((*id).into(), item);
                ok(json!({ "item": answer }))
            }
            _ => error(404, "NOT_FOUND"),
        }
    }
}

fn item_json(id: &str, item: &HeldItem) -> Value {
    json!({
        "id": id,
        "revision": item.revision,
        "generation": item.generation,
        "nonce": item.nonce,
        "ciphertext": item.ciphertext,
        "deletedAt": item.nonce.is_none().then_some(AT),
        "updatedAt": AT,
    })
}

impl OxyState {
    /// The device's live member.
    fn live(&self, device: &str) -> Option<MemberId> {
        let member = self.bound.get(device)?;
        let vault = self.vault.as_ref()?;
        vault.roster.member(member).map(|_| member.clone())
    }

    fn answer(&self, device: &str) -> Value {
        let Some(vault) = &self.vault else {
            return json!({ "vault": null, "roster": [], "members": [], "self": null });
        };
        let head = vault.roster.head();
        let roster = self
            .served_roster
            .clone()
            .unwrap_or_else(|| vault.entries.clone());
        json!({
            "vault": {
                "id": vault.id,
                "genesisHash": head.genesis_hash,
                "rosterSeq": head.seq,
                "rosterHead": head.hash,
                "generation": vault.roster.generation(),
                "revision": vault.revision,
            },
            "roster": roster,
            "members": vault.added.iter().map(|id| json!({
                "id": id,
                "orphaned": self.orphaned.contains(id),
                "removed": vault.roster.member(id).is_none(),
            })).collect::<Vec<_>>(),
            "self": self.live(device),
        })
    }

    fn create(&mut self, device: &str, body: &Value) -> Response {
        if self.vault.is_some() {
            return error(409, "KEYCHAIN_EXISTS");
        }
        let genesis: Entry = serde_json::from_value(body["genesis"].clone()).unwrap();
        let wraps: Vec<Wrap> = serde_json::from_value(body["wraps"].clone()).unwrap();
        let parsed = EntryBody::parse(&genesis.body).unwrap();
        let Ok(roster) =
            VerifiedRoster::verify(&parsed.vault_id, std::slice::from_ref(&genesis), None)
        else {
            return error(400, "KEYCHAIN_ROSTER_INVALID");
        };
        let Op::Genesis { member, .. } = parsed.op else {
            return error(400, "KEYCHAIN_ROSTER_INVALID");
        };
        assert_eq!(wraps.len(), 2);
        assert!(wraps.iter().all(|wrap| wrap.generation == 1));
        self.bound.insert(device.into(), member.id.clone());
        self.vault = Some(HeldVault {
            id: parsed.vault_id,
            entries: vec![genesis],
            roster,
            added: vec![member.id.clone()],
            wraps: wraps
                .into_iter()
                .map(|wrap| (wrap.recipient.as_str().to_owned(), wrap))
                .collect(),
            items: BTreeMap::new(),
            revision: 0,
        });
        (201, json!({ "data": self.answer(device) }))
    }

    /// Appends `entries` with their `wraps`, as Oxy does: continuing the head, verified.
    fn extend(&mut self, entries: &[Entry], wraps: Vec<Wrap>) -> Result<(), Response> {
        let vault = self.vault.as_mut().unwrap();
        let first = EntryBody::parse(&entries[0].body)
            .map_err(|_| error(400, "KEYCHAIN_ROSTER_INVALID"))?;
        if first.seq != vault.roster.head().seq + 1 {
            return Err(error(409, "KEYCHAIN_ROSTER_CONFLICT"));
        }
        let roster = vault
            .roster
            .extend(entries)
            .map_err(|_| error(400, "KEYCHAIN_ROSTER_INVALID"))?;
        for entry in entries {
            match EntryBody::parse(&entry.body).unwrap().op {
                Op::Add { member } => vault.added.push(member.id),
                Op::Remove { member_id } => {
                    vault.wraps.remove(member_id.as_str());
                    self.bound.retain(|_, bound| *bound != member_id);
                }
                _ => {}
            }
        }
        if roster.generation() > vault.roster.generation() {
            // A rotation: a wrap at the new generation for every member left and the recovery key.
            let expected = roster.members().count() + 1;
            assert_eq!(wraps.len(), expected, "every member and the recovery key");
            assert!(wraps
                .iter()
                .all(|wrap| wrap.generation == roster.generation()));
        }
        for wrap in wraps {
            vault.wraps.insert(wrap.recipient.as_str().to_owned(), wrap);
        }
        vault.entries.extend(entries.iter().cloned());
        vault.roster = roster;
        Ok(())
    }

    fn append(&mut self, device: &str, body: &Value) -> Response {
        if self.vault.is_none() {
            return error(404, "KEYCHAIN_NOT_FOUND");
        }
        let entries: Vec<Entry> = serde_json::from_value(body["entries"].clone()).unwrap();
        let wraps: Vec<Wrap> = serde_json::from_value(body["wraps"].clone()).unwrap();
        let caller = self.live(device);
        let added = match EntryBody::parse(&entries[0].body).unwrap().op {
            Op::Add { member } => Some(member.id),
            _ => None,
        };
        match (&caller, &added) {
            // A member adds nobody here: a device is added by approving it.
            (Some(_), Some(_)) => return error(400, "KEYCHAIN_ROSTER_INVALID"),
            (Some(_), None) => {}
            // A device with the recovery key: one add, signed by the recovery key alone.
            (None, Some(_))
                if entries.len() == 1
                    && entries[0].signatures.len() == 1
                    && entries[0].signatures[0].signer == Principal::Recovery => {}
            (None, _) => return error(403, "KEYCHAIN_NOT_MEMBER"),
        }
        if let Err(response) = self.extend(&entries, wraps) {
            return response;
        }
        if caller.is_none() {
            self.bound.insert(device.into(), added.unwrap());
        }
        ok(self.answer(device))
    }

    fn enroll(&mut self, device: &str, body: &Value) -> Response {
        if self.vault.is_none() {
            return error(404, "KEYCHAIN_NOT_FOUND");
        }
        if self.live(device).is_some() {
            return error(409, "KEYCHAIN_ALREADY_MEMBER");
        }
        let member_id = MemberId::parse(body["member"]["id"].as_str().unwrap()).unwrap();
        self.enrollments.retain(|e| e.device != device);
        let enrollment = HeldEnrollment {
            id: ItemId::random().unwrap().to_string(),
            device: device.into(),
            member: body["member"].clone(),
            member_id,
            commitment: body["commitment"].as_str().unwrap().into(),
            approver_nonce: None,
            approver: None,
            device_nonce: None,
            status: "pending",
        };
        let answer = self.enrollment_json(&enrollment);
        self.enrollments.push(enrollment);
        (201, json!({ "data": { "enrollment": answer } }))
    }

    fn find(&self, id: &str) -> Option<&HeldEnrollment> {
        self.enrollments.iter().find(|e| e.id == id)
    }

    fn enrollment_json(&self, e: &HeldEnrollment) -> Value {
        json!({
            "id": e.id,
            "vaultId": self.vault.as_ref().unwrap().id,
            "member": e.member,
            "commitment": e.commitment,
            "approverNonce": e.approver_nonce,
            "approverMemberId": e.approver,
            "deviceNonce": e.device_nonce,
            "status": e.status,
            "expiresAt": "2026-11-07T12:10:00.000Z",
            "createdAt": AT,
        })
    }

    fn approve(&mut self, device: &str, id: &str, body: &Value) -> Response {
        let Some(me) = self.live(device) else {
            return error(403, "KEYCHAIN_NOT_MEMBER");
        };
        let index = self.enrollments.iter().position(|e| e.id == id).unwrap();
        let e = &self.enrollments[index];
        if e.approver.as_deref() != Some(me.as_str()) || e.device_nonce.is_none() {
            return error(409, "KEYCHAIN_ENROLLMENT_STATE");
        }
        let entry: Entry = serde_json::from_value(body["entry"].clone()).unwrap();
        let wrap: Wrap = serde_json::from_value(body["wrap"].clone()).unwrap();
        let (enrolling, member) = (e.device.clone(), e.member_id.clone());
        if let Err(response) = self.extend(std::slice::from_ref(&entry), vec![wrap]) {
            return response;
        }
        self.bound.insert(enrolling, member);
        self.enrollments[index].status = "approved";
        ok(self.answer(device))
    }
}

// The machines.

/// A machine with the account's keychain: its own store, its own device at the fake Oxy.
struct Machine {
    service: Arc<KeychainService<HttpApi>>,
    uid: u32,
    user: String,
    _dir: TempDir,
}

fn machine(base: &str, device: &str) -> Machine {
    let dir = TempDir::new(&format!("keychain-{device}"));
    let device = device.to_owned();
    let service = KeychainService::new(
        HttpApi::new(base).unwrap(),
        dir.path().to_owned(),
        format!("ThinkPad {device}"),
        Box::new(move |_, _| Some(Secret::new(format!("device:{device}")))),
        Memory::BestEffort,
    );
    let machine = Machine {
        service,
        uid: 4242,
        user: "oxy-test".into(),
        _dir: dir,
    };
    machine.service.session_opened(machine.uid, &machine.user);
    machine
}

impl Machine {
    fn unlock(&self, password: &str) -> KeychainState {
        self.service
            .unlock(self.uid, &self.user, &Secret::new(password))
    }

    fn lock(&self) {
        Keychains::lock(&*self.service, self.uid);
    }

    fn settings(&self, request: Request) -> Answer {
        self.service.settings(self.uid, &self.user, request)
    }

    fn registrar(&self, request: Request) -> Answer {
        self.service.registrar(self.uid, &self.user, request)
    }

    fn status(&self) -> Status {
        match self.settings(Request::KeychainStatus {}) {
            Answer::KeychainStatus(status) => status,
            other => panic!("not a status: {other:?}"),
        }
    }

    fn verify(&self, password: &str) {
        assert_eq!(
            self.settings(Request::KeychainVerify {
                password: Secret::new(password)
            }),
            Answer::KeychainVerified
        );
    }

    /// Makes the account's vault, as Settings does: the recovery key shown, then typed back.
    fn create(&self) -> String {
        let Answer::KeychainRecoveryKey { recovery_key } =
            self.settings(Request::KeychainCreate {})
        else {
            panic!("no recovery key");
        };
        let key = recovery_key.expose().to_owned();
        let answer = self.settings(Request::KeychainConfirmRecoveryKey { recovery_key });
        assert!(
            matches!(
                answer,
                Answer::KeychainStatus(Status {
                    state: KeychainState::Member,
                    ..
                })
            ),
            "{answer:?}"
        );
        key
    }

    fn put(&self, item: Value) -> String {
        let item: ItemInput = serde_json::from_value(item).unwrap();
        match self.settings(Request::KeychainPut { item }) {
            Answer::KeychainSaved { item_id } => item_id,
            other => panic!("not saved: {other:?}"),
        }
    }

    fn list(&self) -> Vec<ItemSummary> {
        match self.settings(Request::KeychainList {
            query: None,
            kind: None,
        }) {
            Answer::KeychainItems(items) => items,
            other => panic!("no list: {other:?}"),
        }
    }

    fn reveal(&self, id: &str) -> ItemView {
        match self.settings(Request::KeychainReveal { item_id: id.into() }) {
            Answer::KeychainItem(item) => item,
            other => panic!("not revealed: {other:?}"),
        }
    }

    fn sync(&self) -> Result<(), Reason> {
        self.service.sync_account(self.uid)
    }

    fn member_id(&self) -> MemberId {
        match self.settings(Request::KeychainMembers {}) {
            Answer::KeychainMembers(members) => {
                MemberId::parse(&members.iter().find(|m| m.own).unwrap().member_id).unwrap()
            }
            other => panic!("no members: {other:?}"),
        }
    }

    fn enrollment(&self, request: Request) -> EnrollmentView {
        match self.settings(request) {
            Answer::KeychainEnrollment(view) => view,
            other => panic!("not an enrollment: {other:?}"),
        }
    }
}

fn login(title: &str, username: &str, password: &str, apps: &[&str]) -> Value {
    json!({
        "type": "login",
        "title": title,
        "username": username,
        "password": password,
        "urls": [format!("https://{}.example/login", title.to_lowercase())],
        "apps": apps,
    })
}

fn refused(reason: Reason) -> Answer {
    Answer::refused(reason)
}

/// `b` joins `a`'s vault by approval, both showing the same code; the code.
fn approve(a: &Machine, b: &Machine) -> String {
    let join = || {
        b.enrollment(Request::KeychainJoin {
            method: JoinMethod::Approval,
            recovery_key: None,
        })
    };
    let asked = join();
    assert_eq!(asked.status, EnrollmentProgress::Waiting);
    let Answer::KeychainEnrollments(pending) = a.settings(Request::KeychainEnrollments {}) else {
        panic!("no enrollments");
    };
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name.as_deref(), Some("ThinkPad b"));
    let id = pending[0].enrollment_id.clone();
    let open = |confirm| Request::KeychainApprove {
        enrollment_id: id.clone(),
        confirm,
    };
    // The approver's nonce first: the device has revealed nothing yet.
    assert_eq!(
        a.enrollment(open(false)).status,
        EnrollmentProgress::Waiting
    );
    // The device reveals its own, and shows the code.
    let shown = join();
    assert_eq!(shown.status, EnrollmentProgress::Code);
    let device_code = shown.code.unwrap().expose().to_owned();
    assert_eq!(device_code.len(), 6);
    // The approver checks the commitment and shows the same code.
    let approver = a.enrollment(open(false));
    assert_eq!(approver.code.unwrap().expose(), device_code);
    // Confirmed only once the person proved themselves.
    assert_eq!(
        a.settings(open(true)),
        refused(Reason::VerificationRequired)
    );
    a.verify(PASSWORD_A);
    assert_eq!(a.settings(open(true)), Answer::KeychainApproved);
    assert_eq!(join().status, EnrollmentProgress::Approved);
    assert_eq!(b.status().state, KeychainState::Member);
    device_code
}

#[test]
fn a_vault_is_made_once_its_recovery_key_is_typed_back() {
    let (oxy, base) = Oxy::start();
    let a = machine(&base, "a");
    assert_eq!(a.status().state, KeychainState::Locked);
    assert_eq!(
        a.settings(Request::KeychainCreate {}),
        refused(Reason::Locked)
    );
    assert_eq!(a.unlock(PASSWORD_A), KeychainState::Off);
    let status = a.status();
    assert_eq!(
        (status.state, status.vault),
        (KeychainState::Off, Some(false))
    );
    let Answer::KeychainRecoveryKey { recovery_key } = a.settings(Request::KeychainCreate {})
    else {
        panic!("no recovery key")
    };
    // Eight groups of four.
    assert_eq!(recovery_key.expose().len(), 39);
    assert_eq!(recovery_key.expose().split('-').count(), 8);
    assert_eq!(a.status().state, KeychainState::Creating);
    // Not the key shown: nothing is made.
    assert_eq!(
        a.settings(Request::KeychainConfirmRecoveryKey {
            recovery_key: Secret::new("0000-0000-0000-0000-0000-0000-0000-0000")
        }),
        refused(Reason::RecoveryKeyMismatch)
    );
    assert!(!oxy.has_vault());
    // Typed back as a person types it: lowercase, without hyphens.
    let typed = recovery_key.expose().replace('-', "").to_lowercase();
    let answer = a.settings(Request::KeychainConfirmRecoveryKey {
        recovery_key: Secret::new(typed),
    });
    assert!(
        matches!(
            answer,
            Answer::KeychainStatus(Status {
                state: KeychainState::Member,
                vault: Some(true),
                ..
            })
        ),
        "{answer:?}"
    );
    assert!(oxy.has_vault());
    assert_eq!(oxy.member_of("a"), Some(a.member_id()));
    // Kept on the machine: locked and unlocked again, still a member, offline-ready.
    a.lock();
    assert_eq!(a.unlock(PASSWORD_A), KeychainState::Member);
    // A second vault is refused.
    assert_eq!(
        a.settings(Request::KeychainCreate {}),
        refused(Reason::VaultExists)
    );
}

#[test]
fn another_device_joins_by_approval_showing_the_same_code() {
    let (oxy, base) = Oxy::start();
    let (a, b) = (machine(&base, "a"), machine(&base, "b"));
    a.unlock(PASSWORD_A);
    a.create();
    a.verify(PASSWORD_A);
    let id = a.put(login(
        "Dropsea",
        "ada@oxy.so",
        "s3cret-Dropsea",
        &["so.oxy.dropsea"],
    ));
    // Locked and unlocked: the verification is gone with the lock.
    a.lock();
    a.unlock(PASSWORD_A);
    b.unlock(PASSWORD_B);
    assert_eq!(b.status().vault, Some(true));
    // A machine with a vault to join cannot make another.
    assert_eq!(
        b.settings(Request::KeychainCreate {}),
        refused(Reason::VaultExists)
    );
    approve(&a, &b);
    assert_eq!(oxy.member_of("b"), Some(b.member_id()));
    // The new member read the vault's items in its first sync.
    b.verify(PASSWORD_B);
    let listed = b.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title.expose(), "Dropsea");
    assert_eq!(listed[0].domains[0].expose(), "dropsea.example");
    let ItemFields::Login { password, .. } = b.reveal(&id).fields else {
        panic!("not a login");
    };
    assert_eq!(password.expose(), "s3cret-Dropsea");
    // Both list both members.
    let Answer::KeychainMembers(members) = a.settings(Request::KeychainMembers {}) else {
        panic!()
    };
    assert_eq!(members.len(), 2);
    assert_eq!(members.iter().filter(|m| m.own).count(), 1);
}

#[test]
fn joining_with_the_recovery_key() {
    let (oxy, base) = Oxy::start();
    let (a, b) = (machine(&base, "a"), machine(&base, "b"));
    a.unlock(PASSWORD_A);
    let key = a.create();
    a.verify(PASSWORD_A);
    a.put(login("Mention", "ada", "m3nt10n-pass", &["so.oxy.mention"]));
    b.unlock(PASSWORD_B);
    let join = |key: &str| {
        b.settings(Request::KeychainJoin {
            method: JoinMethod::RecoveryKey,
            recovery_key: Some(Secret::new(key)),
        })
    };
    assert_eq!(
        join("ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ"),
        refused(Reason::RecoveryKeyMismatch)
    );
    assert_eq!(join("not a key"), refused(Reason::RecoveryKeyMismatch));
    assert!(oxy.member_of("b").is_none());
    let answer = join(&key);
    assert!(
        matches!(
            answer,
            Answer::KeychainStatus(Status {
                state: KeychainState::Member,
                ..
            })
        ),
        "{answer:?}"
    );
    // The add is signed by the recovery key alone.
    let roster = oxy.roster();
    assert_eq!(roster.len(), 2);
    assert_eq!(roster[1].signatures[0].signer, Principal::Recovery);
    b.verify(PASSWORD_B);
    assert_eq!(b.list().len(), 1);
}

#[test]
fn removing_a_member_rotates_the_vault_key() {
    let (oxy, base) = Oxy::start();
    let (a, b) = (machine(&base, "a"), machine(&base, "b"));
    a.unlock(PASSWORD_A);
    a.create();
    b.unlock(PASSWORD_B);
    approve(&a, &b);
    let gone = b.member_id();
    // Only once verified (approving verified it: it lasts five minutes, until the lock).
    a.lock();
    a.unlock(PASSWORD_A);
    let remove = Request::KeychainRemoveMember {
        member_id: gone.to_string(),
    };
    assert_eq!(a.settings(remove), refused(Reason::VerificationRequired));
    a.verify(PASSWORD_A);
    // Itself, or no member: refused.
    assert_eq!(
        a.settings(Request::KeychainRemoveMember {
            member_id: a.member_id().to_string()
        }),
        refused(Reason::NotFound)
    );
    assert_eq!(
        a.settings(Request::KeychainRemoveMember {
            member_id: gone.to_string()
        }),
        Answer::KeychainMemberRemoved
    );
    assert_eq!(oxy.generation(), 2);
    assert!(oxy.member_of("b").is_none());
    // Written under the new generation, and readable after a restart.
    let id = a.put(login("Bank", "ada", "n3w-g3nerati0n", &[]));
    a.lock();
    a.unlock(PASSWORD_A);
    a.verify(PASSWORD_A);
    let ItemFields::Login { password, .. } = a.reveal(&id).fields else {
        panic!()
    };
    assert_eq!(password.expose(), "n3w-g3nerati0n");
    // The removed machine finds out at its next sync and forgets its copy.
    assert_eq!(b.sync(), Ok(()));
    assert_eq!(b.status().state, KeychainState::Removed);
    b.verify(PASSWORD_B);
    assert!(b.list().is_empty());
}

#[test]
fn an_orphaned_member_is_removed_by_the_next_member_that_sees_it() {
    let (oxy, base) = Oxy::start();
    let (a, b) = (machine(&base, "a"), machine(&base, "b"));
    a.unlock(PASSWORD_A);
    a.create();
    b.unlock(PASSWORD_B);
    approve(&a, &b);
    let orphan = b.member_id();
    oxy.orphan(&orphan);
    let Answer::KeychainMembers(members) = a.settings(Request::KeychainMembers {}) else {
        panic!()
    };
    assert!(members
        .iter()
        .any(|m| m.orphaned && m.member_id == orphan.to_string()));
    assert_eq!(a.sync(), Ok(()));
    assert_eq!(oxy.generation(), 2);
    let roster = oxy.roster();
    assert_eq!(roster.len(), 4, "genesis, add, remove, generation");
    assert!(oxy.member_of("b").is_none());
}

#[test]
fn a_fork_a_rewrite_or_a_shorter_roster_is_refused() {
    let (oxy, base) = Oxy::start();
    let (a, b) = (machine(&base, "a"), machine(&base, "b"));
    a.unlock(PASSWORD_A);
    a.create();
    b.unlock(PASSWORD_B);
    approve(&a, &b);
    let kept = oxy.roster();
    // A shorter roster (the add rolled back), another vault's, an entry rewritten: each refused,
    // and the head kept stays.
    let other = oxyos_keychain::vault::create("elsewhere", AT).unwrap();
    let mut rewritten = kept.clone();
    rewritten[1].signatures[0].signature[0] ^= 1;
    for served in [
        vec![kept[0].clone()],
        vec![other.genesis.clone()],
        rewritten,
    ] {
        oxy.serve_roster(Some(served));
        assert_eq!(a.sync(), Err(Reason::Oxy));
        assert_eq!(a.status().state, KeychainState::Member);
    }
    oxy.serve_roster(None);
    assert_eq!(a.sync(), Ok(()));
    let Answer::KeychainMembers(members) = a.settings(Request::KeychainMembers {}) else {
        panic!()
    };
    assert_eq!(members.len(), 2);
}

#[test]
fn items_sync_by_revision_and_a_rollback_is_refused() {
    let (oxy, base) = Oxy::start();
    let (a, b) = (machine(&base, "a"), machine(&base, "b"));
    a.unlock(PASSWORD_A);
    a.create();
    b.unlock(PASSWORD_B);
    approve(&a, &b);
    a.verify(PASSWORD_A);
    // More than a page of changes.
    let first = a.put(login("One", "ada", "first-password-1", &[]));
    let two = a.put(login("Two", "ada", "second-password-2", &[]));
    a.put(json!({"type": "note", "title": "Door", "body": "4711"}));
    a.put(json!({"type": "wifi", "ssid": "Home", "password": "wifi-passphrase", "security": "wpa2-psk"}));
    assert_eq!(b.sync(), Ok(()));
    b.verify(PASSWORD_B);
    assert_eq!(b.list().len(), 4);
    // A changes the first: its old password is kept in its history.
    let old = oxy.item(&first);
    let mut changed = login("One", "ada", "changed-password-1", &[]);
    changed["itemId"] = json!(first);
    assert_eq!(a.put(changed), first);
    let ItemFields::Login { password, .. } = a.reveal(&first).fields else {
        panic!()
    };
    assert_eq!(password.expose(), "changed-password-1");
    assert_eq!(b.sync(), Ok(()));
    // Oxy serves the old ciphertext again at a new revision: its counter went back, refused.
    oxy.replay(&first, old);
    assert_eq!(b.sync(), Ok(()));
    let ItemFields::Login { password, .. } = b.reveal(&first).fields else {
        panic!()
    };
    assert_eq!(password.expose(), "changed-password-1");
    // B deletes another; A sees it gone (and refuses the replay too).
    assert_eq!(
        b.settings(Request::KeychainDelete { item_id: two }),
        Answer::KeychainDeleted
    );
    assert_eq!(a.sync(), Ok(()));
    assert_eq!(a.list().len(), 3);
    // A page that promises more and gives nothing is refused, never asked for again forever.
    oxy.state.lock().unwrap().empty_pages = true;
    assert_eq!(a.sync(), Err(Reason::Oxy));
    oxy.state.lock().unwrap().empty_pages = false;
    // Every request carried the device's own token, never another's.
    assert!(oxy
        .requests()
        .iter()
        .all(|request| request.ends_with(" a") || request.ends_with(" b")));
}

#[test]
fn applications_are_offered_only_their_own_logins() {
    let (_oxy, base) = Oxy::start();
    let a = machine(&base, "a");
    a.unlock(PASSWORD_A);
    a.create();
    a.verify(PASSWORD_A);
    a.put(login(
        "Dropsea",
        "ada@oxy.so",
        "dropsea-pass-1",
        &["so.oxy.dropsea"],
    ));
    let mut with_code = login("Mention", "ada", "mention-pass-1", &["so.oxy.mention"]);
    with_code["totp"] = json!("otpauth://totp/Mention:ada?secret=JBSWY3DPEHPK3PXP&issuer=Mention");
    a.put(with_code);
    a.put(login("Bank", "ada", "bank-pass-1", &[]));
    let suggest = |id: &str, field| match a.registrar(Request::AutofillSuggest {
        desktop_entry_id: id.into(),
        field,
    }) {
        Answer::AutofillSuggestions(suggestions) => suggestions,
        other => panic!("{other:?}"),
    };
    let dropsea = suggest("so.oxy.dropsea", Field::Password);
    assert_eq!(dropsea.len(), 1);
    assert_eq!(dropsea[0].title.expose(), "Dropsea");
    assert_eq!(dropsea[0].username.expose(), "ada@oxy.so");
    // Never the item's id.
    assert!(a
        .list()
        .iter()
        .all(|item| item.item_id != dropsea[0].reference));
    // An application with nothing, or an item nobody's: no suggestion.
    assert!(suggest("so.oxy.notes", Field::Username).is_empty());
    assert!(suggest("so.oxy.dropsea", Field::OneTimeCode).is_empty());
    assert!(suggest("so.oxy.dropsea", Field::NewPassword).is_empty());
    let code = suggest("so.oxy.mention", Field::OneTimeCode);
    assert_eq!(code.len(), 1);
    let fill = |id: &str, reference: &str, field| {
        a.registrar(Request::AutofillFill {
            desktop_entry_id: id.into(),
            reference: reference.into(),
            field,
        })
    };
    assert_eq!(
        fill("so.oxy.dropsea", &dropsea[0].reference, Field::Password),
        Answer::AutofillCredential {
            username: Secret::new("ada@oxy.so"),
            password: Secret::new("dropsea-pass-1")
        }
    );
    // Another application asking with Dropsea's ref gets nothing.
    assert_eq!(
        fill("so.oxy.mention", &dropsea[0].reference, Field::Password),
        refused(Reason::NotFound)
    );
    let Answer::AutofillCode { code: digits } =
        fill("so.oxy.mention", &code[0].reference, Field::OneTimeCode)
    else {
        panic!()
    };
    assert_eq!(digits.expose().len(), 6);
    assert!(digits.expose().bytes().all(|b| b.is_ascii_digit()));
    let Answer::GeneratedPassword(generated) = a.registrar(Request::AutofillGenerate {}) else {
        panic!()
    };
    assert_eq!(generated.expose().len(), 23);
    // Not a desktop entry id: refused.
    assert_eq!(
        a.registrar(Request::AutofillSuggest {
            desktop_entry_id: "../etc".into(),
            field: Field::Password
        }),
        refused(Reason::InvalidRequest)
    );
    // Locked: everything answers `locked`, and the refs die with the lock.
    assert_eq!(
        a.registrar(Request::KeychainLock {}),
        Answer::KeychainLocked
    );
    for request in [
        Request::AutofillSuggest {
            desktop_entry_id: "so.oxy.dropsea".into(),
            field: Field::Password,
        },
        Request::AutofillGenerate {},
        Request::AutofillFill {
            desktop_entry_id: "so.oxy.dropsea".into(),
            reference: dropsea[0].reference.clone(),
            field: Field::Password,
        },
    ] {
        assert_eq!(a.registrar(request), refused(Reason::Locked));
    }
    assert_eq!(
        a.settings(Request::KeychainList {
            query: None,
            kind: None
        }),
        refused(Reason::Locked)
    );
    a.unlock(PASSWORD_A);
    assert_eq!(
        fill("so.oxy.dropsea", &dropsea[0].reference, Field::Password),
        refused(Reason::NotFound)
    );
    // Locked again, the verification is gone too.
    assert_eq!(
        a.settings(Request::KeychainList {
            query: None,
            kind: None
        }),
        refused(Reason::VerificationRequired)
    );
    // Settings' requests are not the registrar's, nor its the registrar's.
    assert_eq!(
        a.registrar(Request::KeychainStatus {}),
        refused(Reason::NotPermitted)
    );
    assert_eq!(
        a.settings(Request::AutofillGenerate {}),
        refused(Reason::NotPermitted)
    );
}

#[test]
fn the_audit_finds_reused_and_weak_passwords() {
    let (_oxy, base) = Oxy::start();
    let a = machine(&base, "a");
    a.unlock(PASSWORD_A);
    a.create();
    a.verify(PASSWORD_A);
    let one = a.put(login("One", "ada", "password1", &[]));
    let two = a.put(login("Two", "ada", "password1", &[]));
    a.put(login("Three", "ada", "Hq7-xZ2k-Pw9r-Lm4t", &[]));
    let Answer::KeychainAudit(audit) = a.settings(Request::KeychainAudit {}) else {
        panic!()
    };
    let mut pair = vec![one, two];
    pair.sort();
    assert_eq!(audit.reused, vec![pair.clone()]);
    assert_eq!(audit.reused_count, 2);
    let mut weak = audit.weak.clone();
    weak.sort();
    assert_eq!(weak, pair);
    // A query finds by title, username or domain; a type filters.
    let Answer::KeychainItems(found) = a.settings(Request::KeychainList {
        query: Some(Secret::new("three.EXAMPLE")),
        kind: None,
    }) else {
        panic!()
    };
    assert_eq!(found.len(), 1);
    let Answer::KeychainItems(notes) = a.settings(Request::KeychainList {
        query: None,
        kind: Some(ItemKind::Note),
    }) else {
        panic!()
    };
    assert!(notes.is_empty());
}

#[test]
fn settings_shows_codes_and_makes_strong_passwords_only_verified() {
    let (_oxy, base) = Oxy::start();
    let a = machine(&base, "a");
    a.unlock(PASSWORD_A);
    a.create();
    // Neither a code nor a strong password before the person proved themselves.
    assert_eq!(
        a.settings(Request::KeychainGenerate {}),
        refused(Reason::VerificationRequired)
    );
    a.verify(PASSWORD_A);
    let Answer::GeneratedPassword(generated) = a.settings(Request::KeychainGenerate {}) else {
        panic!("no strong password")
    };
    let generated = generated.expose();
    assert_eq!(
        generated.len(),
        23,
        "four groups of five, joined by hyphens"
    );
    assert_eq!(generated.matches('-').count(), 3);
    // RFC 6238's SHA-1 secret ("12345678901234567890" in base32).
    let uri = "otpauth://totp/Example:bea@example.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=Example";
    let mut with_code = login("Coded", "bea@example.com", "pw-one", &[]);
    with_code["totp"] = json!(uri);
    let coded = a.put(with_code);
    let plain = a.put(login("Plain", "bea@example.com", "pw-two", &[]));
    let code_item = a.put(json!({"type": "totp", "title": "Example", "uri": uri}));
    // A code that is no otpauth URI is refused, and nothing is written.
    let mut broken = login("Broken", "bea@example.com", "pw-three", &[]);
    broken["totp"] = json!("not a code");
    assert_eq!(
        a.settings(Request::KeychainPut {
            item: serde_json::from_value(broken).unwrap()
        }),
        refused(Reason::InvalidRequest)
    );
    let listed = a.list();
    let has_code = |id: &str| {
        listed
            .iter()
            .find(|item| item.item_id == id)
            .unwrap()
            .has_code
    };
    assert!(has_code(&coded) && has_code(&code_item) && !has_code(&plain));
    for id in [&coded, &code_item] {
        let Answer::KeychainCode(view) = a.settings(Request::KeychainCode {
            item_id: id.clone(),
        }) else {
            panic!("no code")
        };
        assert_eq!(view.code.expose().len(), 6);
        assert!(view.code.expose().bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(view.period, 30);
        assert!((1..=30).contains(&view.remaining));
    }
    assert_eq!(
        a.settings(Request::KeychainCode { item_id: plain }),
        refused(Reason::NotFound)
    );
    assert_eq!(
        a.settings(Request::KeychainCode {
            item_id: "not an id".into()
        }),
        refused(Reason::InvalidRequest)
    );
    // The registrar asks neither.
    assert_eq!(
        a.registrar(Request::KeychainGenerate {}),
        refused(Reason::NotPermitted)
    );
    assert_eq!(
        a.registrar(Request::KeychainCode { item_id: coded }),
        refused(Reason::NotPermitted)
    );
    // Locked, the verification is gone with everything else.
    a.lock();
    assert_eq!(
        a.settings(Request::KeychainGenerate {}),
        refused(Reason::Locked)
    );
}

#[test]
fn a_password_changed_elsewhere_asks_for_the_previous_one() {
    let (_oxy, base) = Oxy::start();
    let a = machine(&base, "a");
    a.unlock(PASSWORD_A);
    a.create();
    a.lock();
    // The machine learned a new password: the keybag is still sealed under the old one.
    assert_eq!(a.unlock(PASSWORD_B), KeychainState::NeedsPreviousPassword);
    assert_eq!(a.status().state, KeychainState::NeedsPreviousPassword);
    assert_eq!(
        a.registrar(Request::AutofillGenerate {}),
        refused(Reason::NeedsPreviousPassword)
    );
    let rekey = |previous: &str, current: &str| {
        a.settings(Request::KeychainRekey {
            previous: Secret::new(previous),
            current: Secret::new(current),
        })
    };
    assert_eq!(rekey("wrong", PASSWORD_B), refused(Reason::WrongPassword));
    // The current one must be the one the machine learned.
    assert_eq!(rekey(PASSWORD_A, "another"), refused(Reason::WrongPassword));
    let answer = rekey(PASSWORD_A, PASSWORD_B);
    assert!(
        matches!(
            answer,
            Answer::KeychainStatus(Status {
                state: KeychainState::Member,
                ..
            })
        ),
        "{answer:?}"
    );
    a.lock();
    assert_eq!(a.unlock(PASSWORD_A), KeychainState::NeedsPreviousPassword);
    // Without the previous one: it leaves, and a new keybag is made under the password it has.
    assert_eq!(a.settings(Request::KeychainLeave {}), Answer::KeychainLeft);
    assert_eq!(a.status().state, KeychainState::Off);
    a.lock();
    assert_eq!(a.unlock(PASSWORD_A), KeychainState::Off);
}

#[test]
fn wrong_verifications_are_throttled() {
    let (_oxy, base) = Oxy::start();
    let a = machine(&base, "a");
    a.unlock(PASSWORD_A);
    for _ in 0..5 {
        assert_eq!(
            a.settings(Request::KeychainVerify {
                password: Secret::new("guess")
            }),
            refused(Reason::WrongPassword)
        );
    }
    assert_eq!(
        a.settings(Request::KeychainVerify {
            password: Secret::new(PASSWORD_A)
        }),
        refused(Reason::Throttled)
    );
}

#[test]
fn the_session_ending_or_no_session_locks_it() {
    let (_oxy, base) = Oxy::start();
    let a = machine(&base, "a");
    a.unlock(PASSWORD_A);
    // Its session's last registrar connection closed: locked.
    a.service.session_closed(a.uid, 0);
    assert_eq!(a.status().state, KeychainState::Locked);
    // The login program unlocked it and no session came within the grace: locked by the sweep.
    a.unlock(PASSWORD_A);
    assert_eq!(a.status().state, KeychainState::Off);
    a.service.sweep();
    assert_eq!(
        a.status().state,
        KeychainState::Off,
        "still within its grace"
    );
    a.service.sweep_at(
        std::time::Instant::now() + crate::keychain::service::GRACE + Duration::from_secs(1),
    );
    assert_eq!(a.status().state, KeychainState::Locked);
    // A session's registrar came in time: no grace runs out.
    a.unlock(PASSWORD_A);
    a.service.session_opened(a.uid, &a.user);
    a.service.sweep_at(
        std::time::Instant::now() + crate::keychain::service::GRACE + Duration::from_secs(1),
    );
    assert_eq!(a.status().state, KeychainState::Off);
}

#[test]
fn no_secret_in_debug() {
    let (_oxy, base) = Oxy::start();
    let a = machine(&base, "a");
    a.unlock(PASSWORD_A);
    let key = a.create();
    a.verify(PASSWORD_A);
    let id = a.put(login(
        "Dropsea",
        "ada@oxy.so",
        "dropsea-pass-1",
        &["so.oxy.dropsea"],
    ));
    let mut shown = String::new();
    for answer in [
        a.settings(Request::KeychainList {
            query: None,
            kind: None,
        }),
        a.settings(Request::KeychainReveal { item_id: id }),
        a.registrar(Request::AutofillSuggest {
            desktop_entry_id: "so.oxy.dropsea".into(),
            field: Field::Password,
        }),
        a.registrar(Request::AutofillGenerate {}),
    ] {
        shown += &format!("{answer:?}");
    }
    let request = Request::KeychainVerify {
        password: Secret::new(PASSWORD_A),
    };
    shown += &format!("{request:?} {:?}", a.status());
    for secret in [
        PASSWORD_A,
        "dropsea-pass-1",
        "ada@oxy.so",
        "Dropsea",
        key.as_str(),
    ] {
        assert!(!shown.contains(secret), "{secret} in {shown}");
    }
}

#[test]
fn the_wire_format_is_the_documented_one() {
    for (request, wire) in [
        (
            Request::AutofillSuggest {
                desktop_entry_id: "so.oxy.dropsea".into(),
                field: Field::NewPassword,
            },
            json!({"autofillSuggest": {"desktopEntryId": "so.oxy.dropsea", "field": "newPassword"}}),
        ),
        (
            Request::AutofillFill {
                desktop_entry_id: "so.oxy.dropsea".into(),
                reference: "r1".into(),
                field: Field::OneTimeCode,
            },
            json!({"autofillFill": {"desktopEntryId": "so.oxy.dropsea", "ref": "r1", "field": "oneTimeCode"}}),
        ),
        (
            Request::AutofillGenerate {},
            json!({"autofillGenerate": {}}),
        ),
        (Request::KeychainLock {}, json!({"keychainLock": {}})),
        (Request::KeychainStatus {}, json!({"keychainStatus": {}})),
        (
            Request::KeychainJoin {
                method: JoinMethod::RecoveryKey,
                recovery_key: Some(Secret::new("k")),
            },
            json!({"keychainJoin": {"method": "recoveryKey", "recoveryKey": "k"}}),
        ),
        (
            Request::KeychainApprove {
                enrollment_id: "e".into(),
                confirm: true,
            },
            json!({"keychainApprove": {"enrollmentId": "e", "confirm": true}}),
        ),
        (
            Request::KeychainList {
                query: None,
                kind: Some(ItemKind::Login),
            },
            json!({"keychainList": {"type": "login"}}),
        ),
        (
            Request::KeychainGenerate {},
            json!({"keychainGenerate": {}}),
        ),
        (
            Request::KeychainCode {
                item_id: "i".into(),
            },
            json!({"keychainCode": {"itemId": "i"}}),
        ),
    ] {
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        let read: Request = serde_json::from_value(wire).unwrap();
        assert_eq!(
            serde_json::to_value(&read).unwrap(),
            serde_json::to_value(&request).unwrap()
        );
    }
    for (answer, wire) in [
        (
            Answer::AutofillSuggestions(vec![Suggestion {
                reference: "r1".into(),
                title: Secret::new("Dropsea"),
                username: Secret::new("ada"),
            }]),
            json!({"autofillSuggestions": [{"ref": "r1", "title": "Dropsea", "username": "ada"}]}),
        ),
        (
            Answer::AutofillCredential {
                username: Secret::new("ada"),
                password: Secret::new("pw"),
            },
            json!({"autofillCredential": {"username": "ada", "password": "pw"}}),
        ),
        (
            Answer::AutofillCode {
                code: Secret::new("123456"),
            },
            json!({"autofillCode": {"code": "123456"}}),
        ),
        (
            Answer::GeneratedPassword(Secret::new("abcde-fghij-klmno-pqrs1")),
            json!({"generatedPassword": "abcde-fghij-klmno-pqrs1"}),
        ),
        (Answer::KeychainLocked, json!("keychainLocked")),
        (
            Answer::KeychainCode(CodeView {
                code: Secret::new("287082"),
                period: 30,
                remaining: 12,
            }),
            json!({"keychainCode": {"code": "287082", "period": 30, "remaining": 12}}),
        ),
        (
            Answer::KeychainItems(vec![ItemSummary {
                item_id: "i".into(),
                kind: ItemKind::Login,
                title: Secret::new("Example"),
                username: Some(Secret::new("bea@example.com")),
                domains: vec![Secret::new("example.com")],
                has_code: true,
                modified_at: "2026-11-08T10:00:00.000Z".into(),
            }]),
            json!({"keychainItems": [{"itemId": "i", "type": "login", "title": "Example",
                "username": "bea@example.com", "domains": ["example.com"], "hasCode": true,
                "modifiedAt": "2026-11-08T10:00:00.000Z"}]}),
        ),
        (
            refused(Reason::Locked),
            json!({"refused": {"reason": "locked"}}),
        ),
        (
            refused(Reason::NotPermitted),
            json!({"refused": {"reason": "notPermitted"}}),
        ),
    ] {
        assert_eq!(serde_json::to_value(&answer).unwrap(), wire);
        assert_eq!(serde_json::from_value::<Answer>(wire).unwrap(), answer);
    }
    let item: ItemInput = serde_json::from_value(json!({
        "itemId": "0f8fad5b-d9cb-469f-a165-70867728950e",
        "type": "login", "title": "t", "username": "u", "password": "p"
    }))
    .unwrap();
    assert!(matches!(item.fields, ItemFields::Login { ref urls, .. } if urls.is_empty()));
}

// On the session socket, with real processes.

#[test]
fn on_the_session_socket_settings_manages_and_the_registrar_fills() {
    let (_oxy, base) = Oxy::start();
    let socket = sockets::service(FakeApi::default());
    let dir = TempDir::new("keychain-socket");
    let keychains = KeychainService::new(
        HttpApi::new(&base).unwrap(),
        dir.path().to_owned(),
        "ThinkPad".into(),
        Box::new(|_, _| Some(Secret::new("device:a"))),
        Memory::BestEffort,
    );
    socket.machine.platform.set_keychains(keychains.clone());
    let registrar = Registrar::connect(&socket.path).unwrap();
    // Locked until the session's check accepts the password.
    let client = RegistrarClient::connect(&socket.path).unwrap();
    assert_eq!(client.autofill_generate().unwrap(), Err(Reason::Locked));
    assert_eq!(
        crate::session::check(
            &socket.path,
            Secret::new(PASSWORD),
            crate::platform::Local::Accepted
        )
        .unwrap()
        .result,
        crate::platform::Verdict::Accepted
    );
    sockets::wait_for("the keychain unlocked", || {
        client.autofill_generate().unwrap().is_ok()
    });
    // Settings makes the vault and saves a login for Dropsea.
    let settings_steps = json!([
        {"send": {"keychainStatus": {}}},
        {"send": {"keychainCreate": {}}},
        "wait",
        {"send": {"keychainVerify": {"password": PASSWORD}}},
        {"send": {"keychainPut": {"item": {"type": "login", "title": "Dropsea", "username": "ada",
            "password": "dropsea-pass-1", "apps": ["so.oxy.dropsea"]}}}},
    ]);
    let mut settings = Application::start(&socket, settings_steps);
    registrar
        .register("so.oxy.settings", true, settings.pidfd().as_fd())
        .unwrap()
        .unwrap();
    settings.go();
    assert_eq!(settings.line()["keychainStatus"]["state"], "off");
    let key = settings.line()["keychainRecoveryKey"]["recoveryKey"]
        .as_str()
        .unwrap()
        .to_owned();
    // The recovery key typed back (as Settings would, on its own connection) makes the vault.
    let uid = sockets::me();
    let user = account().user_name().as_str().to_owned();
    let made = keychains.settings(
        uid,
        &user,
        Request::KeychainConfirmRecoveryKey {
            recovery_key: Secret::new(key),
        },
    );
    assert!(
        matches!(
            made,
            Answer::KeychainStatus(Status {
                state: KeychainState::Member,
                ..
            })
        ),
        "{made:?}"
    );
    settings.go();
    assert_eq!(settings.line(), json!("keychainVerified"));
    assert!(settings.line()["keychainSaved"]["itemId"].is_string());
    settings.end();
    // Any other application: refused, whether it asks Settings' requests or AutoFill's.
    let mut dropsea = Application::start(
        &socket,
        json!([
            {"send": {"keychainStatus": {}}},
            {"send": {"keychainList": {}}},
            {"send": {"autofillSuggest": {"desktopEntryId": "so.oxy.dropsea", "field": "password"}}},
        ]),
    );
    registrar
        .register("so.oxy.dropsea", false, dropsea.pidfd().as_fd())
        .unwrap()
        .unwrap();
    dropsea.go();
    for _ in 0..3 {
        assert_eq!(dropsea.line(), sockets::not_permitted());
    }
    dropsea.end();
    // Settings not registered as the system's: refused too.
    let mut pretender = Application::start(&socket, json!([{"send": {"keychainStatus": {}}}]));
    registrar
        .register("so.oxy.settings", false, pretender.pidfd().as_fd())
        .unwrap()
        .unwrap();
    pretender.go();
    assert_eq!(pretender.line(), sockets::not_permitted());
    pretender.end();
    // An unregistered process.
    let mut stranger = Application::start(&socket, json!([{"send": {"autofillGenerate": {}}}]));
    stranger.go();
    assert_eq!(stranger.line(), sockets::not_permitted());
    stranger.end();
    // The registrar fills Dropsea's field, and Dropsea's alone.
    let suggestions = client
        .autofill_suggest("so.oxy.dropsea", Field::Password)
        .unwrap()
        .unwrap();
    assert_eq!(suggestions.len(), 1);
    assert!(client
        .autofill_suggest("so.oxy.mention", Field::Password)
        .unwrap()
        .unwrap()
        .is_empty());
    assert_eq!(
        client
            .autofill_fill("so.oxy.dropsea", &suggestions[0].reference, Field::Password)
            .unwrap(),
        Ok(Fill::Credential {
            username: Secret::new("ada"),
            password: Secret::new("dropsea-pass-1")
        })
    );
    // The session locks: so does the keychain, on the registrar's own connection.
    registrar.keychain_lock().unwrap().unwrap();
    assert_eq!(
        client
            .autofill_suggest("so.oxy.dropsea", Field::Password)
            .unwrap(),
        Err(Reason::Locked)
    );
    // Unlocked by the lock's check, then the session's connections close: locked.
    crate::session::check(
        &socket.path,
        Secret::new(PASSWORD),
        crate::platform::Local::Accepted,
    )
    .unwrap();
    sockets::wait_for("the keychain unlocked", || {
        client.autofill_generate().unwrap().is_ok()
    });
    drop(client);
    drop(registrar);
    sockets::wait_for("the keychain locked with the session", || {
        matches!(
            keychains.settings(uid, &user, Request::KeychainStatus {}),
            Answer::KeychainStatus(Status {
                state: KeychainState::Locked,
                ..
            })
        )
    });
    socket.machine.platform.settle();
}

#[test]
fn only_root_unlocks_on_the_services_own_socket() {
    use crate::ipc::{serve, Enrollers, Listener};
    let socket_dir = TempDir::new("keychain-ipc");
    let path = socket_dir.path().join("socket");
    let listener = Listener::bind(&path).unwrap();
    let machine = super::platform::machine_as(FakeApi::default(), PASSWORD, Some(sockets::me()));
    let platform = machine.platform.clone();
    let (_oxy, base) = Oxy::start();
    let keychains = KeychainService::new(
        HttpApi::new(&base).unwrap(),
        socket_dir.path().join("keychains"),
        "ThinkPad".into(),
        Box::new(|_, _| None),
        Memory::BestEffort,
    );
    std::fs::create_dir(socket_dir.path().join("keychains")).unwrap();
    platform.set_keychains(keychains.clone());
    let served = std::thread::spawn(move || {
        let connection = listener.accept().unwrap();
        // Even a peer that may enroll (the login screen's user) may not unlock.
        serve(
            &connection,
            &platform,
            "http://127.0.0.1:9",
            Enrollers {
                login: Some(sockets::me()),
            },
            |_, _| {},
        )
        .unwrap();
    });
    let user = account().user_name().as_str().to_owned();
    let answer = crate::ipc::unlock_keychain(&path, &user, Secret::new(PASSWORD));
    served.join().unwrap();
    if rustix::process::geteuid().is_root() {
        return;
    }
    assert_eq!(
        answer.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(matches!(
        keychains.settings(sockets::me(), &user, Request::KeychainStatus {}),
        Answer::KeychainStatus(Status {
            state: KeychainState::Locked,
            ..
        })
    ));
}

#[test]
fn a_key_that_cannot_be_locked_in_memory_keeps_the_keychain_locked() {
    // Lowering the limit is this process's: the other tests lock memory best effort, and this one
    // runs its keychain in a child of its own.
    if std::env::var_os("OXYOS_KEYCHAIN_MEMLOCK_CHILD").is_some() {
        rustix::process::setrlimit(
            rustix::process::Resource::Memlock,
            rustix::process::Rlimit {
                current: Some(0),
                maximum: Some(0),
            },
        )
        .unwrap();
        let dir = TempDir::new("keychain-memlock");
        let service = KeychainService::new(
            HttpApi::new("http://127.0.0.1:9").unwrap(),
            dir.path().to_owned(),
            "ThinkPad".into(),
            Box::new(|_, _| None),
            Memory::Locked,
        );
        let state = service.unlock(1, "oxy-test", &Secret::new(PASSWORD_A));
        assert_eq!(state, KeychainState::Locked);
        return;
    }
    if rustix::process::geteuid().is_root() {
        eprintln!("skipped: root locks memory past any limit");
        return;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::keychain::a_key_that_cannot_be_locked_in_memory_keeps_the_keychain_locked",
            "--test-threads",
            "1",
        ])
        .env("OXYOS_KEYCHAIN_MEMLOCK_CHILD", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let _ = Duration::ZERO;
}
