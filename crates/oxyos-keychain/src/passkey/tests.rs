//! The authenticator against WebAuthn L3's and CTAP2's encodings, and its signatures verified by
//! an independent implementation (ring's ECDSA, not RustCrypto's).

use super::*;
use crate::keyring::Keyring;
use crate::tests::AT;
use crate::wire::VaultId;
use ciborium::value::Value;

/// SHA-256 of "example.com", the rpIdHash of WebAuthn L3's examples.
const EXAMPLE_COM_HASH: &str = "a379a6f6eeafb9a55e378c118034e2751e682fab9f2d30ab13d2125586ce1947";
/// SubjectPublicKeyInfo's prefix for an uncompressed P-256 key (RFC 5480): id-ecPublicKey,
/// prime256v1, then the BIT STRING of the point.
const P256_SPKI_PREFIX: &str = "3059301306072a8648ce3d020106082a8648ce3d030107034200";

fn new_passkey<'a>(rp_id: &'a str, user_handle: &'a [u8]) -> NewPasskey<'a> {
    NewPasskey {
        rp_id,
        rp_name: "Example",
        user_handle,
        user_name: "ana@example.com",
        user_display_name: "Ana",
        apps: vec![SecretString::new("so.oxy.example")],
    }
}

fn made(rp_id: &str) -> (Item, Registration) {
    create(
        &new_passkey(rp_id, &[0x11; 32]),
        ItemId::random().unwrap(),
        AT,
    )
    .unwrap()
}

fn passkey(item: &Item) -> &Passkey {
    match &item.content {
        Content::Passkey(passkey) => passkey,
        _ => panic!("not a passkey"),
    }
}

/// The attested credential data, read as a relying party reads it (WebAuthn L3 §6.5.1):
/// (AAGUID, credential id, COSE key's x and y).
fn attested(authenticator_data: &[u8]) -> ([u8; 16], Vec<u8>, Vec<u8>, Vec<u8>) {
    assert!(authenticator_data[32] & FLAG_AT != 0);
    let rest = &authenticator_data[37..];
    let aaguid: [u8; 16] = rest[..16].try_into().unwrap();
    let length = u16::from_be_bytes([rest[16], rest[17]]) as usize;
    let credential_id = rest[18..18 + length].to_vec();
    let mut cose = &rest[18 + length..];
    let key: Value = ciborium::from_reader(&mut cose).unwrap();
    assert!(
        cose.is_empty(),
        "nothing follows the COSE key (no extensions)"
    );
    let map = key.as_map().unwrap();
    let get = |label: i64| {
        map.iter()
            .find(|(k, _)| k.as_integer() == Some(label.into()))
            .map(|(_, v)| v.clone())
            .unwrap()
    };
    assert_eq!(get(1).as_integer(), Some(2.into()), "kty EC2");
    assert_eq!(get(3).as_integer(), Some((-7).into()), "alg ES256");
    assert_eq!(get(-1).as_integer(), Some(1.into()), "crv P-256");
    let x = get(-2).into_bytes().unwrap();
    let y = get(-3).into_bytes().unwrap();
    (aaguid, credential_id, x, y)
}

/// ring's verification of a DER ECDSA P-256 SHA-256 signature by the point `04 ‖ x ‖ y`.
fn ring_verifies(x: &[u8], y: &[u8], message: &[u8], signature: &[u8]) -> bool {
    let mut point = vec![4u8];
    point.extend_from_slice(x);
    point.extend_from_slice(y);
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_ASN1, point)
        .verify(message, signature)
        .is_ok()
}

fn signed(authenticator_data: &[u8], client_data_hash: &ClientDataHash) -> Vec<u8> {
    [authenticator_data, client_data_hash].concat()
}

#[test]
fn relying_party_ids() {
    for good in [
        "example.com",
        "login.example.com",
        "localhost",
        "xn--caf-dma.example",
        "a-b.co",
        "1password.com",
    ] {
        assert!(valid_rp_id(good), "{good}");
    }
    let long = format!("{}.com", "a".repeat(64));
    for bad in [
        "",
        "Example.com",
        "example.com.",
        ".example.com",
        "exa mple.com",
        "-example.com",
        "example-.com",
        "ex_ample.com",
        "https://example.com",
        "example.com:443",
        "127.0.0.1",
        "[::1]",
        "café.example",
        long.as_str(),
    ] {
        assert!(!valid_rp_id(bad), "{bad}");
    }
    assert!(!valid_rp_id(&format!("{}.com", "a.".repeat(126))));
}

#[test]
fn algorithms() {
    assert!(supports(&[-7]));
    assert!(supports(&[-8, -257, -7]));
    assert!(!supports(&[-257, -8]));
    assert!(!supports(&[]));
}

#[test]
fn authenticator_data_layout() {
    // WebAuthn L3 §6.1: rpIdHash (32) ‖ flags (1) ‖ signCount (4).
    let data = authenticator_data("example.com", FLAGS, None);
    assert_eq!(data.len(), 37);
    assert_eq!(hex::encode(&data[..32]), EXAMPLE_COM_HASH);
    assert_eq!(data[32], 0b0001_1101, "UP, UV, BE and BS");
    assert_eq!(&data[33..], &[0, 0, 0, 0], "the counter is 0");
    assert_eq!(FLAGS | FLAG_AT, 0x5d);
}

#[test]
fn cose_key_is_ctap2_canonical() {
    let (_, registration) = made("example.com");
    let (_, _, x, y) = attested(&registration.authenticator_data);
    // WebAuthn L3 §6.5.1.1's example: A5 01 02 03 26 20 01 21 58 20 x 22 58 20 y.
    let mut expected = hex::decode("a50102032620012158 20".replace(' ', "")).unwrap();
    expected.extend_from_slice(&x);
    expected.extend_from_slice(&hex::decode("225820").unwrap());
    expected.extend_from_slice(&y);
    let key = VerifyingKey::from_sec1_bytes(&[&[4u8][..], &x, &y].concat()).unwrap();
    assert_eq!(cose_key(&key).unwrap(), expected);
}

#[test]
fn registration_is_none_attestation() {
    let (item, registration) = made("example.com");
    let object = &registration.attestation_object;
    // CTAP2 canonical: "fmt", "attStmt", "authData" (by length, then bytes).
    let mut prefix = vec![0xa3, 0x63];
    prefix.extend_from_slice(b"fmt");
    prefix.push(0x64);
    prefix.extend_from_slice(b"none");
    prefix.push(0x67);
    prefix.extend_from_slice(b"attStmt");
    prefix.push(0xa0);
    prefix.push(0x68);
    prefix.extend_from_slice(b"authData");
    prefix.push(0x58);
    prefix.push(registration.authenticator_data.len() as u8);
    assert!(object.starts_with(&prefix));
    assert_eq!(
        &object[prefix.len()..],
        registration.authenticator_data.as_slice()
    );
    let data = &registration.authenticator_data;
    assert_eq!(hex::encode(&data[..32]), EXAMPLE_COM_HASH);
    assert_eq!(data[32], FLAGS | FLAG_AT);
    assert_eq!(&data[33..37], &[0, 0, 0, 0]);
    let (aaguid, credential_id, x, y) = attested(data);
    assert_eq!(aaguid, AAGUID);
    assert_eq!(credential_id, registration.credential_id);
    assert_eq!(credential_id.len(), 16);
    // getPublicKey(): the same point as SubjectPublicKeyInfo.
    let spki = hex::encode(&registration.public_key);
    assert_eq!(
        spki,
        format!("{P256_SPKI_PREFIX}04{}{}", hex::encode(&x), hex::encode(&y))
    );
    // The item holds what was answered.
    let kept = passkey(&item);
    assert_eq!(kept.rp_id.expose(), "example.com");
    assert_eq!(kept.credential_id.expose(), b64(&credential_id));
    assert_eq!(kept.user_handle.expose(), b64(&[0x11; 32]));
    assert_eq!(kept.algorithm, -7);
    assert_eq!(kept.apps, vec![SecretString::new("so.oxy.example")]);
    assert_eq!(item.counter, 1);
    item.validate().unwrap();
}

#[test]
fn every_registration_is_new() {
    let (a, ra) = made("example.com");
    let (b, rb) = made("example.com");
    assert_ne!(ra.credential_id, rb.credential_id);
    assert_ne!(ra.public_key, rb.public_key);
    assert_ne!(passkey(&a).private_key, passkey(&b).private_key);
}

#[test]
fn assertions_verify_independently() {
    let (item, registration) = made("example.com");
    let (_, _, x, y) = attested(&registration.authenticator_data);
    let client_data_hash: ClientDataHash = Sha256::digest(
        br#"{"type":"webauthn.get","challenge":"AAEC","origin":"https://example.com"}"#,
    )
    .into();
    let assertion = assert(passkey(&item), "example.com", &client_data_hash).unwrap();
    assert_eq!(assertion.credential_id, registration.credential_id);
    assert_eq!(assertion.user_handle, vec![0x11; 32]);
    assert_eq!(
        assertion.authenticator_data,
        authenticator_data("example.com", FLAGS, None)
    );
    let message = signed(&assertion.authenticator_data, &client_data_hash);
    assert!(ring_verifies(&x, &y, &message, &assertion.signature));
    // And by RustCrypto, from the SubjectPublicKeyInfo a relying party keeps.
    use p256::ecdsa::signature::Verifier;
    use p256::pkcs8::DecodePublicKey;
    let key = VerifyingKey::from_public_key_der(&registration.public_key).unwrap();
    let signature = Signature::from_der(&assertion.signature).unwrap();
    key.verify(&message, &signature).unwrap();
    // Another client data, another authenticator data, or another key: refused.
    let other: ClientDataHash = [0x42; 32];
    assert!(!ring_verifies(
        &x,
        &y,
        &signed(&assertion.authenticator_data, &other),
        &assertion.signature
    ));
    let mut flags = assertion.authenticator_data.clone();
    flags[32] ^= FLAG_UV;
    assert!(!ring_verifies(
        &x,
        &y,
        &signed(&flags, &client_data_hash),
        &assertion.signature
    ));
    let (_, another) = made("example.com");
    let (_, _, x2, y2) = attested(&another.authenticator_data);
    assert!(!ring_verifies(&x2, &y2, &message, &assertion.signature));
}

#[test]
fn a_passkey_signs_only_for_its_relying_party() {
    let (item, _) = made("example.com");
    assert!(assert(passkey(&item), "example.org", &[0; 32]).is_err());
    assert!(assert(passkey(&item), "login.example.com", &[0; 32]).is_err());
}

#[test]
fn a_passkey_survives_the_vault() {
    // Sealed and opened as the vault keeps it, the item still signs for its public key.
    let (item, registration) = made("github.com");
    let vault = VaultId::random().unwrap();
    let keyring = Keyring::generate().unwrap();
    let sealed = item.seal(&vault, &keyring).unwrap();
    let opened = Item::open(&vault, &item.id, &sealed, &keyring, None).unwrap();
    assert_eq!(opened.content.kind(), crate::item::Kind::Passkey);
    let (_, _, x, y) = attested(&registration.authenticator_data);
    let hash = [7u8; 32];
    let assertion = assert(passkey(&opened), "github.com", &hash).unwrap();
    assert!(ring_verifies(
        &x,
        &y,
        &signed(&assertion.authenticator_data, &hash),
        &assertion.signature
    ));
}

#[test]
fn selection() {
    let (item, registration) = made("example.com");
    let kept = passkey(&item);
    let own = registration.credential_id.to_vec();
    let other = vec![0u8; 16];
    // Discoverable: any of the relying party's when no list is given.
    assert!(matches(kept, "example.com", &[]));
    assert!(matches(kept, "example.com", &[other.clone(), own.clone()]));
    assert!(!matches(kept, "example.com", std::slice::from_ref(&other)));
    assert!(!matches(kept, "example.org", &[]));
    assert!(!matches(kept, "example.org", std::slice::from_ref(&own)));
    assert!(excluded(kept, "example.com", std::slice::from_ref(&own)));
    assert!(!excluded(kept, "example.com", &[]));
    assert!(!excluded(kept, "example.com", std::slice::from_ref(&other)));
    assert!(!excluded(kept, "example.org", std::slice::from_ref(&own)));
}

#[test]
fn what_is_refused_at_registration() {
    let id = || ItemId::random().unwrap();
    assert!(create(&new_passkey("Example.com", &[1]), id(), AT).is_err());
    assert!(create(&new_passkey("example.com", &[]), id(), AT).is_err());
    assert!(create(&new_passkey("example.com", &[1; 65]), id(), AT).is_err());
    assert!(create(&new_passkey("example.com", &[1; 64]), id(), AT).is_ok());
}

#[test]
fn names_are_kept_to_64_bytes() {
    let long = "ñ".repeat(40); // 80 bytes
    let new = NewPasskey {
        rp_name: &long,
        user_name: &long,
        user_display_name: &long,
        ..new_passkey("example.com", &[1])
    };
    let (item, _) = create(&new, ItemId::random().unwrap(), AT).unwrap();
    let kept = passkey(&item);
    for text in [&kept.rp_name, &kept.user_name, &kept.user_display_name] {
        assert_eq!(text.expose(), "ñ".repeat(32));
    }
}

#[test]
fn no_key_in_debug() {
    let (item, registration) = made("example.com");
    let shown = format!(
        "{item:?} {registration:?} {:?} {:?}",
        new_passkey("example.com", &[1]),
        assert(passkey(&item), "example.com", &[0; 32]).unwrap()
    );
    let kept = passkey(&item);
    assert!(!shown.contains(kept.private_key.expose()), "{shown}");
    assert!(!shown.contains("ana@example.com"), "{shown}");
    assert!(
        !shown.contains(&hex::encode(&registration.public_key)),
        "{shown}"
    );
}
