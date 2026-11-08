// The shell's mapping (docs/adr/0058-the-platform-api.mdx, decision 10): the system's sheets
// followed, the person's answers told, AutoFill's pick and the emoji picker answered.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isPlatformError } from '../src/errors.ts';
import { createSession, type Events } from '../src/session.ts';
import { createShell } from '../src/shell.ts';

function played() {
  const listeners = new Map<string, (event: any) => void>();
  const events: Events = {
    addListener(name, handler) {
      listeners.set(name, handler);
      return { remove() {} };
    },
  };
  const sent: Record<string, unknown>[] = [];
  const answered: [number, boolean][] = [];
  const session = createSession({ request: (json) => (sent.push(JSON.parse(json)), sent.length) }, events);
  const shell = createShell(session, (sheet, accepted) => answered.push([sheet, accepted]), events);
  const emit = (name: string, event: unknown) => listeners.get(name)?.(event);
  return { shell, sent, answered, emit };
}

test('the sheets are followed as the session shows and dismisses them; an answer takes one away', () => {
  const { shell, answered, emit } = played();
  assert.deepEqual(shell.sheets.current(), []);
  emit('OxyOSSheet', { sheet: 1, text: '{"kind":"install"}' });
  emit('OxyOSSheet', { sheet: 1, text: '{"kind":"install"}' });
  emit('OxyOSSheet', { sheet: 2, text: '{"kind":"wifi-password"}' });
  assert.deepEqual(shell.sheets.current()?.map(({ sheet }) => sheet), [1, 2]);
  shell.answerSheet(1, true);
  assert.deepEqual(answered, [[1, true]]);
  emit('OxyOSSheetDismissed', { sheet: 2 });
  assert.deepEqual(shell.sheets.current(), []);
});

test("a password and a pairing code are told to the session alone, nobody waiting for its answer", () => {
  const { shell, sent, emit } = played();
  emit('OxyOSSheet', { sheet: 4, text: '{"kind":"wifi-password"}' });
  shell.answerPassword(4, 'secret');
  shell.answerCode(5, '123456');
  assert.deepEqual(sent, [{ password: { sheet: 4, password: 'secret' } }, { bluetoothCode: { sheet: 5, code: '123456' } }]);
  assert.deepEqual(shell.sheets.current(), []);
});

test("AutoFill's pick and the emoji picker resolve once the session answered; a refusal rejects", async () => {
  const { shell, sent, emit } = played();
  const chosen = shell.chooseAutofill(2);
  emit('OxyOSSystemAnswer', { serial: 1, answer: { chosen: {} } });
  await chosen;
  const emoji = shell.showEmojiPicker();
  emit('OxyOSSystemAnswer', { serial: 2, answer: { refused: { reason: 'needsInput' } } });
  await assert.rejects(emoji, (error) => isPlatformError(error, 'needsInput') && error.domain === 'shell');
  assert.deepEqual(sent, [{ chooseAutofill: { index: 2 } }, { showEmoji: {} }]);
});

test("the shell's state is followed, its events told and its intents sent, in the shell alone", () => {
  const listeners = new Map<string, (event: any) => void>();
  const events: Events = {
    addListener(name, handler) {
      listeners.set(name, handler);
      return { remove: () => listeners.delete(name) };
    },
  };
  const session = createSession({ request: () => 1 }, events);
  const focused: unknown[] = [];
  const shell = createShell(session, null, events, {
    getState: () => ({ windows: [] }),
    focusWindow: (window: number) => focused.push(window),
  });
  assert.equal(shell.isShell, true);
  assert.deepEqual(shell.state.current(), { windows: [] });
  listeners.get('OxyOSShellChanged')?.({ windows: [{ id: 3 }] });
  assert.deepEqual(shell.state.current(), { windows: [{ id: 3 }] });
  const bezels: unknown[] = [];
  const stop = shell.on('bezel', (told) => bezels.push(told));
  listeners.get('OxyOSShellBezel')?.({ control: 'output' });
  stop();
  assert.deepEqual(bezels, [{ control: 'output' }]);
  shell.intents.focusWindow(3);
  assert.deepEqual(focused, [3]);
  listeners.get('OxyOSShellDesktopKeyboard')?.({ focused: true });
  assert.equal(shell.desktopKeyboard.current(), true);

  // Any other host: its OxyOSShell throws; nothing is followed or sent.
  const elsewhere = createShell(session, null, events, {
    getState: () => {
      throw new Error("OxyOSShell: only the session's shell has it");
    },
    focusWindow: () => assert.fail('sent'),
  });
  assert.equal(elsewhere.isShell, false);
  assert.equal(elsewhere.state.current(), null);
  assert.equal(elsewhere.intents.focusWindow(3), undefined);
});
