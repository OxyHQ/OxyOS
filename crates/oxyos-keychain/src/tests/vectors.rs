//! The Oxy SDK's cross-implementation vectors (Oxy ADR 0063 D8), reproduced byte for byte.
//!
//! `tests/vectors/oxy-keychain-v1.json` is a verbatim copy of
//! `packages/core/src/crypto/keychain/__tests__/vectors.json` in OxyHQ/oxy at commit
//! `fd06e237b9f56ac5e1035576c33ccad336ef10dd` (branch `feat/keychain-crypto`), SHA-256
//! `0dae7e6355cdcb056b05bd5eebb81842806f1e6797ef2b61190c1e807ba9044f`. When the SDK changes it,
//! copy it again and say where from here.
//!
//! Every value is rebuilt from its inputs here (ephemeral keys and nonces through the crate's
//! test hooks), compared byte for byte, and opened back; every rejection is refused for the
//! SDK's reason.

use crate::enroll;
use crate::item::{Item, SealedItem};
use crate::keyring::Keyring;
use crate::keys::KeyPairs;
use crate::recovery::RecoveryKey;
use crate::roster::{Entry, EntryBody, Head, Member, Principal, VerifiedRoster};
use crate::seal::{self, Sealed};
use crate::totp::{Algorithm, Totp};
use crate::wire::{b64, hex, is_box_key, unb64, MemberId, VaultId};
use crate::wrap::{self, Wrap};
use crate::Error;
use serde_json::Value;
use sha2::{Digest, Sha256};

const VECTORS: &str = include_str!("../../tests/vectors/oxy-keychain-v1.json");

fn vectors() -> Value {
    serde_json::from_str(VECTORS).expect("the vectors are JSON")
}

fn s(value: &Value) -> &str {
    value.as_str().expect("a string")
}

fn d(value: &Value) -> Vec<u8> {
    unb64(s(value), "vector").expect("base64url")
}

fn d32(value: &Value) -> [u8; 32] {
    d(value).try_into().expect("32 bytes")
}

fn d24(value: &Value) -> [u8; 24] {
    d(value).try_into().expect("24 bytes")
}

/// The JSON of a value as the SDK's `canonicalize` writes it, through the canonicalizer crate.
fn jcs(value: &Value) -> String {
    serde_json_canonicalizer::to_string(value).unwrap()
}

fn vault_id(v: &Value) -> VaultId {
    VaultId::parse(s(&v["vaultId"])).unwrap()
}

/// Every key pair the vectors name: the devices', by member id, and the recovery keys'.
fn signers(v: &Value) -> Vec<(Principal, KeyPairs)> {
    let mut all = Vec::new();
    for device in v["devices"].as_object().unwrap().values() {
        let id = MemberId::parse(s(&device["member"]["id"])).unwrap();
        let keys = KeyPairs::from_secrets(
            &d32(&device["signingSecretKey"]),
            &d32(&device["boxSecretKey"]),
        );
        all.push((Principal::Member(id), keys));
    }
    for recovery in v["recoveryKeys"].as_array().unwrap() {
        let keys = KeyPairs::from_secrets(
            &d32(&recovery["signingSecretKey"]),
            &d32(&recovery["boxSecretKey"]),
        );
        all.push((Principal::Recovery, keys));
    }
    all
}

fn entries(value: &Value) -> Vec<Entry> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| Entry {
            body: s(&entry["body"]).to_owned(),
            signatures: serde_json::from_value(entry["signatures"].clone()).unwrap(),
        })
        .collect()
}

fn verified_roster(v: &Value) -> VerifiedRoster {
    VerifiedRoster::verify(&vault_id(v), &entries(&v["roster"]["entries"]), None).unwrap()
}

#[test]
fn the_file_is_the_sdks_version_1() {
    let v = vectors();
    assert_eq!(v["version"], 1);
    assert_eq!(
        hex(&Sha256::digest(VECTORS.as_bytes())),
        "0dae7e6355cdcb056b05bd5eebb81842806f1e6797ef2b61190c1e807ba9044f"
    );
}

#[test]
fn device_keys() {
    let v = vectors();
    for device in v["devices"].as_object().unwrap().values() {
        let keys = KeyPairs::from_secrets(
            &d32(&device["signingSecretKey"]),
            &d32(&device["boxSecretKey"]),
        );
        assert_eq!(b64(&keys.signing_key()), s(&device["signingPublicKey"]));
        assert_eq!(b64(&keys.box_key()), s(&device["boxPublicKey"]));
        let member: Member = serde_json::from_value(device["member"].clone()).unwrap();
        assert_eq!(member.signing_key, keys.signing_key());
        assert_eq!(member.box_key, keys.box_key());
        // The member as the roster and the enrollment write it.
        assert_eq!(
            serde_json_canonicalizer::to_string(&member).unwrap(),
            jcs(&device["member"])
        );
    }
}

#[test]
fn recovery_keys() {
    let v = vectors();
    for vector in v["recoveryKeys"].as_array().unwrap() {
        let bytes: [u8; 20] = d(&vector["recoveryKey"]).try_into().unwrap();
        let key = RecoveryKey::from_bytes(&bytes);
        assert_eq!(*key.to_text(), s(&vector["text"]));
        for typed in vector["parsesFrom"].as_array().unwrap() {
            let parsed = RecoveryKey::parse(s(typed)).unwrap_or_else(|_| panic!("{typed}"));
            assert!(parsed.matches(&key), "{typed}");
        }
        let keys = key.keys().unwrap();
        let (signing, boxing) = keys.secrets();
        assert_eq!(b64(&*signing), s(&vector["signingSecretKey"]));
        assert_eq!(b64(&*boxing), s(&vector["boxSecretKey"]));
        assert_eq!(b64(&keys.signing_key()), s(&vector["signingPublicKey"]));
        assert_eq!(b64(&keys.box_key()), s(&vector["boxPublicKey"]));
        let public = key.public().unwrap();
        assert_eq!(b64(&public.signing_key), s(&vector["signingPublicKey"]));
        assert_eq!(b64(&public.box_key), s(&vector["boxPublicKey"]));
    }
    for text in v["recoveryKeyRejections"].as_array().unwrap() {
        assert!(RecoveryKey::parse(s(text)).is_err(), "{text}");
    }
}

#[test]
fn seal() {
    let v = vectors();
    for vector in v["seal"].as_array().unwrap() {
        let recipient = d32(&vector["recipientBoxPublicKey"]);
        let sealed = seal::seal_with(
            &d32(&vector["ephemeralSecretKey"]),
            &d24(&vector["nonce"]),
            &recipient,
            &d(&vector["plaintext"]),
            &d(&vector["aad"]),
        )
        .unwrap();
        let expected = &vector["sealed"];
        assert_eq!(b64(&sealed.epk), s(&expected["epk"]));
        assert_eq!(b64(&sealed.nonce), s(&expected["nonce"]));
        assert_eq!(b64(&sealed.ciphertext), s(&expected["ciphertext"]));
        let secret = KeyPairs::from_secrets(&[1; 32], &d32(&vector["recipientBoxSecretKey"]));
        assert_eq!(secret.box_key(), recipient);
        let given = Sealed {
            epk: d32(&expected["epk"]),
            nonce: d24(&expected["nonce"]),
            ciphertext: d(&expected["ciphertext"]),
        };
        let opened = seal::open(secret.box_secret(), &given, &d(&vector["aad"])).unwrap();
        assert_eq!(b64(&opened), s(&vector["plaintext"]));
    }
    for key in v["sealRejectedBoxKeys"].as_array().unwrap() {
        let key = d32(key);
        assert!(
            seal::seal_with(&[7; 32], &[0; 24], &key, &[0], &[]).is_err(),
            "{}",
            b64(&key)
        );
        assert!(!is_box_key(&key));
    }
}

#[test]
fn keyrings() {
    let v = vectors();
    for vector in v["keyrings"].as_array().unwrap() {
        let generations = vector["generations"].as_object().unwrap();
        let keys: Vec<[u8; 32]> = (1..=generations.len())
            .map(|g| d32(&generations[&g.to_string()]))
            .collect();
        let keyring = Keyring::from_keys(&keys);
        let plaintext = keyring.to_plaintext().unwrap();
        assert_eq!(
            std::str::from_utf8(&plaintext).unwrap(),
            s(&vector["plaintext"])
        );
        assert_eq!(
            jcs(&serde_json::json!({ "generations": generations })),
            s(&vector["plaintext"])
        );
        let read = Keyring::from_plaintext(s(&vector["plaintext"]).as_bytes()).unwrap();
        assert_eq!(read.generation() as usize, keys.len());
        for (g, key) in keys.iter().enumerate() {
            assert_eq!(read.get(g as u32 + 1).unwrap().bytes(), key);
        }
    }
}

#[test]
fn roster_entries() {
    let v = vectors();
    let all = signers(&v);
    for (i, vector) in v["roster"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let body = s(&vector["body"]);
        assert_eq!(
            format!("oxy.keychain.roster.v1\n{body}"),
            s(&vector["signingInput"])
        );
        let parsed = EntryBody::parse(body).unwrap();
        assert_eq!(parsed.seq, i as u64);
        // The body rebuilt from its parse, through this crate's own JCS, and through the
        // canonicalizer from a plain JSON value: the same bytes as the SDK's.
        assert_eq!(parsed.to_jcs().unwrap(), body);
        assert_eq!(jcs(&serde_json::from_str(body).unwrap()), body);
        let entry = Entry {
            body: body.to_owned(),
            signatures: serde_json::from_value(vector["signatures"].clone()).unwrap(),
        };
        assert_eq!(entry.hash(), s(&vector["hash"]));
        // Each signature is reproduced (Ed25519 is deterministic) by its signer's key.
        let signed_by: Vec<(Principal, &KeyPairs)> = entry
            .signatures
            .iter()
            .map(|signature| {
                let keys = all
                    .iter()
                    .filter(|(who, keys)| {
                        *who == signature.signer
                            && crate::keys::verify(
                                &keys.signing_key(),
                                s(&vector["signingInput"]).as_bytes(),
                                &signature.signature,
                            )
                    })
                    .map(|(_, keys)| keys)
                    .collect::<Vec<_>>();
                assert_eq!(keys.len(), 1, "entry {i}");
                (signature.signer.clone(), keys[0])
            })
            .collect();
        let rebuilt = parsed.sign(&signed_by).unwrap();
        assert_eq!(rebuilt, entry, "entry {i}");
    }
}

#[test]
fn roster_verifies_to_the_expected_state() {
    let v = vectors();
    let expected = &v["roster"]["expected"];
    let roster = verified_roster(&v);
    assert_eq!(roster.vault_id().as_str(), s(&expected["vaultId"]));
    assert_eq!(roster.genesis_hash(), s(&expected["genesisHash"]));
    let head = roster.head();
    assert_eq!(head.seq, expected["head"]["seq"].as_u64().unwrap());
    assert_eq!(head.hash, s(&expected["head"]["hash"]));
    assert_eq!(
        roster.generation() as u64,
        expected["generation"].as_u64().unwrap()
    );
    assert_eq!(
        b64(&roster.recovery().signing_key),
        s(&expected["recovery"]["signingKey"])
    );
    assert_eq!(
        b64(&roster.recovery().box_key),
        s(&expected["recovery"]["boxKey"])
    );
    let mut members: Vec<Value> = roster
        .members()
        .map(|member| serde_json::to_value(member).unwrap())
        .collect();
    let mut want = expected["members"].as_array().unwrap().clone();
    members.sort_by_key(|member| member["id"].to_string());
    want.sort_by_key(|member| member["id"].to_string());
    assert_eq!(members, want);
    let removed: Vec<&str> = roster.removed().iter().map(MemberId::as_str).collect();
    let want: Vec<&str> = expected["removedMemberIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(s)
        .collect();
    assert_eq!(removed, want);
    // Kept and checked again: the same roster is an extension of its own head.
    assert!(VerifiedRoster::verify(
        &vault_id(&v),
        &entries(&v["roster"]["entries"]),
        Some(&head)
    )
    .is_ok());
}

#[test]
fn roster_rejections() {
    let v = vectors();
    let genesis = s(&v["roster"]["expected"]["genesisHash"]).to_owned();
    for vector in v["rosterRejections"].as_array().unwrap() {
        let name = s(&vector["name"]);
        // The SDK pins the genesis and the head apart; a head here carries its genesis, and a
        // genesis pinned alone is the head at `seq` 0.
        let known = match (&vector["knownHead"], &vector["genesisHash"]) {
            (Value::Object(head), _) => Some(Head {
                genesis_hash: genesis.clone(),
                seq: head["seq"].as_u64().unwrap(),
                hash: s(&head["hash"]).to_owned(),
            }),
            (_, Value::String(pinned)) => Some(Head {
                genesis_hash: pinned.clone(),
                seq: 0,
                hash: pinned.clone(),
            }),
            _ => None,
        };
        let entries: Vec<Entry> = serde_json::from_value(vector["entries"].clone()).expect(name);
        let error =
            VerifiedRoster::verify(&vault_id(&v), &entries, known.as_ref()).expect_err(name);
        assert_eq!(error.reason(), s(&vector["reason"]), "{name}: {error:?}");
    }
}

#[test]
fn wraps() {
    let v = vectors();
    let roster = verified_roster(&v);
    let all = signers(&v);
    let mut recipients = Vec::new();
    for vector in v["wraps"].as_array().unwrap() {
        let vault = VaultId::parse(s(&vector["vaultId"])).unwrap();
        assert_eq!(vault, *roster.vault_id());
        let expected: Wrap = serde_json::from_value(vector["wrap"].clone()).unwrap();
        // The wrap as the SDK writes it, field for field.
        assert_eq!(serde_json::to_value(&expected).unwrap(), vector["wrap"]);
        let recipient: Principal = serde_json::from_value(vector["recipient"].clone()).unwrap();
        let signer: Principal = serde_json::from_value(vector["signer"].clone()).unwrap();
        recipients.push(recipient.clone());
        assert_eq!(
            b64(&wrap::aad(&vault, &recipient, expected.generation)),
            s(&vector["aad"])
        );
        assert_eq!(
            String::from_utf8(expected.signing_input(&vault).unwrap()).unwrap(),
            s(&vector["signingInput"])
        );
        let keyring = Keyring::from_plaintext(s(&vector["keyringPlaintext"]).as_bytes()).unwrap();
        let recipient_box = match &recipient {
            Principal::Member(id) => roster.member(id).unwrap().box_key,
            Principal::Recovery => roster.recovery().box_key,
        };
        let signing = KeyPairs::from_secrets(&d32(&vector["signerSigningSecretKey"]), &[1; 32]);
        let rebuilt = Wrap::create_with(
            &d32(&vector["ephemeralSecretKey"]),
            &d24(&vector["nonce"]),
            &vault,
            recipient.clone(),
            &recipient_box,
            &keyring,
            signer,
            &signing,
        )
        .unwrap();
        assert!(
            rebuilt == expected,
            "the wrap to {recipient:?} is rebuilt byte for byte"
        );
        // Opened by its recipient, against the verified roster.
        let box_secret = d32(&vector["recipientBoxSecretKey"]);
        let keys = all
            .iter()
            .find(|(who, keys)| {
                *who == recipient && keys.secrets().1.as_slice() == box_secret.as_slice()
            })
            .map(|(_, keys)| keys)
            .expect("the recipient's keys");
        let opened = expected.open(&roster, &recipient, keys).unwrap();
        assert_eq!(
            std::str::from_utf8(&opened.to_plaintext().unwrap()).unwrap(),
            s(&vector["keyringPlaintext"])
        );
    }
    assert!(recipients.contains(&Principal::Recovery));
}

#[test]
fn enrollment() {
    let v = vectors();
    let mut zero_padded = false;
    for vector in v["enrollment"].as_array().unwrap() {
        let vault = VaultId::parse(s(&vector["vaultId"])).unwrap();
        let member: Member = serde_json::from_value(vector["member"].clone()).unwrap();
        let device = d32(&vector["deviceNonce"]);
        let approver = d32(&vector["approverNonce"]);
        assert_eq!(
            b64(&enroll::commit_input(&vault, &member, &device).unwrap()),
            s(&vector["commitInput"])
        );
        let commitment = enroll::commitment(&vault, &member, &device).unwrap();
        assert_eq!(b64(&commitment), s(&vector["commitment"]));
        assert!(enroll::opens(&commitment, &vault, &member, &device).unwrap());
        let input = enroll::code_input(&vault, &member, &approver, &device).unwrap();
        assert_eq!(b64(&input), s(&vector["codeInput"]));
        assert_eq!(hex(&Sha256::digest(&input)), s(&vector["codeHash"]));
        let code = enroll::code(&vault, &member, &approver, &device).unwrap();
        assert_eq!(*code, s(&vector["code"]));
        zero_padded |= code.starts_with('0');
    }
    assert!(
        zero_padded,
        "a code with a leading zero is among the vectors"
    );
}

#[test]
fn items() {
    let v = vectors();
    let generations = v["keyrings"][0]["generations"].as_object().unwrap();
    let keyring = Keyring::from_keys(&[d32(&generations["1"]), d32(&generations["2"])]);
    let mut exactly_one_block = false;
    for vector in v["items"].as_array().unwrap() {
        let vault = VaultId::parse(s(&vector["vaultId"])).unwrap();
        let id = crate::ItemId::parse(s(&vector["itemId"])).unwrap();
        let generation = vector["generation"].as_u64().unwrap() as u32;
        assert_eq!(
            keyring.get(generation).unwrap().bytes(),
            &d32(&vector["vaultKey"])
        );
        let jcs_text = s(&vector["plaintextJcs"]);
        assert_eq!(
            jcs(&vector["item"]),
            jcs_text,
            "the canonicalizer agrees with the SDK"
        );
        let padded = d(&vector["paddedPlaintext"]);
        // Read from the SDK's plaintext, then written again: the same bytes.
        let item = Item::from_plaintext(&padded).unwrap();
        assert_eq!(*item.to_plaintext().unwrap(), padded);
        assert_eq!(std::str::from_utf8(&padded).unwrap().trim_end(), jcs_text);
        exactly_one_block |= jcs_text.len() == 256 && padded.len() == 256;
        assert_eq!(
            b64(&crate::item::aad(&vault, &id, generation)),
            s(&vector["aad"])
        );
        let sealed = item
            .seal_at(&d24(&vector["nonce"]), generation, &vault, &keyring)
            .unwrap();
        let expected = SealedItem {
            generation,
            nonce: d24(&vector["nonce"]),
            ciphertext: d(&vector["ciphertext"]),
        };
        assert!(
            sealed == expected,
            "{:?} is sealed byte for byte",
            item.content.kind()
        );
        let opened = Item::open(&vault, &id, &expected, &keyring, Some(item.counter)).unwrap();
        assert_eq!(*opened.to_plaintext().unwrap(), padded);
        // A counter below one already seen: rolled back, refused.
        assert!(matches!(
            Item::open(&vault, &id, &expected, &keyring, Some(item.counter + 1)),
            Err(Error::Rollback)
        ));
    }
    assert!(
        exactly_one_block,
        "a plaintext of exactly 256 bytes gets no padding"
    );
}

#[test]
fn totp() {
    let v = vectors();
    for vector in v["totp"]["rfc6238"].as_array().unwrap() {
        let algorithm = match s(&vector["algorithm"]) {
            "SHA1" => Algorithm::Sha1,
            "SHA256" => Algorithm::Sha256,
            "SHA512" => Algorithm::Sha512,
            other => panic!("{other}"),
        };
        let totp = Totp::new(
            &d(&vector["secret"]),
            algorithm,
            vector["digits"].as_u64().unwrap() as u32,
            vector["period"].as_u64().unwrap(),
        )
        .unwrap();
        assert_eq!(
            *totp.code_at(vector["unixSeconds"].as_u64().unwrap()),
            s(&vector["code"])
        );
    }
    for vector in v["totp"]["otpauth"].as_array().unwrap() {
        let totp = Totp::parse(s(&vector["uri"])).unwrap();
        let algorithm = match totp.algorithm() {
            Algorithm::Sha1 => "SHA1",
            Algorithm::Sha256 => "SHA256",
            Algorithm::Sha512 => "SHA512",
        };
        assert_eq!(algorithm, s(&vector["algorithm"]));
        assert_eq!(totp.digits() as u64, vector["digits"].as_u64().unwrap());
        assert_eq!(totp.period(), vector["period"].as_u64().unwrap());
        assert_eq!(totp.issuer().expose(), s(&vector["issuer"]));
        assert_eq!(totp.account().expose(), s(&vector["account"]));
        assert_eq!(b64(&totp.secret), s(&vector["secret"]));
        assert_eq!(
            *totp.code_at(vector["unixSeconds"].as_u64().unwrap()),
            s(&vector["code"])
        );
    }
}
