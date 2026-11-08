// `@oxyos/platform/react` (docs/adr/0058-the-platform-api.mdx, decision 3): followed state for a
// view.
import { useCallback, useRef, useSyncExternalStore } from 'react';

import type { Followed } from './followed.ts';

/**
 * The state `source` follows, for a view, which renders again when it changes; with `select`, only
 * when the part it selects changes (compared with Object.is). Following begins as the first view
 * subscribes, so the platform watches it exactly while something shows it.
 */
export function useFollowed<T>(source: Followed<T>): T | null;
export function useFollowed<T, S>(source: Followed<T>, select: (state: T | null) => S): S;
export function useFollowed<T, S>(source: Followed<T>, select?: (state: T | null) => S): S | T | null {
  const last = useRef<{ from: T | null; by: (state: T | null) => S; selected: S } | null>(null);
  const getSnapshot = useCallback(() => {
    const state = source.current();
    if (!select) return state;
    const known = last.current;
    if (known && known.by === select && Object.is(known.from, state)) return known.selected;
    const selected = select(state);
    // The same part selected from a new state is the same snapshot: nothing renders again.
    last.current = { from: state, by: select, selected: known && Object.is(known.selected, selected) ? known.selected : selected };
    return last.current.selected;
  }, [source, select]);
  return useSyncExternalStore(source.subscribe, getSnapshot);
}
