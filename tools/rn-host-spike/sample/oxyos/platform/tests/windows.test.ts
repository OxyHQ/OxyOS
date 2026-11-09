// The window's mapping (docs/adr/0058-the-platform-api.mdx, decision 10): its state and its first
// responder followed, menus' choices told, keys offered to each listener until one takes it.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import type { Events } from '../src/session.ts';
import { createWindows, type KeyEvent } from '../src/windows.ts';

function played() {
  const listeners = new Map<string, (event: any) => void>();
  const events: Events = {
    addListener(name, handler) {
      listeners.set(name, handler);
      return { remove() {} };
    },
  };
  let hostListener: ((event: KeyEvent) => boolean) | undefined;
  const calls: unknown[] = [];
  const windows = createWindows(
    {
      window: {
        getConstants: () => ({ available: true, metrics: { radius: 16 } }),
        getState: () => ({ active: true }),
        close: () => calls.push('close'),
        minimize: () => calls.push('minimize'),
        zoom: () => calls.push('zoom'),
        startMove: () => true,
        setSizeLimits: (...limits) => calls.push(limits),
      },
      menuBar: { getConstants: () => ({ available: true }), setMenus: (entries) => (calls.push(entries.length), 7) },
      keyboard: { getConstants: () => ({ available: true }), setListener: (listener) => (hostListener = listener) },
      editing: { getConstants: () => ({ available: true }), getState: () => null, perform: (action) => action === 'copy' },
    },
    events,
  );
  return { windows, calls, emit: (name: string, event: unknown) => listeners.get(name)?.(event), key: (event: KeyEvent) => hostListener?.(event) };
}

const key = (k: string): KeyEvent => ({ key: k, code: k, altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, repeat: false });

test("the window's state is followed; its operations reach the host", () => {
  const { windows, calls, emit } = played();
  assert.deepEqual(windows.state.current(), { active: true });
  const told: unknown[] = [];
  windows.state.subscribe((state) => told.push(state));
  emit('OxyOSWindowChanged', { active: false });
  assert.deepEqual(told, [{ active: false }]);
  windows.zoom();
  windows.setSizeLimits({ minWidth: 400, minHeight: 300 });
  assert.deepEqual(calls, ['zoom', [400, 300, 0, 0]]);
  assert.deepEqual(windows.metrics, { radius: 16 });
});

test("the menus are sent as a commit; a choice in the bar is told by its commit and index", () => {
  const { windows, emit } = played();
  assert.equal(windows.menus.set([{ parent: -1, kind: 1, flags: 0, label: 'Archivo', shortcut: '' }]), 7);
  const chosen: unknown[] = [];
  const stop = windows.menus.onChoose((choice) => chosen.push(choice));
  emit('OxyOSMenuActivated', { serial: 7, index: 0 });
  stop();
  emit('OxyOSMenuActivated', { serial: 7, index: 1 });
  assert.deepEqual(chosen, [{ serial: 7, index: 0 }]);
});

test('a key is offered to each listener until one takes it', () => {
  const { windows, key: press } = played();
  const seen: string[] = [];
  windows.keys.onKey((event) => void seen.push(`a ${event.key}`));
  const stop = windows.keys.onKey((event) => (seen.push(`b ${event.key}`), event.key === 'Enter'));
  assert.equal(press(key('Enter')), true);
  stop();
  assert.equal(press(key('Enter')), false);
  assert.deepEqual(seen, ['a Enter', 'b Enter', 'a Enter']);
});

test("the first responder's state is followed and its actions performed", () => {
  const { windows, emit } = played();
  assert.equal(windows.editing.state.current(), null);
  emit('OxyOSEditingChanged', { copy: true });
  assert.deepEqual(windows.editing.state.current(), { copy: true });
  assert.equal(windows.editing.perform('copy'), true);
  assert.equal(windows.editing.perform('paste'), false);
});

test('the modifiers held are told to each follower as they change, until it stops', () => {
  const { windows, emit } = played();
  const seen: boolean[] = [];
  const stop = windows.keys.onModifiers((modifiers) => void seen.push(modifiers.altKey));
  emit('OxyOSModifiersChanged', { altKey: true, ctrlKey: false, metaKey: false, shiftKey: false });
  stop();
  emit('OxyOSModifiersChanged', { altKey: false, ctrlKey: false, metaKey: false, shiftKey: false });
  assert.deepEqual(seen, [true]);
});
