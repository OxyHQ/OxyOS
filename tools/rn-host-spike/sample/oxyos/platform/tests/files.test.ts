// The files' mapping (docs/adr/0058-the-platform-api.mdx, decision 10): the file service's answers
// as the platform's calls resolve and reject, the person's files asked for at first use, and
// following.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isPlatformError } from '../src/errors.ts';
import { createFiles, createFileService, fileError, type FilesModule } from '../src/files.ts';
import type { Events } from '../src/session.ts';

type Answer = { ok: true; value: unknown } | { ok: false; error: string; message: string };

/** A played OxyOSFiles: each request answered by `answer` (its op and arguments), and its events. */
function played(answer: (request: Record<string, any>) => Answer, { granted = true } = {}) {
  const requests: Record<string, any>[] = [];
  const asked: string[] = [];
  let listener: ((json: string) => void) | undefined;
  const events: Events = {
    addListener(name, handler) {
      assert.equal(name, 'OxyOSFilesEvent');
      listener = handler;
      return { remove() {} };
    },
  };
  const reply = (request: Record<string, any>) => ({ json: JSON.stringify(answer(request)) });
  const native: FilesModule = {
    call(json) {
      const request = JSON.parse(json);
      requests.push(request);
      return reply(request);
    },
    request(_id, json) {
      const request = JSON.parse(json);
      requests.push(request);
      return Promise.resolve(reply(request));
    },
  };
  const service = createFileService(native, events, async (capability) => {
    asked.push(capability);
    return granted;
  });
  return { service, files: createFiles(service), requests, asked, tell: (event: unknown) => listener?.(JSON.stringify(event)) };
}

const notGranted: Answer = { ok: false, error: 'not-granted', message: "this application was not granted the person's files" };

test("the service's errors are the platform's codes, its own word kept as the reason", () => {
  const cases: [string, string, string][] = [
    ['not-found', 'notFound', 'notFound'],
    ['no-destination', 'notFound', 'noDestination'],
    ['exists', 'exists', 'exists'],
    ['refused', 'notPermitted', 'refused'],
    ['not-granted', 'notPermitted', 'notGranted'],
    ['invalid-name', 'invalid', 'invalidName'],
    ['cancelled', 'cancelled', 'cancelled'],
    ['network', 'failed', 'network'],
    ['io', 'failed', 'io'],
  ];
  for (const [service, code, reason] of cases) {
    const error = fileError(service, 'said');
    assert.ok(isPlatformError(error, code as never), service);
    assert.equal(error.domain, 'files');
    assert.equal(error.reason, reason);
    assert.equal(error.details.message, 'said');
  }
  assert.equal(fileError('refused').refused, true);
  assert.equal(fileError('not-found').refused, false);
});

test('a request refused the person\'s files asks for them once and is made again if granted', async () => {
  let granted = false;
  const { files, requests, asked } = played((request) =>
    request.op === 'places' && !granted ? notGranted : { ok: true, value: { home: 'file:///home/ada/' } },
  );
  const answer = files.places();
  // Granted by the time it is asked again.
  granted = true;
  assert.deepEqual(await answer, { home: 'file:///home/ada/' });
  assert.deepEqual(asked, ['files']);
  assert.deepEqual(
    requests.map(({ op }) => op),
    ['places', 'places'],
  );
  const denied = played(() => notGranted, { granted: false });
  await assert.rejects(denied.files.trashed(), (error) => isPlatformError(error, 'notPermitted') && error.reason === 'notGranted');
  assert.deepEqual(denied.asked, ['files']);
  assert.equal(denied.requests.length, 1);
});

test('a synchronous call never waits for the person: it throws as the service answered', () => {
  const { service, asked } = played(() => notGranted);
  assert.throws(() => service.call('stat', { uri: 'file:///home/ada/a.txt' }), (error) =>
    isPlatformError(error, 'notPermitted'),
  );
  assert.deepEqual(asked, []);
});

test("items refused the person's files are done again once they are granted; the rest as they were", async () => {
  let granted = false;
  const { files, requests } = played((request) => ({
    ok: true,
    value: request.uris.map((uri: string) =>
      uri.includes('/home/') && !granted
        ? { uri, ok: false, error: 'not-granted', message: '' }
        : uri.includes('missing')
          ? { uri, ok: false, error: 'not-found', message: '' }
          : { uri, ok: true, result: `${uri}.trashed` },
    ),
  }));
  const trashing = files.trash(['file:///home/ada/a.txt', 'file:///data/own/missing', 'file:///data/own/b.txt']);
  granted = true;
  assert.deepEqual(await trashing, [
    { uri: 'file:///home/ada/a.txt', ok: true, result: 'file:///home/ada/a.txt.trashed' },
    { uri: 'file:///data/own/missing', ok: false, code: 'notFound', reason: 'notFound' },
    { uri: 'file:///data/own/b.txt', ok: true, result: 'file:///data/own/b.txt.trashed' },
  ]);
  assert.deepEqual(requests[1]?.uris, ['file:///home/ada/a.txt']);
});

test('a request aborted rejects with cancelled at once; without the module every call is unavailable', async () => {
  const native: FilesModule = {
    call: () => ({ json: '{"ok":true,"value":null}' }),
    request: () => new Promise(() => {}),
  };
  const service = createFileService(native, { addListener: () => ({ remove() {} }) }, async () => true);
  const controller = new AbortController();
  const listing = createFiles(service).list('file:///home/ada/', { signal: controller.signal });
  controller.abort();
  await assert.rejects(listing, (error) => isPlatformError(error, 'cancelled'));
  const none = createFiles(createFileService(null, { addListener: () => ({ remove() {} }) }, async () => true));
  assert.equal(none.available, false);
  await assert.rejects(none.info('file:///home/ada/'), (error) => isPlatformError(error, 'unavailable'));
});

test("a watch's events reach its listener until it stops, the service told", async () => {
  const { files, requests, tell } = played((request) => ({ ok: true, value: request.op === 'watchTrash' ? 4 : null }));
  let changes = 0;
  const stop = files.onTrashChange(() => (changes += 1));
  await new Promise((resolve) => setImmediate(resolve));
  tell({ watch: 4 });
  tell({ watch: 5 });
  stop();
  tell({ watch: 4 });
  assert.equal(changes, 1);
  assert.deepEqual(requests.at(-1), { op: 'unwatch', watch: 4 });
});

test('reading gives text or bytes as asked', async () => {
  const native: FilesModule = {
    call: () => ({ json: '{"ok":true,"value":null}' }),
    request: (_id, json, _content, as) => {
      const { encoding } = JSON.parse(json);
      return Promise.resolve(
        as === 'text'
          ? { json: '{"ok":true,"value":null}', text: encoding === 'base64' ? 'SG9sYQ==' : 'Hola' }
          : { json: '{"ok":true,"value":null}', bytes: new Uint8Array([72]) },
      );
    },
  };
  const files = createFiles(createFileService(native, { addListener: () => ({ remove() {} }) }, async () => true));
  assert.equal(await files.read('file:///a'), 'Hola');
  assert.equal(await files.read('file:///a', 'base64'), 'SG9sYQ==');
  assert.deepEqual(await files.read('file:///a', 'bytes'), new Uint8Array([72]));
});
