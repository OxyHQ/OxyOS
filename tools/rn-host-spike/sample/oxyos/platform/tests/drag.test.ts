// The drag's mapping (docs/adr/0058-the-platform-api.mdx, decision 10): how the host's drag ends,
// as the platform's call resolves and rejects.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { createDrag, type DragModule } from '../src/drag.ts';
import { isPlatformError } from '../src/errors.ts';
import type { Events } from '../src/session.ts';

/** A played OxyOSDrag that ends each drag as `end` says: dropped with an action, or cancelled. */
function played(end: () => { action: string } | { cancelled: string }) {
  const started: unknown[][] = [];
  const native: DragModule = {
    getConstants: () => ({ available: true }),
    start(data, actions, preview, count, options, onCancel) {
      started.push([data, actions, preview, count, options]);
      const how = end();
      if ('cancelled' in how) {
        onCancel(how.cancelled);
        return Promise.resolve(null);
      }
      return Promise.resolve(how.action);
    },
  };
  return { native, started };
}

function events() {
  let listener: ((event: { type?: string; dropped?: boolean }) => void) | undefined;
  const bus: Events = {
    addListener(name, handler) {
      assert.equal(name, 'OxyOSWindowDrag');
      listener = handler;
      return { remove: () => (listener = undefined) };
    },
  };
  return { bus, tell: (event: { type?: string; dropped?: boolean }) => listener?.(event) };
}

const request = { data: { text: 'Hola' }, actions: ['copy', 'move'] as const };

test('a drag resolves with the action its drop took, or none and why', async () => {
  let end: { action: string } | { cancelled: string } = { action: 'move' };
  const { native, started } = played(() => end);
  const drag = createDrag(native, events().bus);
  assert.deepEqual(await drag.start({ ...request, preview: 12, count: 3, vanish: true }), { action: 'move' });
  assert.deepEqual(started[0], [{ text: 'Hola' }, ['copy', 'move'], 12, 3, { vanish: true }]);
  end = { cancelled: 'not-accepted' };
  assert.deepEqual(await drag.start(request), { action: null, reason: 'notAccepted' });
  end = { cancelled: 'escape' };
  assert.deepEqual(await drag.start(request), { action: null, reason: 'escape' });
  assert.deepEqual(started[2]?.slice(2), [-1, 1, { vanish: false }]);
});

test('a drag with no press to start from rejects with needsInput; one offering nothing, invalid', async () => {
  const { native, started } = played(() => ({ cancelled: 'refused' }));
  const drag = createDrag(native, events().bus);
  await assert.rejects(drag.start(request), (error) => isPlatformError(error, 'needsInput'));
  await assert.rejects(drag.start({ data: {}, actions: ['copy'] }), (error) => isPlatformError(error, 'invalid'));
  await assert.rejects(drag.start({ data: { text: 'x' }, actions: [] }), (error) => isPlatformError(error, 'invalid'));
  assert.equal(started.length, 1);
});

test('without a window the host rejects: unavailable; without the module, every call too', async () => {
  const native: DragModule = {
    getConstants: () => ({ available: true }),
    start: () => Promise.reject(new Error('OxyOSDrag: dragging needs a window on OxyOS')),
  };
  await assert.rejects(createDrag(native, events().bus).start(request), (error) => isPlatformError(error, 'unavailable'));
  const none = createDrag(null, events().bus);
  assert.equal(none.available, false);
  await assert.rejects(none.start(request), (error) => isPlatformError(error, 'unavailable'));
});

test('a drag aborted rejects with cancelled at once; drags over the window are told until unsubscribed', async () => {
  const native: DragModule = {
    getConstants: () => ({ available: true }),
    start: () => new Promise(() => {}),
  };
  const { bus, tell } = events();
  const drag = createDrag(native, bus);
  const controller = new AbortController();
  const dragging = drag.start(request, { signal: controller.signal });
  controller.abort();
  await assert.rejects(dragging, (error) => isPlatformError(error, 'cancelled'));
  const told: unknown[] = [];
  const stop = drag.onWindowDrag((event) => told.push(event));
  tell({ type: 'enter' });
  tell({ type: 'exit', dropped: true });
  stop();
  tell({ type: 'enter' });
  assert.deepEqual(told, [{ type: 'enter' }, { type: 'exit', dropped: true }]);
});
