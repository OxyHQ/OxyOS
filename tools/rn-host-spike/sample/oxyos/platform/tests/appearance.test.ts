// The appearance's mapping (docs/adr/0058-the-platform-api.mdx, decision 10): each followed as the
// host and React Native tell it.
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { createAppearance } from '../src/appearance.ts';
import type { Events } from '../src/session.ts';

function played() {
  const listeners = new Map<string, (event: unknown) => void>();
  const events: Events = {
    addListener(name, handler) {
      listeners.set(name, handler);
      return { remove() {} };
    },
  };
  let tag = 'es-ES-u-hc-h23';
  let schemeListener: ((preferences: { colorScheme: 'light' | 'dark' | null }) => void) | undefined;
  let motionListener: ((enabled: boolean) => void) | undefined;
  let motionRemoved = false;
  const appearance = createAppearance(
    {
      preferences: { getState: () => ({ accent: '#0a84ff', reduceTransparency: false }) },
      locale: { getLocale: () => tag },
      colorSchemes: {
        getColorScheme: () => 'light',
        addChangeListener: (listener) => {
          schemeListener = listener;
          return { remove() {} };
        },
      },
      motion: {
        isReduceMotionEnabled: async () => false,
        addEventListener: (_event, listener) => {
          motionListener = listener;
          return {
            remove: () => {
              motionRemoved = true;
            },
          };
        },
      },
    },
    events,
  );
  return {
    appearance,
    emit: (name: string, event?: unknown) => listeners.get(name)?.(event),
    setTag: (next: string) => (tag = next),
    scheme: (colorScheme: 'light' | 'dark') => schemeListener?.({ colorScheme }),
    motion: (enabled: boolean) => motionListener?.(enabled),
    motionRemoved: () => motionRemoved,
  };
}

test('the drawing preferences and the locale are followed as the host tells them', () => {
  const { appearance, emit, setTag } = played();
  assert.deepEqual(appearance.drawing.current(), { accent: '#0a84ff', reduceTransparency: false });
  emit('OxyOSPreferencesChanged', { accent: '', reduceTransparency: true });
  assert.deepEqual(appearance.drawing.current(), { accent: null, reduceTransparency: true });
  assert.equal(appearance.locale.current(), 'es-ES-u-hc-h23');
  setTag('es-MX-u-hc-h12');
  emit('OxyOSLocaleChanged');
  assert.equal(appearance.locale.current(), 'es-MX-u-hc-h12');
});

test('light or dark as React Native tells it; reduced motion asked while followed', async () => {
  const { appearance, scheme, motion, motionRemoved } = played();
  assert.equal(appearance.colorScheme.current(), 'light');
  scheme('dark');
  assert.equal(appearance.colorScheme.current(), 'dark');
  const told: boolean[] = [];
  const stop = appearance.reducedMotion.subscribe((enabled) => told.push(enabled));
  await new Promise((resolve) => setImmediate(resolve));
  motion(true);
  stop();
  assert.deepEqual(told, [false, true]);
  assert.equal(motionRemoved(), true);
});

test('without a compositor there are no drawing preferences', () => {
  const appearance = createAppearance(
    { preferences: { getState: () => null }, locale: null, colorSchemes: null, motion: null },
    { addListener: () => ({ remove() {} }) },
  );
  assert.equal(appearance.drawing.current(), null);
  assert.equal(appearance.locale.current(), null);
});
