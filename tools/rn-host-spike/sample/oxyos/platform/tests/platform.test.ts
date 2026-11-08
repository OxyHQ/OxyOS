// The platform API's mapping (docs/adr/0058-the-platform-api.mdx, decision 10): errors,
// cancellation and following, over a played host module.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { codeOf, errorOf, isPlatformError, PlatformError } from '../src/errors.ts';
import { followed } from '../src/followed.ts';
import { createFrameworks } from '../src/frameworks.ts';
import { createSession, type Events } from '../src/session.ts';

/** A played OxyOSSystem: the requests sent, and a way to answer one or tell state unasked. */
function played({ sends = true } = {}) {
  const sent: { serial: number; body: Record<string, unknown> }[] = [];
  let listener: ((event: { serial: number; answer: unknown }) => void) | undefined;
  const events: Events = {
    addListener(name, handler) {
      assert.equal(name, 'OxyOSSystemAnswer');
      listener = handler;
      return { remove() {} };
    },
  };
  let next = 0;
  const native = {
    request(json: string) {
      if (!sends) return 0;
      next += 1;
      sent.push({ serial: next, body: JSON.parse(json) });
      return next;
    },
  };
  const session = createSession(native, events);
  return {
    session,
    platform: createFrameworks(session),
    sent,
    answer: (serial: number, answer: unknown) => listener?.({ serial, answer }),
    tell: (answer: unknown) => listener?.({ serial: 0, answer }),
  };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));

test("a session's reasons are the platform's codes, or failed with the reason kept", () => {
  assert.equal(codeOf('notPermitted'), 'notPermitted');
  assert.equal(codeOf('needsInput'), 'needsInput');
  assert.equal(codeOf('locked'), 'locked');
  assert.equal(codeOf('invalidRequest'), 'invalid');
  assert.equal(codeOf('notSent'), 'unavailable');
  assert.equal(codeOf('wrongPassword'), 'failed');
  assert.equal(codeOf(undefined), 'failed');
  assert.equal(errorOf('network', { joined: {} }), null);
  const refused = errorOf('network', { refused: { reason: 'notPermitted' } });
  assert.ok(isPlatformError(refused, 'notPermitted'));
  assert.equal(refused?.domain, 'network');
  const failed = errorOf('network', { failed: { reason: 'wrongPassword' } });
  assert.equal(failed?.code, 'failed');
  assert.equal(failed?.reason, 'wrongPassword');
  assert.equal(errorOf('store', null)?.code, 'failed');
});

test('a call is the wire request, and resolves with what succeeded', async () => {
  const { platform, sent, answer } = played();
  const joined = platform.network.join('LukeSkyRouter');
  assert.deepEqual(sent.at(-1)?.body, { join: { ssid: 'LukeSkyRouter' } });
  answer(sent.at(-1)!.serial, { joined: { ssid: 'LukeSkyRouter' } });
  assert.deepEqual(await joined, { joined: { ssid: 'LukeSkyRouter' } });
  void platform.audio.setVolume('output', 0.5);
  assert.deepEqual(sent.at(-1)?.body, { soundVolume: { direction: 'output', volume: 0.5 } });
  void platform.bluetooth.connect('dev');
  assert.deepEqual(sent.at(-1)?.body, { bluetoothConnect: { id: 'dev' } });
  void platform.settings.refreshUpdates();
  assert.deepEqual(sent.at(-1)?.body, { state: {} });
});

test('a refusal or a failure rejects with a PlatformError of its framework, never resolves', async () => {
  const { platform, sent, answer } = played();
  const refused = platform.power.setChargeLimit(true);
  answer(sent.at(-1)!.serial, { refused: { reason: 'needsInput' } });
  await assert.rejects(refused, (error) => isPlatformError(error, 'needsInput') && error.domain === 'power');
  const failed = platform.network.join('Office');
  answer(sent.at(-1)!.serial, { failed: { reason: 'wrongPassword' } });
  await assert.rejects(failed, (error) => isPlatformError(error, 'failed') && error.reason === 'wrongPassword');
});

test('without the host module, or a request not sent, a call rejects with unavailable', async () => {
  const none = createFrameworks(createSession(null, { addListener: () => ({ remove() {} }) }));
  assert.equal(none.network.available, false);
  await assert.rejects(none.network.scan(), (error) => isPlatformError(error, 'unavailable'));
  const { platform } = played({ sends: false });
  await assert.rejects(platform.store.open('so.oxy.inbox'), (error) => isPlatformError(error, 'unavailable'));
});

test('aborting rejects at once with cancelled, and the answer that comes later is dropped', async () => {
  const { platform, sent, answer } = played();
  const controller = new AbortController();
  const asked = platform.store.install('so.oxy.inbox', { signal: controller.signal });
  controller.abort();
  await assert.rejects(asked, (error) => isPlatformError(error, 'cancelled') && error.domain === 'store');
  answer(sent.at(-1)!.serial, { installed: {} });
  // Already aborted: nothing is sent.
  const before = sent.length;
  await assert.rejects(platform.network.scan({ signal: controller.signal }), (error) =>
    isPlatformError(error, 'cancelled'),
  );
  assert.equal(sent.length, before);
});

test("an install's progress goes to its caller, then its answer", async () => {
  const { platform, sent, answer } = played();
  const seen: unknown[] = [];
  const installing = platform.store.install('so.oxy.inbox', { onProgress: (progress) => seen.push(progress) });
  const { serial } = sent.at(-1)!;
  answer(serial, { progress: { done: 1, total: 2 } });
  answer(serial, { installed: { id: 'so.oxy.inbox' } });
  assert.deepEqual(await installing, { installed: { id: 'so.oxy.inbox' } });
  assert.deepEqual(seen, [{ done: 1, total: 2 }]);
});

test('followed state is asked for as its first follower subscribes, then told as it changes', async () => {
  const { platform, sent, answer, tell } = played();
  assert.equal(platform.network.wifi.current(), null);
  const told: unknown[] = [];
  const stop = platform.network.wifi.subscribe((state) => told.push(state));
  assert.deepEqual(sent.at(-1)?.body, { networks: {} });
  // A second follower asks nothing more.
  const count = sent.length;
  const stopAgain = platform.network.wifi.subscribe(() => {});
  assert.equal(sent.length, count);
  answer(sent.at(-1)!.serial, { network: { wifi: 'on' } });
  tell({ network: { wifi: 'off' } });
  assert.deepEqual(told, [{ wifi: 'on' }, { wifi: 'off' }]);
  assert.deepEqual(platform.network.wifi.current(), { wifi: 'off' });
  stop();
  stopAgain();
  // An answer to another request carries the state too.
  const scanning = platform.network.scan();
  answer(sent.at(-1)!.serial, { network: { wifi: 'on', scanning: true } });
  await scanning;
  assert.deepEqual(platform.network.wifi.current(), { wifi: 'on', scanning: true });
  await settle();
});

test("the input's level is metered exactly while something follows it", () => {
  const { platform, sent, tell } = played();
  const levels: number[] = [];
  const first = platform.audio.inputLevel.subscribe((level) => levels.push(level));
  assert.deepEqual(sent.at(-1)?.body, { soundMeter: { on: true } });
  const second = platform.audio.inputLevel.subscribe(() => {});
  tell({ inputLevel: 0.25 });
  first();
  first();
  assert.notDeepEqual(sent.at(-1)?.body, { soundMeter: { on: false } });
  second();
  assert.deepEqual(sent.at(-1)?.body, { soundMeter: { on: false } });
  assert.deepEqual(levels, [0.25]);
});

test('a followed state tells its followers in order and stops each once', () => {
  let starts = 0;
  let stops = 0;
  const state = followed<number>({ start: () => starts++, stop: () => stops++ });
  const seen: number[] = [];
  const stop = state.subscribe((value) => seen.push(value));
  state.tell(1);
  stop();
  stop();
  state.tell(2);
  assert.deepEqual(seen, [1]);
  assert.equal(state.current(), 2);
  assert.deepEqual([starts, stops], [1, 1]);
  assert.ok(new PlatformError('audio', 'busy') instanceof Error);
});
