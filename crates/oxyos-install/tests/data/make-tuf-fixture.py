#!/usr/bin/env python3
"""Writes `tuf-fixture/`: a TUF repository made by python-tuf, for `oxyos-install`'s test that
its TUF client reads what the reference implementation writes (docs/adr/0030, decision 4).

Run with python-tuf 6.0.0 and securesystemslib[crypto] 1.6.0 (a virtual environment):
    python3 -m venv venv && venv/bin/pip install tuf==6.0.0 'securesystemslib[crypto]==1.6.0'
    venv/bin/python make-tuf-fixture.py

Keys come from fixed seeds (the same as the Rust tests': byte N repeated), so the output is the
same every run but for the signatures' bytes, which Ed25519 makes deterministic too.
"""
import json
import os
import shutil
from datetime import datetime, timezone

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from securesystemslib.signer import CryptoSigner, SSlibKey
from tuf.api.metadata import (
    Metadata, Root, Snapshot, Targets, Timestamp, TargetFile, MetaFile, Key,
)
from tuf.api.serialization.json import JSONSerializer

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "tuf-fixture")
EXPIRES = datetime(2100, 1, 1, tzinfo=timezone.utc)


def signer(seed_byte):
    private = Ed25519PrivateKey.from_private_bytes(bytes([seed_byte]) * 32)
    return CryptoSigner(private)


ROOT_A, ROOT_B, ROOT_C = signer(1), signer(2), signer(3)
TARGETS, SNAPSHOT, TIMESTAMP = signer(4), signer(5), signer(6)
ROOT_D = signer(7)


def write(metadata, name, signers):
    metadata.signatures.clear()
    for s in signers:
        metadata.sign(s, append=True)
    with open(os.path.join(OUT, "metadata", name), "wb") as f:
        f.write(metadata.to_bytes(JSONSerializer(compact=False)))


def main():
    shutil.rmtree(OUT, ignore_errors=True)
    os.makedirs(os.path.join(OUT, "metadata"))

    root = Metadata(Root(expires=EXPIRES, consistent_snapshot=True))
    for role, keys, threshold in [
        ("root", [ROOT_A, ROOT_B, ROOT_C], 2),
        ("targets", [TARGETS], 1),
        ("snapshot", [SNAPSHOT], 1),
        ("timestamp", [TIMESTAMP], 1),
    ]:
        for s in keys:
            root.signed.add_key(s.public_key, role)
        root.signed.roles[role].threshold = threshold
    write(root, "1.root.json", [ROOT_A, ROOT_B])

    # Version 2 of root rotates one root key (C out, D in), signed by the old threshold and the new.
    root.signed.version = 2
    root.signed.revoke_key(ROOT_C.public_key.keyid, "root")
    root.signed.add_key(ROOT_D.public_key, "root")
    write(root, "2.root.json", [ROOT_A, ROOT_B, ROOT_D])

    payload = b"oxyos fixture target\n"
    target = TargetFile.from_data("so.oxy.fixture/1/any/manifest.json", payload, ["sha256"])
    # TUF's `custom`, as python-tuf keeps it: in the target's unrecognized fields.
    target.unrecognized_fields["custom"] = {"oxyos": {"note": "custom data travels with the target"}}
    targets = Metadata(Targets(expires=EXPIRES))
    targets.signed.targets[target.path] = target
    write(targets, "1.targets.json", [TARGETS])
    # A consistent snapshot's target: its directory, then its digest before its name.
    directory = os.path.join(OUT, "targets", "so.oxy.fixture", "1", "any")
    os.makedirs(directory)
    with open(os.path.join(directory, target.hashes["sha256"] + ".manifest.json"), "wb") as f:
        f.write(payload)

    snapshot = Metadata(Snapshot(expires=EXPIRES))
    snapshot.signed.meta["targets.json"] = MetaFile(version=1)
    write(snapshot, "1.snapshot.json", [SNAPSHOT])

    timestamp = Metadata(Timestamp(expires=EXPIRES))
    timestamp.signed.snapshot_meta = MetaFile(version=1)
    write(timestamp, "timestamp.json", [TIMESTAMP])


if __name__ == "__main__":
    main()
