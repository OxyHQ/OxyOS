// The account's mapping (docs/adr/0058-the-platform-api.mdx, decision 10): the host's snapshots as
// followed state, tokens and locking.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { accountStateOf, createAccount, type AccountModule } from '../src/account.ts';
import { isPlatformError } from '../src/errors.ts';
import type { Events } from '../src/session.ts';

const ada = { status: 'signed_in', account: { id: 'u1', handle: 'ada', displayName: 'Ada' }, online: true };

/** A played OxyOSAccountSession holding `held`, and a way to tell a change. */
function played(held: unknown, { available = true, subscribes = true, locks = true } = {}) {
  let listener: ((snapshot: unknown) => void) | undefined;
  const calls: string[] = [];
  const events: Events = {
    addListener(name, handler) {
      assert.equal(name, 'OxyOSAccountSessionChanged');
      listener = handler;
      return { remove() {} };
    },
  };
  const native: AccountModule = {
    getConstants: () => ({ available }),
    snapshot: () => (calls.push('snapshot'), held),
    subscribe: () => (calls.push('subscribe'), subscribes),
    getAccessToken: async (reason) => (calls.push(`token ${reason}`), { accessToken: 'a.b.c', expiresAt: '2026-11-08T10:00:00Z' }),
    lockSession: () => locks,
  };
  return {
    account: createAccount(native, events),
    calls,
    hold: (next: unknown) => (held = next),
    tell: (snapshot: unknown) => listener?.(snapshot),
  };
}

test("the service's snapshot is read strictly; anything else is the service unavailable", () => {
  assert.deepEqual(accountStateOf(ada), ada);
  for (const wrong of [null, {}, { status: 'signed_in', account: null, online: true }, { ...ada, online: 'yes' }]) {
    assert.equal(accountStateOf(wrong).status, 'unavailable');
  }
  assert.deepEqual(accountStateOf({ status: 'signed_out', account: null, online: false }).account, null);
});

test("the host's own snapshot is read until followed, then the changes it tells; nothing new is the same state", () => {
  const { account, calls, hold, tell } = played(ada);
  const first = account.state.current();
  assert.equal(first?.account?.handle, 'ada');
  hold({ status: 'signed_out', account: null, online: true });
  assert.equal(account.state.current()?.status, 'signed_out');
  const told: string[] = [];
  const stop = account.state.subscribe((state) => told.push(state.status));
  assert.ok(calls.includes('subscribe'));
  tell(ada);
  tell(ada);
  assert.deepEqual(told, ['signed_in']);
  // The service went away (the host holds nothing then): unavailable until followed again.
  hold(null);
  tell(null);
  assert.deepEqual(told, ['signed_in', 'unavailable']);
  assert.equal(account.state.current()?.status, 'unavailable');
  stop();
});

test('a token for a reason, null when the service has none; a reason that is not one is invalid', async () => {
  const { account } = played(ada);
  assert.deepEqual(await account.accessToken('boot'), { accessToken: 'a.b.c', expiresAt: '2026-11-08T10:00:00Z' });
  await assert.rejects(account.accessToken('later' as never), (error) => isPlatformError(error, 'invalid'));
  const none = played(ada, { available: false }).account;
  assert.equal(none.available, false);
  assert.equal(none.state.current(), null);
  await assert.rejects(none.accessToken('boot'), (error) => isPlatformError(error, 'unavailable'));
});

test('locking needs the active window', async () => {
  await played(ada).account.lock();
  await assert.rejects(played(ada, { locks: false }).account.lock(), (error) => isPlatformError(error, 'notPermitted'));
});
