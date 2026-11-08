// State that changes on its own (docs/adr/0058-the-platform-api.mdx, decision 3, "Followed
// state"): the last told, or null before the first, and who follows it, the shape React's
// useSyncExternalStore takes. Following begins with the first subscriber and ends with the last,
// so what costs something to watch is watched exactly while something shows it.

export interface Followed<T> {
  /** The last state told, or null before the first. */
  current(): T | null;
  /** Follows it; the function returned stops following. */
  subscribe(listener: (state: T) => void): () => void;
}

/** A Followed the platform tells (`tell`), its watch begun by `start` and ended by `stop`. */
export interface Told<T> extends Followed<T> {
  tell(state: T): void;
}

export function followed<T>(watch?: { start?: () => void; stop?: () => void }): Told<T> {
  let last: T | null = null;
  const listeners = new Set<(state: T) => void>();
  return {
    current: () => last,
    subscribe(listener) {
      listeners.add(listener);
      if (listeners.size === 1) watch?.start?.();
      let following = true;
      return () => {
        if (!following) return;
        following = false;
        listeners.delete(listener);
        if (listeners.size === 0) watch?.stop?.();
      };
    },
    tell(state) {
      last = state;
      for (const listener of [...listeners]) listener(state);
    },
  };
}
