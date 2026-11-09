// The application's mapping (docs/adr/0058-the-platform-api.mdx, decision 10): opening a URL and
// the permissions, as the platform's calls resolve and reject.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { ApplicationConstants, OpenAnswer } from '../specs/NativeOxyOSApplication.ts';
import { createApplication, type ApplicationModule } from '../src/application.ts';
import { isPlatformError } from '../src/errors.ts';
import { createSession, type Events, type Launch } from '../src/session.ts';

const constants: ApplicationConstants = {
  id: 'so.oxy.inbox',
  recorded: true,
  module: 'Inbox',
  manifest: null,
  dataDirectory: '/home/ada/.local/share/oxyos/apps/so.oxy.inbox',
  cacheDirectory: '/home/ada/.cache/oxyos/apps/so.oxy.inbox',
  deviceName: 'thinkpad',
  systemVersion: '1',
  launchURL: 'inbox://thread/3',
  backgroundItems: false,
  quitRequests: false,
};

/** A played session whose launch is `launch`, and whose capability asks answer `granted`. */
function playedSession(launch: Launch | null, granted: boolean) {
  let listener: ((event: { serial: number; granted: boolean }) => void) | undefined;
  const events: Events = {
    addListener(name, handler) {
      if (name === 'OxyOSCapabilityResult') listener = handler;
      return { remove() {} };
    },
  };
  const asked: string[] = [];
  const session = createSession(
    {
      request: () => 1,
      getLaunch: () => launch,
      requestCapability(capability) {
        asked.push(capability);
        if (granted) launch?.capabilities.push(capability);
        queueMicrotask(() => listener?.({ serial: asked.length, granted }));
        return asked.length;
      },
    },
    events,
  );
  return { session, asked };
}

/** No events: the background items and quitting are platform.test.ts's. */
const quiet: Events = { addListener: () => ({ remove() {} }) };

const module = (answer: OpenAnswer, opened: string[] = []): ApplicationModule => ({
  getConstants: () => constants,
  open: async (url) => (opened.push(url), answer),
});

test('what the session started it as and to open', () => {
  const { session } = playedSession(null, true);
  const application = createApplication(module({ opened: true, refused: null }), session, quiet);
  assert.equal(application.id, 'so.oxy.inbox');
  assert.equal(application.launchURL, 'inbox://thread/3');
  assert.equal(application.directories.cache, '/home/ada/.cache/oxyos/apps/so.oxy.inbox');
  const none = createApplication(null, session, quiet);
  assert.equal(none.available, false);
  assert.equal(none.launchURL, null);
});

test("a URL opened resolves; the session's refusal rejects with its code", async () => {
  const { session } = playedSession(null, true);
  const opened: string[] = [];
  await createApplication(module({ opened: true, refused: null }, opened), session, quiet).open('dropsea://trash');
  assert.deepEqual(opened, ['dropsea://trash']);
  const refusals: [string, string][] = [
    ['needsInput', 'needsInput'],
    ['notOpened', 'failed'],
    ['invalid', 'invalid'],
    ['unavailable', 'unavailable'],
  ];
  for (const [reason, code] of refusals) {
    const application = createApplication(module({ opened: false, refused: { reason, message: 'said' } }), session, quiet);
    await assert.rejects(application.open('https://oxy.so'), (error) =>
      isPlatformError(error, code as never) && error.reason === reason && error.details.message === 'said',
    );
  }
  await assert.rejects(createApplication(null, session, quiet).open('x:y'), (error) => isPlatformError(error, 'unavailable'));
});

test('a permission is granted to a system application; an installed one asks the person once', async () => {
  const system = playedSession({ origin: 'system', narrowed: false, capabilities: [], grants: ['store'] }, false);
  const ours = createApplication(module({ opened: true, refused: null }), system.session, quiet);
  assert.equal(ours.permission('files'), 'granted');
  assert.equal(await ours.requestPermission('files'), 'granted');
  assert.deepEqual(system.asked, []);
  assert.deepEqual(ours.grants, ['store']);

  const installed = playedSession({ origin: 'installed', narrowed: true, capabilities: [], grants: [] }, true);
  const theirs = createApplication(module({ opened: true, refused: null }), installed.session, quiet);
  assert.equal(theirs.permission('notifications'), 'denied');
  assert.equal(await theirs.requestPermission('notifications'), 'granted');
  assert.equal(theirs.permission('notifications'), 'granted');
  assert.deepEqual(installed.asked, ['notifications']);

  const refused = playedSession({ origin: 'installed', narrowed: true, capabilities: [], grants: [] }, false);
  assert.equal(await createApplication(null, refused.session, quiet).requestPermission('camera'), 'denied');
});
