use super::{ACCOUNT, OTHER_ACCOUNT};
use crate::id::base32;
use crate::{AccountId, InvalidAccountId, MAX_USER_NAME};

#[test]
fn base32_is_rfc_4648s_in_lowercase_without_padding() {
    // RFC 4648 section 10's vectors.
    for (bytes, text) in [
        ("", ""),
        ("f", "my"),
        ("fo", "mzxq"),
        ("foo", "mzxw6"),
        ("foob", "mzxw6yq"),
        ("fooba", "mzxw6ytb"),
        ("foobar", "mzxw6ytboi"),
    ] {
        assert_eq!(base32(bytes.as_bytes()), text);
    }
}

#[test]
fn a_uuid_names_a_user_that_fits() {
    let name = AccountId::parse(ACCOUNT).unwrap().user_name();
    assert_eq!(name.as_str(), "oxy-agipnrekdz5tzhjoj5ngw7enty");
    assert_eq!(name.as_str().len(), 30);
    assert!(name.as_str().len() <= MAX_USER_NAME);
}

#[test]
fn a_legacy_id_names_a_user_that_fits() {
    let name = AccountId::parse(OTHER_ACCOUNT).unwrap().user_name();
    assert_eq!(name.as_str(), "oxy-mt34finy5hj7jiocwpka");
    assert_eq!(name.as_str().len(), 24);
}

#[test]
fn every_name_is_one_the_system_accepts() {
    for id in [
        ACCOUNT,
        OTHER_ACCOUNT,
        "ffffffffffffffffffffffff",
        "00000000-0000-7000-8000-000000000000",
    ] {
        let name = AccountId::parse(id).unwrap().user_name();
        let text = name.as_str();
        assert!(text.starts_with("oxy-"));
        assert!(text.len() <= MAX_USER_NAME, "{text}");
        // shadow's and systemd's rule: [a-z_][a-z0-9_-]*
        assert!(text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'));
    }
}

#[test]
fn different_accounts_never_share_a_user() {
    let ids = [
        "0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9e",
        "0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9f",
        "0190f6c4-8a1e-7b3c-ad2e-4f5a6b7c8d9e",
        "64f7c2a1b8e9d3f4a1c2b3d4",
        "64f7c2a1b8e9d3f4a1c2b3d5",
        "000000000000000000000000",
    ];
    let names: std::collections::HashSet<_> = ids
        .iter()
        .map(|id| AccountId::parse(id).unwrap().user_name())
        .collect();
    assert_eq!(names.len(), ids.len());
}

#[test]
fn nothing_but_oxys_two_id_shapes_is_an_account() {
    for text in [
        "",
        "ada",
        "root",
        "../../etc/passwd",
        "64F7C2A1B8E9D3F4A1C2B3D4",
        "64f7c2a1b8e9d3f4a1c2b3d",
        "64f7c2a1b8e9d3f4a1c2b3d4a",
        "64f7c2a1b8e9d3f4a1c2b3dg",
        "0190F6C4-8A1E-7B3C-9D2E-4F5A6B7C8D9E",
        // A uuid v4: Oxy generates none.
        "0190f6c4-8a1e-4b3c-9d2e-4f5a6b7c8d9e",
        // The variant is not RFC 9562's.
        "0190f6c4-8a1e-7b3c-cd2e-4f5a6b7c8d9e",
        "0190f6c48a1e7b3c9d2e4f5a6b7c8d9e",
        "0190f6c4-8a1e7-b3c-9d2e-4f5a6b7c8d9e",
        "0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9:",
        "0190f6c4-8a1e-7b3c-9d2e-4f5a6b7c8d9\n",
    ] {
        assert_eq!(AccountId::parse(text), Err(InvalidAccountId), "{text:?}");
    }
}
