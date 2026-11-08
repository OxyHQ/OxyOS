// The search's mapping (docs/adr/0058-the-platform-api.mdx, decision 10): the index's answers as
// the platform's calls resolve and reject, its changes and its followed state.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isPlatformError } from '../src/errors.ts';
import { createSearch, type SearchModule } from '../src/search.ts';
import type { Events } from '../src/session.ts';

/** A played OxyOSSearch: the requests sent, and a way to answer one or tell the index's own. */
function played({ sends = true, available = true } = {}) {
  const sent: Record<string, unknown>[] = [];
  let listener: ((event: { serial: number; text: string }) => void) | undefined;
  const events: Events = {
    addListener(name, handler) {
      assert.equal(name, 'OxyOSSearchText');
      listener = handler;
      return { remove() {} };
    },
  };
  const native: SearchModule = {
    getConstants: () => ({ available }),
    request(json) {
      if (!sends) return 0;
      sent.push(JSON.parse(json));
      return sent.length;
    },
  };
  return {
    search: createSearch(native, events),
    sent,
    answer: (serial: number, answer: unknown) => listener?.({ serial, text: JSON.stringify(answer) }),
  };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));

test("a query resolves with its answer's value; a refusal rejects with the index's reason as a code", async () => {
  const { search, sent, answer } = played();
  const found = search.query({ text: 'nota' });
  assert.deepEqual(sent[0], { query: { text: 'nota' } });
  answer(1, { results: { items: [], topHit: null, indexing: false } });
  assert.deepEqual(await found, { items: [], topHit: null, indexing: false });
  const preferences = search.setPreferences({ categories: [] });
  answer(2, { refused: { reason: 'notPermitted' } });
  await assert.rejects(preferences, (error) => isPlatformError(error, 'notPermitted') && error.domain === 'search');
  const odd = search.status();
  answer(3, { a: 1, b: 2 });
  await assert.rejects(odd, (error) => isPlatformError(error, 'failed'));
  await assert.rejects(played({ sends: false }).search.query({}), (error) => isPlatformError(error, 'unavailable'));
  await assert.rejects(played({ available: false }).search.query({}), (error) => isPlatformError(error, 'unavailable'));
});

test('a query aborted rejects with cancelled at once; its answer is dropped', async () => {
  const { search, answer } = played();
  const controller = new AbortController();
  const found = search.query({ text: 'x' }, { signal: controller.signal });
  controller.abort();
  await assert.rejects(found, (error) => isPlatformError(error, 'cancelled'));
  answer(1, { results: {} });
});

test('the index watches while something listens to its changes', async () => {
  const { search, sent, answer } = played();
  let changed = 0;
  const stop = search.onChange(() => (changed += 1));
  const also = search.onChange(() => {});
  answer(0, { changed: {} });
  stop();
  also();
  assert.equal(changed, 1);
  assert.deepEqual(sent, [{ watch: {} }, { unwatch: {} }]);
});

test("the index's state is followed while something follows it", async () => {
  const { search, sent, answer } = played();
  const told: unknown[] = [];
  const stop = search.state.subscribe((state) => told.push(state));
  assert.deepEqual(sent[0], { followStatus: {} });
  answer(1, { status: { indexing: true, items: 3 } });
  await settle();
  answer(0, { status: { indexing: false, items: 4 } });
  assert.deepEqual(told, [{ indexing: true, items: 3 }, { indexing: false, items: 4 }]);
  assert.deepEqual(search.state.current(), { indexing: false, items: 4 });
  stop();
  assert.deepEqual(sent.at(-1), { unfollowStatus: {} });
});
