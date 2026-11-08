// The pasteboard's and the sounds' mapping (docs/adr/0058-the-platform-api.mdx, decision 10):
// what the host's modules answer, as the platform's calls resolve and reject.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isPlatformError } from '../src/errors.ts';
import { createPasteboard, type ClipboardModule } from '../src/pasteboard.ts';
import type { Events } from '../src/session.ts';
import { createSounds } from '../src/sounds.ts';

function events() {
  let listener: ((event: { text?: boolean }) => void) | undefined;
  const bus: Events = {
    addListener(name, handler) {
      assert.equal(name, 'OxyOSClipboardChanged');
      listener = handler;
      return { remove: () => (listener = undefined) };
    },
  };
  return { bus, change: (text: boolean) => listener?.({ text }) };
}

test('a copy refused by the compositor rejects with needsInput; a paste reads its text', async () => {
  let copied = false;
  const native: ClipboardModule = {
    hasString: async () => true,
    setString: async () => copied,
    getString: async () => 'Hola',
  };
  const pasteboard = createPasteboard(native, events().bus);
  await assert.rejects(pasteboard.writeText('x'), (error) => isPlatformError(error, 'needsInput'));
  copied = true;
  await pasteboard.writeText('x');
  assert.equal(await pasteboard.readText(), 'Hola');
  assert.equal(await pasteboard.hasText(), true);
});

test('without a window the host throws at once: a rejection with unavailable, never a throw', async () => {
  const native: ClipboardModule = {
    hasString: () => {
      throw new Error('OxyOSClipboard: the clipboard needs a window on OxyOS');
    },
    setString: () => {
      throw new Error('no window');
    },
    getString: () => {
      throw new Error('no window');
    },
  };
  const pasteboard = createPasteboard(native, events().bus);
  await assert.rejects(pasteboard.readText(), (error) => isPlatformError(error, 'unavailable'));
  await assert.rejects(pasteboard.writeText('x'), (error) => isPlatformError(error, 'unavailable'));
  const none = createPasteboard(null, events().bus);
  assert.equal(none.available, false);
  await assert.rejects(none.hasText(), (error) => isPlatformError(error, 'unavailable'));
});

test('a paste aborted rejects with cancelled; a change is told until unsubscribed', async () => {
  const native: ClipboardModule = {
    hasString: async () => false,
    setString: async () => true,
    getString: () => new Promise(() => {}),
  };
  const { bus, change } = events();
  const pasteboard = createPasteboard(native, bus);
  const controller = new AbortController();
  const reading = pasteboard.readText({ signal: controller.signal });
  controller.abort();
  await assert.rejects(reading, (error) => isPlatformError(error, 'cancelled'));
  const told: boolean[] = [];
  const stop = pasteboard.onChange(({ hasText }) => told.push(hasText));
  change(true);
  stop();
  change(false);
  assert.deepEqual(told, [true]);
});

test('a sound is played by meaning; without an input to cite it rejects with needsInput', async () => {
  let played: string[] = [];
  let sent = true;
  const sounds = createSounds({ getConstants: () => ({ available: true }), play: (name) => (played.push(name), sent) });
  await sounds.play('move-to-trash');
  assert.deepEqual(played, ['move-to-trash']);
  sent = false;
  await assert.rejects(sounds.play('empty-trash'), (error) => isPlatformError(error, 'needsInput'));
  played = [];
  await assert.rejects(sounds.play('beep' as never), (error) => isPlatformError(error, 'invalid'));
  assert.deepEqual(played, []);
  const none = createSounds({ getConstants: () => ({ available: false }), play: () => true });
  assert.equal(none.available, false);
  await assert.rejects(none.play('empty-trash'), (error) => isPlatformError(error, 'unavailable'));
});
