// The platform API's mapping (docs/adr/0058-the-platform-api.mdx, decision 10): errors,
// cancellation and following, over a played host module.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { ApplicationConstants } from '../specs/NativeOxyOSApplication.ts';
import { answerQuit, createApplication } from '../src/application.ts';
import { codeOf, errorOf, isPlatformError, PlatformError } from '../src/errors.ts';
import { followed } from '../src/followed.ts';
import { createFrameworks } from '../src/frameworks.ts';
import { createKeychain } from '../src/keychain.ts';
import { createSession, type Events } from '../src/session.ts';

/** A played OxyOSSystem: the requests sent, and a way to answer one or tell state unasked. */
function played({ sends = true } = {}) {
  const sent: { serial: number; body: Record<string, unknown> }[] = [];
  let listener: ((event: { serial: number; answer?: unknown; text?: string }) => void) | undefined;
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
    tellText: (text: string) => listener?.({ serial: 0, text }),
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
  assert.equal(failed?.refused, false);
  assert.equal(refused?.refused, true);
  // What else the session said is kept, for a caller that reads it (the store's `told`).
  const told = errorOf('store', { failed: { reason: 'noRelease', told: true, detail: 'none for this machine' } });
  assert.deepEqual(told?.details, { told: true, detail: 'none for this machine' });
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

test("Wi-Fi's private address, resetting and diagnostics are the session's requests (ADR 0038)", async () => {
  const { platform, sent, answer } = played();
  const set = platform.network.setPrivateAddress('uuid-1', 'rotating');
  assert.deepEqual(sent.at(-1)?.body, { privateAddress: { id: 'uuid-1', mode: 'rotating' } });
  answer(sent.at(-1)!.serial, { privateAddress: { mode: 'rotating', rejoined: true } });
  assert.deepEqual(await set, { privateAddress: { mode: 'rotating', rejoined: true } });
  void platform.network.resetPreview();
  assert.deepEqual(sent.at(-1)?.body, { networkResetPreview: {} });
  const reset = platform.network.reset();
  assert.deepEqual(sent.at(-1)?.body, { networkReset: {} });
  answer(sent.at(-1)!.serial, { failed: { reason: 'busy' } });
  await assert.rejects(reset, (error) => isPlatformError(error, 'busy') && error.domain === 'network');
  void platform.network.diagnose();
  assert.deepEqual(sent.at(-1)?.body, { wifiDiagnostics: {} });
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
  const asked = platform.store.install({ id: 'so.oxy.inbox' }, { signal: controller.signal });
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
  const installing = platform.store.install({ listing: 'inbox' }, { onProgress: (progress) => seen.push(progress) });
  assert.deepEqual(sent.at(-1)?.body, { install: { listing: 'inbox' } });
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

test('a capability asked at first use is answered by its serial; those asked meanwhile share it', async () => {
  let listener: ((event: { serial: number; granted: boolean }) => void) | undefined;
  const events: Events = {
    addListener(name, handler) {
      if (name === 'OxyOSCapabilityResult') listener = handler;
      return { remove() {} };
    },
  };
  const asked: string[] = [];
  const session = createSession({ request: () => 1, requestCapability: (name) => (asked.push(name), asked.length) }, events);
  const first = session.requestCapability('files');
  const second = session.requestCapability('files');
  assert.deepEqual(asked, ['files']);
  listener?.({ serial: 1, granted: true });
  assert.deepEqual(await Promise.all([first, second]), [true, true]);
  // Answered, it is asked again next time: the session keeps the account's decision.
  const again = session.requestCapability('files');
  listener?.({ serial: 2, granted: false });
  assert.equal(await again, false);
  assert.equal(await createSession(null, events).requestCapability('files'), false);
});

test("Activity Monitor's samples are followed while something shows them, of the view it chose", async () => {
  const { platform, sent, tellText, answer } = played();
  const { activity } = platform;
  // The view is kept until following begins, which sends it.
  activity.setView('memory');
  assert.equal(sent.length, 0);
  const samples: unknown[] = [];
  const stop = activity.sample.subscribe((sample) => samples.push(sample));
  assert.deepEqual(sent.at(-1)?.body, { activityFollow: { view: 'memory' } });
  // A sample comes as the session's text (SystemData), parsed once.
  tellText(JSON.stringify({ activity: { view: 'memory', processes: [{ pid: 7 }] } }));
  assert.deepEqual(samples, [{ view: 'memory', processes: [{ pid: 7 }] }]);
  assert.deepEqual(activity.sample.current(), { view: 'memory', processes: [{ pid: 7 }] });
  // Another view while followed: told at once; the same view again: nothing.
  activity.setView('cpu');
  assert.deepEqual(sent.at(-1)?.body, { activityFollow: { view: 'cpu' } });
  const before = sent.length;
  activity.setView('cpu');
  assert.equal(sent.length, before);
  stop();
  assert.deepEqual(sent.at(-1)?.body, { activityFollow: { view: null } });
  // Quitting refused rejects with the platform's error.
  const quitting = activity.quit(7, 123, false);
  answer(sent.at(-1)!.serial, { refused: { reason: 'needsInput' } });
  await assert.rejects(quitting, (error: unknown) => isPlatformError(error, 'needsInput'));
  assert.deepEqual(sent.at(-1)?.body, { activityQuit: { pid: 7, started: 123, force: false } });
});

test("backups are asked for as first followed, and no backup service is a state of its own", () => {
  const { session, sent, answer, tell } = played();
  // Before the session said: null.
  assert.equal(session.state.backup.current(), null);
  const told: unknown[] = [];
  const stop = session.state.backup.subscribe((state) => told.push(state));
  assert.deepEqual(sent.at(-1)?.body, { backup: {} });
  // No service: told as such, not as "not said yet".
  answer(sent.at(-1)!.serial, { backup: null });
  assert.deepEqual(session.state.backup.current(), { backup: null });
  // The service came: its state, told unasked.
  tell({ backup: { automatic: true, destinations: [] } });
  assert.deepEqual(told, [{ backup: null }, { backup: { automatic: true, destinations: [] } }]);
  // An answer without it says nothing of it.
  tell({ power: { battery: null } });
  assert.equal(told.length, 2);
  stop();
});

/** A played OxyOSKeychain: what it was asked, and its answer to each, in turn. */
function playedKeychain(answers: unknown[], available = true) {
  const asked: unknown[][] = [];
  const module = {
    getConstants: () => ({ available }),
    savePassword(...args: unknown[]) {
      asked.push(args);
      return Promise.resolve(answers.shift());
    },
  };
  return { keychain: createKeychain({ keychain: module }), asked };
}

test("asking to save a password: the person's decision, or the error it is", async () => {
  const { keychain, asked } = playedKeychain([
    { saved: {} },
    { notSaved: {} },
    { refused: { reason: 'locked' } },
    { refused: { reason: 'notMember' } },
    { refused: { reason: 'tooMany' } },
    { refused: { reason: 'invalidRequest' } },
    { failed: { reason: 'offline' } },
    { surprise: {} },
  ]);
  const credential = { username: 'bea@example.com', password: 'made-up', domain: 'example.com' };
  assert.equal(await keychain.savePassword(credential), 'saved');
  assert.deepEqual(asked[0], ['bea@example.com', 'made-up', 'example.com']);
  assert.equal(await keychain.savePassword({ username: 'bea', password: 'made-up' }), 'notSaved');
  assert.deepEqual(asked[1], ['bea', 'made-up', null]);
  const rejected = async (code: string, reason?: string) => {
    const error = await keychain.savePassword(credential).catch((error: unknown) => error);
    assert.ok(error instanceof PlatformError);
    assert.equal(error.domain, 'keychain');
    assert.equal(error.code, code);
    if (reason) assert.equal(error.reason, reason);
    assert.ok(!String(error.message).includes('made-up'));
  };
  await rejected('locked');
  await rejected('unavailable', 'notMember');
  await rejected('busy', 'tooMany');
  await rejected('invalid');
  await rejected('failed', 'offline');
  await rejected('failed', 'noAnswer');
});

test('asking to save is unavailable without the module and cancellable', async () => {
  const none = createKeychain({ keychain: null });
  assert.equal(createKeychain().available, false);
  assert.equal(none.available, false);
  await assert.rejects(none.savePassword({ username: 'a', password: 'b' }), (error: unknown) =>
    isPlatformError(error, 'unavailable'),
  );
  const { keychain } = playedKeychain([], false);
  await assert.rejects(keychain.savePassword({ username: 'a', password: 'b' }), (error: unknown) =>
    isPlatformError(error, 'unavailable'),
  );
  // Aborted while the person is asked: rejected at once, the answer dropped.
  let answer: (value: unknown) => void = () => {};
  const waiting = createKeychain({
    keychain: {
      getConstants: () => ({ available: true }),
      savePassword: () => new Promise((resolve) => (answer = resolve)),
    },
  });
  const controller = new AbortController();
  const asked = waiting.savePassword({ username: 'a', password: 'b' }, { signal: controller.signal });
  controller.abort();
  await assert.rejects(asked, (error: unknown) => isPlatformError(error, 'cancelled'));
  answer({ saved: {} });
  await settle();
});

/** An application's constants, as the host gives them, with `more`. */
const applicationConstants = (more: Partial<ApplicationConstants>): ApplicationConstants => ({
  id: 'so.oxy.chat',
  recorded: true,
  module: 'Chat',
  manifest: null,
  dataDirectory: null,
  cacheDirectory: null,
  deviceName: '',
  systemVersion: '',
  launchURL: null,
  backgroundItems: false,
  quitRequests: false,
  ...more,
});
/** A session that asks nothing (the application's own calls do not reach it). */
const quietSession = () => createSession(null, { addListener: () => ({ remove() {} }) });
const opens = async () => ({ opened: true, refused: null });

/** A played OxyOSApplication: the asks sent, and a way to answer one or tell a change unasked. */
function playedApplication({ available = true, sends = true } = {}) {
  const asked: { serial: number; kind: string; action: string }[] = [];
  let listener: ((event: { serial: number; kind: string; status: string }) => void) | undefined;
  const events: Events = {
    addListener(name, handler) {
      if (name === 'OxyOSQuitRequested') return { remove() {} };
      assert.equal(name, 'OxyOSBackgroundItemState');
      listener = handler;
      return { remove() {} };
    },
  };
  let next = 0;
  const native = {
    getConstants: () => applicationConstants({ backgroundItems: available }),
    open: opens,
    backgroundItem(kind: string, action: string) {
      if (!sends) return 0;
      next += 1;
      asked.push({ serial: next, kind, action });
      return next;
    },
  };
  return {
    application: createApplication(native, quietSession(), events),
    asked,
    answer: (serial: number, kind: string, status: string) => listener?.({ serial, kind, status }),
  };
}

test('an application registers its login item and background activity, the session deciding', async () => {
  const { application, asked, answer } = playedApplication();
  assert.equal(application.loginItem.available, true);
  // Registered and allowed: enabled, and its followers are told.
  const told: string[] = [];
  const stop = application.loginItem.subscribe((status) => told.push(status));
  assert.deepEqual(asked, [{ serial: 1, kind: 'loginItem', action: 'status' }]);
  answer(1, 'loginItem', 'notRegistered');
  const registering = application.loginItem.register();
  answer(2, 'loginItem', 'enabled');
  assert.equal(await registering, 'enabled');
  assert.deepEqual(told, ['notRegistered', 'enabled']);
  // The person removed it from Open at Login: told unasked.
  answer(0, 'loginItem', 'notRegistered');
  assert.equal(application.loginItem.current(), 'notRegistered');
  stop();
  // Background activity the person turned off: refused, saying why; one the platform gives none.
  const refused = application.backgroundActivity.register();
  answer(3, 'background', 'requiresApproval');
  await assert.rejects(refused, (error: unknown) => {
    assert.ok(isPlatformError(error, 'notPermitted'));
    assert.equal(error.domain, 'application');
    assert.equal(error.reason, 'requiresApproval');
    return true;
  });
  const none = application.backgroundActivity.register();
  answer(4, 'background', 'notPermitted');
  await assert.rejects(none, (error: unknown) => isPlatformError(error, 'notPermitted') && error.reason === undefined);
  // Not kept: failed. Unregistering answers how it then stands.
  const unkept = application.backgroundActivity.register();
  answer(5, 'background', 'notRegistered');
  await assert.rejects(unkept, (error: unknown) => isPlatformError(error, 'failed'));
  const unregistering = application.backgroundActivity.unregister();
  answer(6, 'background', 'notRegistered');
  assert.equal(await unregistering, 'notRegistered');
  // An answer of another kind, or an unknown status, tells nothing.
  answer(0, 'loginItem', 'sideways');
  assert.equal(application.loginItem.current(), 'notRegistered');
});

test('an ask is cancelled with its signal, and without the session it is unavailable', async () => {
  const { application, answer } = playedApplication();
  const controller = new AbortController();
  const asking = application.loginItem.status({ signal: controller.signal });
  controller.abort();
  await assert.rejects(asking, (error: unknown) => isPlatformError(error, 'cancelled'));
  // Its answer, when it comes, still tells the followers, and finds no one waiting.
  answer(1, 'loginItem', 'enabled');
  assert.equal(application.loginItem.current(), 'enabled');
  const aborted = new AbortController();
  aborted.abort();
  await assert.rejects(application.loginItem.register({ signal: aborted.signal }), (error: unknown) =>
    isPlatformError(error, 'cancelled'),
  );
  const older = playedApplication({ available: false }).application;
  assert.equal(older.backgroundActivity.available, false);
  await assert.rejects(older.backgroundActivity.register(), (error: unknown) => isPlatformError(error, 'unavailable'));
  const unsent = playedApplication({ sends: false }).application;
  await assert.rejects(unsent.loginItem.status(), (error: unknown) => isPlatformError(error, 'unavailable'));
});

test("the account's applications are followed as Settings asks for them and as the session tells them", () => {
  const { platform, answer, tell } = played();
  const told: unknown[] = [];
  // Settings' Privacy page follows them through the platform's settings.
  platform.settings.decisions.subscribe((apps) => told.push(apps));
  void platform.settings.privacy();
  answer(1, { privacy: [{ id: 'so.oxy.chat', capabilities: { background: 'granted' }, runningInBackground: false }] });
  tell({ privacy: [{ id: 'so.oxy.chat', capabilities: { background: 'granted' }, runningInBackground: true }] });
  assert.deepEqual(
    told.map((apps) => (apps as { runningInBackground: boolean }[])[0].runningInBackground),
    [false, true],
  );
});

/** A played OxyOSApplication that asks to quit: what JavaScript said it handles, and its answers. */
function playedQuit({ quitRequests = true } = {}) {
  const handles: boolean[] = [];
  const replies: { serial: number; quit: boolean }[] = [];
  let ask: ((event: { serial: number; reason: string }) => void) | undefined;
  const events: Events = {
    addListener(name, handler) {
      if (name === 'OxyOSQuitRequested') ask = handler;
      return { remove() {} };
    },
  };
  const native = {
    getConstants: () => applicationConstants({ quitRequests }),
    open: opens,
    backgroundItem: () => 0,
    handlesQuit: (handling: boolean) => void handles.push(handling),
    replyToQuit: (serial: number, quit: boolean) => {
      replies.push({ serial, quit });
      return true;
    },
  };
  return {
    application: createApplication(native, quietSession(), events),
    handles,
    replies,
    ask: (serial: number, reason: string) => ask?.({ serial, reason }),
  };
}

test('the session asks an application to quit, and any handler may cancel (ADR 0064)', async () => {
  const { application, handles, replies, ask } = playedQuit();
  assert.equal(application.quitRequests, true);
  // Told the host it handles the question with the first handler, and no more with the last gone.
  const reasons: string[] = [];
  let dirty = true;
  const stopFirst = application.onQuitRequest((reason) => {
    reasons.push(reason);
    return dirty ? 'cancel' : 'quit';
  });
  const stopSecond = application.onQuitRequest(async (): Promise<'quit'> => 'quit');
  assert.deepEqual(handles, [true]);
  // An unsaved document cancels the restart.
  ask(7, 'restart');
  await settle();
  assert.deepEqual(replies, [{ serial: 7, quit: false }]);
  assert.deepEqual(reasons, ['restart']);
  // Saved: every handler quits, and the application answers quit.
  dirty = false;
  ask(8, 'shutDown');
  await settle();
  assert.deepEqual(replies.at(-1), { serial: 8, quit: true });
  // An unknown reason is no request.
  ask(9, 'sideways');
  await settle();
  assert.equal(replies.length, 2);
  stopFirst();
  stopFirst();
  assert.deepEqual(handles, [true]);
  stopSecond();
  assert.deepEqual(handles, [true, false]);
});

test('a handler that throws or rejects cancels: nothing unsaved is lost to a bug', async () => {
  assert.equal(await answerQuit([], 'logOut'), 'quit');
  assert.equal(await answerQuit([() => 'quit'], 'logOut'), 'quit');
  assert.equal(
    await answerQuit(
      [
        () => {
          throw new Error('boom');
        },
      ],
      'logOut',
    ),
    'cancel',
  );
  assert.equal(await answerQuit([async () => Promise.reject(new Error('boom'))], 'restart'), 'cancel');
  // Asked in order: the first cancel ends the asking.
  const asked: string[] = [];
  const answer = await answerQuit(
    [
      () => {
        asked.push('first');
        return 'cancel';
      },
      () => {
        asked.push('second');
        return 'quit';
      },
    ],
    'shutDown',
  );
  assert.equal(answer, 'cancel');
  assert.deepEqual(asked, ['first']);
});

test('where the session never asks, a handler is kept and the host is told nothing', () => {
  const { application, handles } = playedQuit({ quitRequests: false });
  assert.equal(application.quitRequests, false);
  const stop = application.onQuitRequest(() => 'quit');
  stop();
  assert.deepEqual(handles, []);
});

test("backups and Settings › Storage are the session's requests, rejecting with its refusal", async () => {
  const { platform, sent, answer } = played();
  const now = platform.backup.backUpNow();
  assert.deepEqual(sent.at(-1)?.body, { backupNow: {} });
  answer(sent.at(-1)!.serial, { refused: { reason: 'needsInput' } });
  await assert.rejects(now, (error: unknown) => isPlatformError(error, 'needsInput') && error.domain === 'backup');
  void platform.backup.setExcluded(['/home/a/tmp']);
  assert.deepEqual(sent.at(-1)?.body, { backupExcluded: { paths: ['/home/a/tmp'] } });
  const measured = platform.settings.storage();
  answer(sent.at(-1)!.serial, { storage: { volume: {} } });
  assert.deepEqual(await measured, { storage: { volume: {} } });
  void platform.settings.uninstall('so.oxy.chat');
  assert.deepEqual(sent.at(-1)?.body, { uninstall: { id: 'so.oxy.chat' } });
  void platform.settings.resetApplication('so.oxy.chat');
  assert.deepEqual(sent.at(-1)?.body, { resetApplication: { id: 'so.oxy.chat' } });
});
