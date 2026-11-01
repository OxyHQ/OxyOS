// Reanimated's `useReducedMotion` on OxyOS: the session's Reduce Motion as it is now, rendering
// again when the session changes it (../ReducedMotion.oxyos.js), where Reanimated's own returns the
// setting the application started with.
import { useSyncExternalStore } from 'react';
import { isReducedMotionEnabledInSystem, subscribe } from '../ReducedMotion.oxyos';

export function useReducedMotion() {
  return useSyncExternalStore(subscribe, isReducedMotionEnabledInSystem);
}
