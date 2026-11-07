//! The roster's verifier: every rule, every tamper, every fork (beyond the SDK's vectors).

use super::{Device, Vault, AT};
use crate::recovery::{RecoveryKey, RecoveryPublic};
use crate::roster::{Entry, EntryBody, Head, Op, Principal, RosterError, VerifiedRoster};
use crate::wire::{MemberId, VaultId};

fn reason(vault: &Vault, entries: &[Entry], known: Option<&Head>) -> &'static str {
    VerifiedRoster::verify(&vault.id, entries, known)
        .expect_err("refused")
        .reason()
}

/// The entry with its body replaced by `edit` of it, signatures kept.
fn edited(entry: &Entry, edit: impl Fn(&str) -> String) -> Entry {
    Entry {
        body: edit(&entry.body),
        signatures: entry.signatures.clone(),
    }
}

#[test]
fn a_vault_grows_and_shrinks() {
    let mut vault = Vault::new();
    let roster = vault.roster();
    assert_eq!(roster.head().seq, 0);
    assert_eq!(roster.generation(), 1);
    assert_eq!(roster.members().count(), 1);
    let b = Device::new("b");
    vault.add(&b);
    let remove = vault.next(
        Op::Remove {
            member_id: b.id.clone(),
        },
        (vault.a.principal(), &vault.a.keys),
    );
    vault.entries.push(remove);
    // The `remove` is not complete until its `generation`.
    assert_eq!(reason(&vault, &vault.entries, None), "incomplete");
    let generation = EntryBody {
        vault_id: vault.id.clone(),
        seq: 3,
        prev: Some(vault.entries[2].hash()),
        created_at: AT.into(),
        op: Op::Generation { generation: 2 },
    }
    .sign(&[(vault.a.principal(), &vault.a.keys)])
    .unwrap();
    vault.entries.push(generation);
    let roster = vault.roster();
    assert_eq!(roster.generation(), 2);
    assert!(roster.member(&b.id).is_none());
    assert_eq!(roster.removed(), [b.id.clone()]);
    // A rotation needs no removal.
    vault.append(Op::Generation { generation: 3 });
    assert_eq!(vault.roster().generation(), 3);
}

#[test]
fn appends_are_checked_before_they_are_sent() {
    let mut vault = Vault::new();
    let b = Device::new("b");
    vault.add(&b);
    let roster = vault.roster();
    let c = Device::new("c");
    let by_b = roster
        .next(
            Op::Add {
                member: c.member.clone(),
            },
            AT.into(),
        )
        .sign(&[(b.principal(), &b.keys)])
        .unwrap();
    assert!(roster.extend(&[by_b.clone()]).is_ok());
    let by_c = roster
        .next(
            Op::Add {
                member: Device::new("d").member,
            },
            AT.into(),
        )
        .sign(&[(c.principal(), &c.keys)])
        .unwrap();
    assert_eq!(roster.extend(&[by_c]).unwrap_err().reason(), "signer");
}

#[test]
fn a_changed_body_or_signature_is_refused() {
    let mut vault = Vault::new();
    vault.add(&Device::new("bob"));
    let mut entries = vault.entries.clone();
    // A name changed: the signature no longer covers it.
    entries[1] = edited(&vault.entries[1], |body| body.replace("\"bob\"", "\"eve\""));
    assert_eq!(reason(&vault, &entries, None), "signature");
    // A signature changed.
    let mut entries = vault.entries.clone();
    entries[1].signatures[0].signature[5] ^= 1;
    assert_eq!(reason(&vault, &entries, None), "signature");
    // The genesis' recovery signature changed.
    let mut entries = vault.entries.clone();
    let recovery = entries[0]
        .signatures
        .iter_mut()
        .find(|s| s.signer == Principal::Recovery)
        .unwrap();
    recovery.signature[0] ^= 1;
    assert_eq!(reason(&vault, &entries, None), "signature");
}

#[test]
fn only_the_canonical_closed_schema_is_read() {
    let vault = Vault::new();
    let genesis = &vault.entries[0];
    for (edit, why) in [
        (
            Box::new(|b: &str| b.replacen(':', ": ", 1)) as Box<dyn Fn(&str) -> String>,
            "white space",
        ),
        (
            Box::new(|b: &str| b.replacen('{', "{\"extra\":1,", 1)),
            "an unknown field",
        ),
        (
            Box::new(|b: &str| b.replace("\"op\":\"genesis\"", "\"op\":\"genesys\"")),
            "an unknown op",
        ),
        (
            Box::new(|b: &str| b.replace("\"platform\":\"oxyos\"", "\"platform\":\"plan9\"")),
            "an unknown platform",
        ),
        (
            Box::new(|b: &str| b.replace("\"v\":1", "\"v\":2")),
            "another version",
        ),
        (
            Box::new(|b: &str| b.replace(AT, "2026-11-07T12:00:00Z")),
            "a timestamp without milliseconds",
        ),
        (
            Box::new(|b: &str| b.replace("\"prev\":null,", "")),
            "no prev",
        ),
        (
            Box::new(|b: &str| b.replace("\"name\":\"a\"", "\"name\":\"\"")),
            "an empty name",
        ),
        (
            Box::new(|b: &str| b.replace("\"seq\":0", "\"seq\":0.0")),
            "a number spelt otherwise",
        ),
    ] {
        let entries = [edited(genesis, edit)];
        assert_eq!(reason(&vault, &entries, None), "format", "{why}");
    }
    // A name of 257 bytes.
    let mut long = Device::new("x");
    long.member.name = "é".repeat(128) + "x";
    let body = EntryBody::genesis(
        vault.id.clone(),
        long.member.clone(),
        vault.roster().recovery().clone(),
        AT.into(),
    );
    assert!(
        body.sign(&[]).is_ok(),
        "building does not check; reading does"
    );
    let entry = body.sign(&[]).unwrap();
    assert_eq!(reason(&vault, &[entry], None), "format");
    // A member whose box key is of low order.
    let mut weak = Device::new("w");
    weak.member.box_key = [0; 32];
    let entry = EntryBody::genesis(
        vault.id.clone(),
        weak.member,
        vault.roster().recovery().clone(),
        AT.into(),
    )
    .sign(&[])
    .unwrap();
    assert_eq!(reason(&vault, &[entry], None), "format");
}

#[test]
fn entries_continue_one_another_in_one_vault() {
    let mut vault = Vault::new();
    vault.add(&Device::new("b"));
    vault.add(&Device::new("c"));
    assert_eq!(reason(&vault, &[], None), "genesis");
    // Out of order, a gap, a duplicate.
    let swapped = [
        vault.entries[0].clone(),
        vault.entries[2].clone(),
        vault.entries[1].clone(),
    ];
    assert_eq!(reason(&vault, &swapped, None), "continuity");
    let gap = [vault.entries[0].clone(), vault.entries[2].clone()];
    assert_eq!(reason(&vault, &gap, None), "continuity");
    // Without its genesis.
    assert_eq!(reason(&vault, &vault.entries[1..], None), "continuity");
    // Another vault's roster.
    let other = VaultId::random().unwrap();
    assert_eq!(
        VerifiedRoster::verify(&other, &vault.entries, None)
            .unwrap_err()
            .reason(),
        "vault_mismatch"
    );
    // An entry of another vault, in this one's place.
    let body = EntryBody {
        vault_id: other,
        ..vault
            .roster()
            .next(Op::Generation { generation: 2 }, AT.into())
    };
    let mut mixed = vault.entries.clone();
    mixed.push(body.sign(&[(vault.a.principal(), &vault.a.keys)]).unwrap());
    assert_eq!(reason(&vault, &mixed, None), "vault_mismatch");
}

#[test]
fn signers_are_held_to_their_rights() {
    let mut vault = Vault::new();
    let b = Device::new("b");
    vault.add(&b);
    let recovery = (Principal::Recovery, &vault.recovery);
    // The recovery key adds (recovery), and signs nothing else.
    let c = Device::new("c");
    assert!(vault
        .roster()
        .extend(&[vault.next(
            Op::Add {
                member: c.member.clone()
            },
            (Principal::Recovery, &vault.recovery)
        )])
        .is_ok());
    for op in [
        Op::Remove {
            member_id: b.id.clone(),
        },
        Op::Generation { generation: 2 },
        Op::Recovery {
            recovery: RecoveryKey::generate().unwrap().public().unwrap(),
        },
    ] {
        let entry = vault.next(op, (recovery.0.clone(), recovery.1));
        assert_eq!(
            vault.roster().extend(&[entry]).unwrap_err().reason(),
            "signer"
        );
    }
    // A stranger, claiming a member's id or its own.
    let stranger = Device::new("s");
    let as_member = vault.next(
        Op::Generation { generation: 2 },
        (b.principal(), &stranger.keys),
    );
    assert_eq!(
        vault.roster().extend(&[as_member]).unwrap_err().reason(),
        "signature"
    );
    let as_itself = vault.next(
        Op::Generation { generation: 2 },
        (stranger.principal(), &stranger.keys),
    );
    assert_eq!(
        vault.roster().extend(&[as_itself]).unwrap_err().reason(),
        "signer"
    );
    // Two signatures where one is taken, and none.
    let roster = vault.roster();
    let twice = roster
        .next(Op::Generation { generation: 2 }, AT.into())
        .sign(&[
            (vault.a.principal(), &vault.a.keys),
            (b.principal(), &b.keys),
        ])
        .unwrap();
    assert_eq!(roster.extend(&[twice]).unwrap_err().reason(), "signature");
    let unsigned = roster
        .next(Op::Generation { generation: 2 }, AT.into())
        .sign(&[])
        .unwrap();
    assert_eq!(
        roster.extend(&[unsigned]).unwrap_err().reason(),
        "signature"
    );
    // A genesis signed twice by its member instead of by the recovery key.
    let genesis = &vault.entries[0];
    let mut doubled = genesis.clone();
    let member_signature = doubled
        .signatures
        .iter()
        .find(|s| s.signer != Principal::Recovery)
        .unwrap()
        .clone();
    doubled.signatures = vec![member_signature.clone(), member_signature];
    assert_eq!(reason(&vault, &[doubled], None), "signature");
}

#[test]
fn a_replaced_recovery_key_signs_nothing_more() {
    let mut vault = Vault::new();
    let replacement = RecoveryKey::generate().unwrap().keys().unwrap();
    vault.append(Op::Recovery {
        recovery: RecoveryPublic {
            box_key: replacement.box_key(),
            signing_key: replacement.signing_key(),
        },
    });
    let add = || Op::Add {
        member: Device::new("late").member,
    };
    let by_old = vault.next(add(), (Principal::Recovery, &vault.recovery));
    assert_eq!(
        vault.roster().extend(&[by_old]).unwrap_err().reason(),
        "signature"
    );
    let by_new = vault.next(add(), (Principal::Recovery, &replacement));
    assert!(vault.roster().extend(&[by_new]).is_ok());
}

#[test]
fn membership_and_generations_follow_the_rules() {
    let mut vault = Vault::new();
    let b = Device::new("b");
    vault.add(&b);
    let a = (vault.a.principal(), &vault.a.keys);
    let roster = vault.roster();
    // Adding a current member again; removing a stranger.
    let again = vault.next(
        Op::Add {
            member: b.member.clone(),
        },
        (a.0.clone(), a.1),
    );
    assert_eq!(roster.extend(&[again]).unwrap_err().reason(), "membership");
    let stranger = vault.next(
        Op::Remove {
            member_id: MemberId::random().unwrap(),
        },
        (a.0.clone(), a.1),
    );
    assert_eq!(
        roster.extend(&[stranger]).unwrap_err().reason(),
        "membership"
    );
    // A generation that skips one, or goes back.
    for generation in [3, 2 + 1000] {
        let skip = vault.next(Op::Generation { generation }, (a.0.clone(), a.1));
        assert_eq!(roster.extend(&[skip]).unwrap_err().reason(), "generation");
    }
    let back = roster
        .next(Op::Generation { generation: 1 }, AT.into())
        .sign(&[(a.0.clone(), a.1)])
        .unwrap();
    assert_eq!(roster.extend(&[back]).unwrap_err().reason(), "format");
    // A member may remove itself; the rotation is then another member's to sign.
    let remove_self = vault.next(
        Op::Remove {
            member_id: b.id.clone(),
        },
        (b.principal(), &b.keys),
    );
    let after = roster.extend(&[remove_self.clone()]);
    assert_eq!(after.unwrap_err().reason(), "incomplete");
    let rotation = EntryBody {
        vault_id: vault.id.clone(),
        seq: 3,
        prev: Some(remove_self.hash()),
        created_at: AT.into(),
        op: Op::Generation { generation: 2 },
    };
    let by_b = rotation.sign(&[(b.principal(), &b.keys)]).unwrap();
    assert_eq!(
        roster
            .extend(&[remove_self.clone(), by_b])
            .unwrap_err()
            .reason(),
        "signer"
    );
    let by_a = rotation.sign(&[(a.0, a.1)]).unwrap();
    assert!(roster.extend(&[remove_self, by_a]).is_ok());
}

#[test]
fn a_known_head_pins_the_vault_and_its_history() {
    let mut vault = Vault::new();
    vault.add(&Device::new("b"));
    let known = vault.roster().head();
    vault.add(&Device::new("c"));
    // An extension.
    assert!(VerifiedRoster::verify(&vault.id, &vault.entries, Some(&known)).is_ok());
    // A shorter history: the head is not there.
    assert_eq!(
        reason(&vault, &vault.entries[..1], Some(&known)),
        "not_extension"
    );
    // A fork: another entry at the head's place.
    let mut fork = vault.entries[..1].to_vec();
    let a = (vault.a.principal(), &vault.a.keys);
    let other = VerifiedRoster::verify(&vault.id, &fork, None)
        .unwrap()
        .next(
            Op::Add {
                member: Device::new("mallory").member,
            },
            AT.into(),
        )
        .sign(&[a])
        .unwrap();
    fork.push(other);
    assert_eq!(reason(&vault, &fork, Some(&known)), "not_extension");
    // A head this roster never had, at a place it has.
    let mut wrong = known.clone();
    wrong.hash = "0".repeat(64);
    assert_eq!(
        reason(&vault, &vault.entries, Some(&wrong)),
        "not_extension"
    );
    // Another vault altogether under this vault's id: another genesis.
    let a2 = Device::new("a");
    let recovery = RecoveryKey::generate().unwrap().keys().unwrap();
    let genesis = EntryBody::genesis(
        vault.id.clone(),
        a2.member.clone(),
        RecoveryPublic {
            box_key: recovery.box_key(),
            signing_key: recovery.signing_key(),
        },
        AT.into(),
    )
    .sign(&[(a2.principal(), &a2.keys), (Principal::Recovery, &recovery)])
    .unwrap();
    assert_eq!(reason(&vault, &[genesis], Some(&known)), "genesis_mismatch");
}

#[test]
fn errors_and_debug_name_no_member() {
    let vault = Vault::new();
    let shown = format!("{:?} {:?}", vault.a.member, vault.roster());
    assert!(
        !shown.contains("name: \"a\"") && !shown.contains("\"a\""),
        "{shown}"
    );
    let error = RosterError::Signer { at: 3 };
    assert_eq!(error.to_string(), "signer at entry 3");
}
