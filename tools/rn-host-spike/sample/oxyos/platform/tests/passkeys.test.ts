// Passkeys in the platform API (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7):
// WebAuthn's JSON shapes through the host's module, WebAuthn's errors as PlatformErrors.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isPlatformError, PlatformError } from '../src/errors.ts';
import { createKeychain } from '../src/keychain.ts';
import { createPasskeys, credentialOf, type CreationOptionsJSON, type PasskeyModule } from '../src/passkeys.ts';

const creation: CreationOptionsJSON = {
  rp: { id: 'github.com', name: 'GitHub' },
  user: { id: 'YWRh', name: 'ada', displayName: 'Ada Lovelace' },
  challenge: 'Y2hhbGxlbmdl',
  pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
};

function played(answer: unknown, available = true) {
  const asked: { ceremony: string; options: unknown }[] = [];
  let settle: (value: unknown) => void = () => {};
  const module: PasskeyModule = {
    getConstants: () => ({ available }),
    passkey(ceremony, options) {
      asked.push({ ceremony, options: JSON.parse(options) });
      return answer === 'later' ? new Promise((resolve) => (settle = resolve)) : Promise.resolve(answer);
    },
  };
  return { passkeys: createPasskeys(module), asked, settle: (value: unknown) => settle(value) };
}

test('a creation hands WebAuthn options to the host and answers its credential', async () => {
  const credential = { id: 'Q1JFRA', rawId: 'Q1JFRA', type: 'public-key', response: {} };
  const { passkeys, asked } = played({ credential });
  assert.equal(passkeys.available, true);
  assert.deepEqual(await passkeys.create(creation), credential);
  assert.deepEqual(asked, [{ ceremony: 'create', options: creation }]);
});

test('a sign-in asks get', async () => {
  const credential = { id: 'Q1JFRA', response: { signature: 'U0lH' } };
  const { passkeys, asked } = played({ credential });
  assert.deepEqual(await passkeys.get({ challenge: 'Z2V0', rpId: 'github.com' }), credential);
  assert.equal(asked[0].ceremony, 'get');
});

test("WebAuthn's errors are PlatformErrors of the keychain, the DOMException's name kept", () => {
  for (const [name, reason, code] of [
    ['NotAllowedError', 'notAllowed', 'notPermitted'],
    ['NotAllowedError', 'locked', 'locked'],
    ['InvalidStateError', 'invalidState', 'exists'],
    ['NotSupportedError', 'notSupported', 'unavailable'],
    ['TypeError', 'invalidRequest', 'invalid'],
    ['UnknownError', 'offline', 'failed'],
  ] as const) {
    assert.throws(
      () => credentialOf({ error: { name, reason } }),
      (error: unknown) =>
        error instanceof PlatformError && error.domain === 'keychain' && error.code === code && error.reason === name,
    );
  }
  assert.throws(() => credentialOf({ failed: { reason: 'x' } }), (e: unknown) => isPlatformError(e, 'unavailable'));
  assert.throws(() => credentialOf(null), (e: unknown) => isPlatformError(e, 'failed'));
});

test('unavailable outside an application the session launched; a signal cancels', async () => {
  await assert.rejects(played({}, false).passkeys.create(creation), (e: unknown) => isPlatformError(e, 'unavailable'));
  await assert.rejects(createPasskeys(null).get({ challenge: 'Z2V0', rpId: 'a.example' }), (e: unknown) =>
    isPlatformError(e, 'unavailable'),
  );
  const aborted = new AbortController();
  aborted.abort();
  await assert.rejects(played({}).passkeys.create(creation, { signal: aborted.signal }), (e: unknown) =>
    isPlatformError(e, 'cancelled'),
  );
  const { passkeys, settle } = played('later');
  const controller = new AbortController();
  const waiting = passkeys.get({ challenge: 'Z2V0', rpId: 'github.com' }, { signal: controller.signal });
  controller.abort();
  await assert.rejects(waiting, (e: unknown) => isPlatformError(e, 'cancelled'));
  settle({ credential: {} });
});

test("the keychain framework holds passkeys beside saving, each from its own host module", async () => {
  const credential = { id: 'Q1JFRA' };
  const keychain = createKeychain({
    passkeys: { getConstants: () => ({ available: true }), passkey: async () => ({ credential }) },
  });
  assert.equal(keychain.available, false, 'no OxyOSKeychain: saving unavailable');
  assert.equal(keychain.passkeys.available, true);
  assert.deepEqual(await keychain.passkeys.create(creation), credential);
  assert.equal(createKeychain().passkeys.available, false);
  await assert.rejects(createKeychain().passkeys.get({ challenge: 'Z2V0', rpId: 'github.com' }), (e: unknown) =>
    isPlatformError(e, 'unavailable'),
  );
});
