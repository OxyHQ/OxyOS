// Reanimated's reduced motion on OxyOS: the session's Reduce Motion (docs/adr/0019-settings.mdx), as
// it is now, not as it was when the application started. Reanimated's own module
// (src/ReducedMotion.native.ts) reads `_REANIMATED_IS_REDUCED_MOTION` once, which the host sets from
// the session's preference when JavaScript installs Reanimated (reanimated-host.h); this one starts
// from it and follows the preference as the session changes it, as React Native's AccessibilityInfo
// tells it (`reduceMotionChanged`), so `useReducedMotion()` and every animation with
// `ReduceMotion.System` (the default) honour a change at once, no restart. A `ReducedMotionConfig`
// holds the setting while it is mounted (`hold`): `Always` and `Never` keep it whatever the session
// says, `System` follows the session; the innermost one mounted decides.
//
// metro.config.js resolves this file wherever Reanimated names src/ReducedMotion, as it resolves
// React Native's files to their OxyOS variants.
import { AccessibilityInfo } from 'react-native';
import { ReduceMotion } from 'react-native-reanimated/src/commonTypes';
import { createReducedMotionManager } from 'react-native-reanimated/src/ReducedMotionCommon';

let system = !!globalThis._REANIMATED_IS_REDUCED_MOTION;
const listeners = new Set();
/** The modes the mounted ReducedMotionConfigs hold, the innermost (the latest mounted) last. */
const held = [];

/** Whether the session's Reduce Motion is on now. */
export function isReducedMotionEnabledInSystem() {
  return system;
}

/** What animations read (`uiValue` on the UI runtime): the setting, or what a config holds. */
export const ReducedMotionManager = createReducedMotionManager(system);

function apply() {
  const mode = held.length > 0 ? held[held.length - 1].mode : ReduceMotion.System;
  const enabled = mode === ReduceMotion.System ? system : mode === ReduceMotion.Always;
  if (enabled !== ReducedMotionManager.jsValue) ReducedMotionManager.setEnabled(enabled);
}

/** Holds `mode` until the returned function releases it (ReducedMotionConfig). */
export function hold(mode) {
  const entry = { mode };
  held.push(entry);
  apply();
  return () => {
    held.splice(held.indexOf(entry), 1);
    apply();
  };
}

/** Calls `listener` whenever the session's Reduce Motion changes (useReducedMotion). */
export function subscribe(listener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function told(enabled) {
  if (typeof enabled !== 'boolean' || enabled === system) return;
  system = enabled;
  apply();
  for (const listener of [...listeners]) listener();
}

AccessibilityInfo.addEventListener('reduceMotionChanged', told);
// A change the session told between the host reading the setting and this listener: asked once.
AccessibilityInfo.isReduceMotionEnabled().then(told, () => {});
