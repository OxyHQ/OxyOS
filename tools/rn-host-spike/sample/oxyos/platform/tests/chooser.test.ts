// The chooser's mapping (docs/adr/0058-the-platform-api.mdx, decision 10): the session's answers to
// a panel, as the platform's calls resolve and reject.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { acceptedTypes, createChooser, type FileChooserModule } from '../src/chooser.ts';
import { isPlatformError } from '../src/errors.ts';
import type { Events } from '../src/session.ts';

/** A played OxyOSFileChooser: the requests asked and withdrawn, and a way to answer one. */
function played({ request = null as string | null, sends = true } = {}) {
  const asked: { serial: number; body: unknown }[] = [];
  const withdrawn: number[] = [];
  const answers: unknown[] = [];
  let listener: ((event: { serial: number; answer: unknown }) => void) | undefined;
  const events: Events = {
    addListener(name, handler) {
      assert.equal(name, 'OxyOSFilesChosen');
      listener = handler;
      return { remove() {} };
    },
  };
  const native: FileChooserModule = {
    getConstants: () => ({ request }),
    choose(json) {
      if (!sends) return 0;
      asked.push({ serial: asked.length + 1, body: JSON.parse(json) });
      return asked.length;
    },
    cancel: (serial) => void withdrawn.push(serial),
    answer: (json) => (answers.push(JSON.parse(json)), true),
  };
  return {
    chooser: createChooser(native, events),
    asked,
    withdrawn,
    answers,
    answer: (serial: number, answer: unknown) => listener?.({ serial, answer }),
  };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));

test("a panel resolves with what the person chose, or null where they cancelled", async () => {
  const { chooser, asked, answer } = played();
  const opening = chooser.open({ multiple: true, types: ['image/PNG; q=1', '.png', 'nonsense'], title: 'Images' });
  assert.deepEqual(asked[0]?.body, {
    open: { multiple: true, directory: false, types: ['image/png', '.png'], title: 'Images' },
  });
  answer(1, { chosen: ['file:///home/a.png', 'file:///home/b.png'] });
  assert.deepEqual(await opening, ['file:///home/a.png', 'file:///home/b.png']);
  const saving = chooser.save({ name: 'Informe.txt' });
  assert.deepEqual(asked[1]?.body, { save: { name: 'Informe.txt', types: [] } });
  answer(2, { chosen: ['file:///home/Informe.txt'] });
  assert.equal(await saving, 'file:///home/Informe.txt');
  const cancelled = chooser.open();
  answer(3, { cancelled: {} });
  assert.equal(await cancelled, null);
});

test("the session's refusal rejects with its code; nothing sent, unavailable", async () => {
  const { chooser, answer } = played();
  const opening = chooser.open();
  answer(1, { refused: { reason: 'needsInput' } });
  await assert.rejects(opening, (error) => isPlatformError(error, 'needsInput') && error.domain === 'chooser');
  const again = chooser.open();
  answer(2, { refused: { reason: 'invalidRequest' } });
  await assert.rejects(again, (error) => isPlatformError(error, 'invalid'));
  await assert.rejects(played({ sends: false }).chooser.open(), (error) => isPlatformError(error, 'unavailable'));
  const none = createChooser(null, { addListener: () => ({ remove() {} }) });
  assert.equal(none.available, false);
  assert.equal(none.request, null);
  await assert.rejects(none.open(), (error) => isPlatformError(error, 'unavailable'));
});

test('a panel withdrawn rejects with cancelled at once, the session told, its answer dropped', async () => {
  const { chooser, withdrawn, answer } = played();
  const controller = new AbortController();
  const opening = chooser.open({ signal: controller.signal });
  controller.abort();
  await assert.rejects(opening, (error) => isPlatformError(error, 'cancelled'));
  assert.deepEqual(withdrawn, [1]);
  answer(1, { cancelled: {} });
  await settle();
  // An abort after the answer withdraws nothing.
  const later = new AbortController();
  const answered = chooser.open({ signal: later.signal });
  answer(2, { chosen: ['file:///home/a.png'] });
  assert.deepEqual(await answered, ['file:///home/a.png']);
  later.abort();
  assert.deepEqual(withdrawn, [1]);
});

test('the chooser the session started reads its request and answers it', async () => {
  const request = JSON.stringify({
    mode: 'save',
    multiple: false,
    directory: false,
    types: ['text/plain'],
    title: null,
    suggestedName: 'Informe.txt',
    folder: null,
    application: { id: 'so.oxy.notes', name: 'Notes' },
  });
  const { chooser, answers } = played({ request });
  assert.equal(chooser.request?.mode, 'save');
  assert.equal(chooser.request?.suggestedName, 'Informe.txt');
  assert.deepEqual(chooser.request?.application, { id: 'so.oxy.notes', name: 'Notes' });
  await chooser.choose(['file:///home/Informe.txt']);
  await chooser.cancel();
  assert.deepEqual(answers, [{ chosen: ['file:///home/Informe.txt'] }, { cancelled: {} }]);
  // Any other application has no request, and nothing to answer.
  await assert.rejects(played().chooser.choose([]), (error) => isPlatformError(error, 'unavailable'));
});

test('accepted types: media types lowercased without parameters, extensions kept, any type none', () => {
  assert.deepEqual(acceptedTypes(['IMAGE/*', '.tar.gz', 'text/plain;charset=utf-8', 'x', 3]), [
    'image/*',
    '.tar.gz',
    'text/plain',
  ]);
  assert.deepEqual(acceptedTypes(['image/png', '*/*']), []);
  assert.deepEqual(acceptedTypes(undefined), []);
});
