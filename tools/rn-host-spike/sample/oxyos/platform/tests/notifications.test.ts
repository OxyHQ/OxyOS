// The notifications' mapping (docs/adr/0058-the-platform-api.mdx, decision 10): what the host's
// module answers and tells, as the platform's calls resolve and reject and its events are told.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { PostedNotification, TakenResponse } from '../specs/NativeOxyOSNotifications.ts';
import { isPlatformError } from '../src/errors.ts';
import { createNotifications, type NotificationsModule } from '../src/notifications.ts';
import type { Events } from '../src/session.ts';

/** A played OxyOSNotifications: what was posted and withdrawn, the responses waiting, and its events. */
function played({ sent = 'sent' } = {}) {
  const posted: { serial: number; notification: PostedNotification }[] = [];
  const withdrawn: number[] = [];
  const waiting: TakenResponse[] = [];
  const listeners = new Map<string, Set<(event: any) => void>>();
  const events: Events = {
    addListener(name, handler) {
      const set = listeners.get(name) ?? new Set();
      set.add(handler);
      listeners.set(name, set);
      return { remove: () => set.delete(handler) };
    },
  };
  const native: NotificationsModule = {
    post(serial, notification) {
      if (sent === 'sent') posted.push({ serial, notification });
      return sent;
    },
    withdraw: (notification) => void withdrawn.push(notification),
    takeResponses: () => waiting.splice(0),
  };
  const emit = (name: string, event: unknown) => {
    for (const listener of [...(listeners.get(name) ?? [])]) listener(event);
  };
  return { notifications: createNotifications(native, events), posted, withdrawn, waiting, emit };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));

const response = (more: Partial<TakenResponse> = {}): TakenResponse => ({
  notification: 7,
  action: 0,
  identifier: 'mail-1',
  title: 'Ana',
  subtitle: '',
  body: 'Hola',
  data: '{"thread":3}',
  actionId: '',
  reply: '',
  ...more,
});

test("a post resolves with the session's number once it says posted", async () => {
  const { notifications, posted, emit } = played();
  const posting = notifications.post({
    identifier: 'mail-1',
    title: 'Ana',
    body: 'Hola',
    data: { thread: 3 },
    actions: [{ id: 'reply', label: 'Reply', reply: 'Message' }, { id: 'read', label: 'Mark as Read', background: true }],
  });
  assert.deepEqual(posted[0], {
    serial: 1,
    notification: {
      identifier: 'mail-1',
      title: 'Ana',
      subtitle: '',
      body: 'Hola',
      data: '{"thread":3}',
      actions: [
        { id: 'reply', label: 'Reply', reply: 'Message', background: false },
        { id: 'read', label: 'Mark as Read', reply: null, background: true },
      ],
      banner: true,
      sound: true,
    },
  });
  emit('OxyOSNotificationResult', { serial: 1, notification: 7, outcome: 'posted' });
  assert.equal(await posting, 7);
});

test("a post the session or the host refused rejects with the code it is", async () => {
  const { notifications, emit } = played();
  const throttled = notifications.post({ identifier: 'a' });
  emit('OxyOSNotificationResult', { serial: 1, notification: 0, outcome: 'throttled' });
  await assert.rejects(throttled, (error) => isPlatformError(error, 'busy') && error.reason === 'throttled');
  const refused = notifications.post({ identifier: 'b' });
  emit('OxyOSNotificationResult', { serial: 2, notification: 0, outcome: 'refused' });
  await assert.rejects(refused, (error) => isPlatformError(error, 'notPermitted') && error.reason === 'notLaunched');
  await assert.rejects(played({ sent: 'notPermitted' }).notifications.post({ identifier: 'c' }), (error) =>
    isPlatformError(error, 'notPermitted'),
  );
  await assert.rejects(played({ sent: 'notSent' }).notifications.post({ identifier: 'd' }), (error) =>
    isPlatformError(error, 'unavailable'),
  );
});

test('a post with too many actions or too much data is invalid, and nothing is sent', async () => {
  const { notifications, posted } = played();
  const action = { id: 'x', label: 'X' };
  await assert.rejects(notifications.post({ identifier: 'a', actions: [action, action, action, action] }), (error) =>
    isPlatformError(error, 'invalid'),
  );
  await assert.rejects(notifications.post({ identifier: 'a', data: { text: 'x'.repeat(4096) } }), (error) =>
    isPlatformError(error, 'invalid'),
  );
  assert.equal(posted.length, 0);
});

test('without a compositor every call rejects with unavailable; aborted, a post rejects with cancelled', async () => {
  const throwing: NotificationsModule = {
    post: () => {
      throw new Error('OxyOSNotifications: notifications need a connection to the compositor on OxyOS');
    },
    withdraw: () => {
      throw new Error('no compositor');
    },
    takeResponses: () => [],
  };
  const bus: Events = { addListener: () => ({ remove() {} }) };
  const none = createNotifications(throwing, bus);
  await assert.rejects(none.post({ identifier: 'a' }), (error) => isPlatformError(error, 'unavailable'));
  await assert.rejects(none.withdrawAll(), (error) => isPlatformError(error, 'unavailable'));
  const absent = createNotifications(null, bus);
  assert.equal(absent.available, false);
  await assert.rejects(absent.withdraw(3), (error) => isPlatformError(error, 'unavailable'));
  const { notifications, emit } = played();
  const controller = new AbortController();
  const posting = notifications.post({ identifier: 'a' }, { signal: controller.signal });
  controller.abort();
  await assert.rejects(posting, (error) => isPlatformError(error, 'cancelled'));
  emit('OxyOSNotificationResult', { serial: 1, notification: 7, outcome: 'posted' });
  await settle();
});

test('withdrawing one, or all', async () => {
  const { notifications, withdrawn } = played();
  await notifications.withdraw(7);
  await notifications.withdrawAll();
  assert.deepEqual(withdrawn, [7, 0]);
});

test('responses are told once each, one waiting from before to the first follower', () => {
  const { notifications, waiting, emit } = played();
  // The press that started the application, before anything followed.
  waiting.push(response());
  emit('OxyOSNotificationResponse', {});
  assert.equal(waiting.length, 1);
  const told: unknown[] = [];
  const stop = notifications.onResponse((answer) => told.push(answer));
  assert.deepEqual(told, [
    { notification: 7, identifier: 'mail-1', title: 'Ana', subtitle: '', body: 'Hola', data: { thread: 3 }, action: null, reply: null },
  ]);
  waiting.push(response({ action: 1, actionId: 'reply', reply: 'Vale', data: 'not json' }));
  emit('OxyOSNotificationResponse', {});
  assert.deepEqual(told[1], {
    notification: 7,
    identifier: 'mail-1',
    title: 'Ana',
    subtitle: '',
    body: 'Hola',
    data: {},
    action: 'reply',
    reply: 'Vale',
  });
  stop();
  waiting.push(response());
  emit('OxyOSNotificationResponse', {});
  assert.equal(told.length, 2);
  assert.equal(waiting.length, 1);
});

test('a notification no longer kept is told by its number until unsubscribed', () => {
  const { notifications, emit } = played();
  const removed: number[] = [];
  const stop = notifications.onRemoved((number) => removed.push(number));
  emit('OxyOSNotificationRemoved', { notification: 7 });
  stop();
  emit('OxyOSNotificationRemoved', { notification: 8 });
  assert.deepEqual(removed, [7]);
});
