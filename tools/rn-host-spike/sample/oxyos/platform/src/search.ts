// `search` (docs/adr/0058-the-platform-api.mdx, decision 1; docs/adr/0051-search.mdx): the session's
// search index, which every window reaches: searching the person's things, donating an
// application's own, and (Settings alone) the index's preferences and rebuilding it. Every request
// is one object of the index's vocabulary (`{query: {...}}`, `{watch: {}}`...), answered once with
// one object: `{refused: {reason}}` rejects with a PlatformError of `search`, any other resolves with
// its value. The compositor tells the index who asks (this application, the shell, Settings);
// nothing here says it, and the index filters what each may see (ADR 0051, decision 6).
import { errorOf, PlatformError } from './errors.ts';
import { followed } from './followed.ts';
import type { Followed } from './followed.ts';
import type { CallOptions } from './frameworks.ts';
import { hostCall } from './host.ts';
import type { Events } from './session.ts';

/** The host's module (OxyOSSearch): a request as JSON, its serial (0: not sent), answered by
    `OxyOSSearchText` naming it; serial 0 is the index's own (`{changed}`, `{status}`). */
export interface SearchModule {
  getConstants(): { available?: boolean };
  request(json: string): number;
}

type Json = Record<string, unknown>;

export function createSearch(native: SearchModule | null | undefined, events: Events) {
  const available = Boolean(native?.getConstants()?.available);
  const waiting = new Map<number, (answer: unknown) => void>();
  const changes = new Set<() => void>();
  let ask: (request: Json, options?: CallOptions) => Promise<any>;
  // The index's state, told by it while something follows it: asked to tell it as the first
  // follower subscribes, and to stop as the last one goes.
  const state = followed<Json>({
    start: () => void ask({ followStatus: {} }).then((status) => state.tell(status), () => {}),
    stop: () => void ask({ unfollowStatus: {} }).catch(() => {}),
  });

  if (native && available) {
    events.addListener('OxyOSSearchText', ({ serial, text }: { serial: number; text: string }) => {
      let answer: unknown;
      try {
        answer = JSON.parse(text);
      } catch {
        answer = null;
      }
      if (serial === 0) {
        const told = (answer ?? {}) as Json;
        if (told.changed !== undefined) for (const listener of [...changes]) listener();
        if (told.status) state.tell(told.status as Json);
        return;
      }
      waiting.get(serial)?.(answer);
      waiting.delete(serial);
    });
  }

  /** One request: the value of its answer, or its refusal rejected. Aborted, it rejects with
      `cancelled` at once and the answer is dropped. */
  ask = async (request: Json, options?: CallOptions) => {
    if (!native || !available) throw new PlatformError('search', 'unavailable');
    let serial = 0;
    const answer = await hostCall(
      'search',
      () => {
        serial = native.request(JSON.stringify(request));
        if (!serial) return Promise.resolve({ refused: { reason: 'notSent' } });
        return new Promise<unknown>((resolve) => waiting.set(serial, resolve));
      },
      options,
      () => waiting.delete(serial),
    );
    const keys = answer && typeof answer === 'object' && !Array.isArray(answer) ? Object.keys(answer) : [];
    if (keys.length !== 1) throw new PlatformError('search', 'failed', 'noAnswer');
    const error = errorOf('search', answer);
    if (error) throw error;
    return (answer as Json)[keys[0]!];
  };
  const done = async (request: Json, options?: CallOptions) => {
    await ask(request, options);
  };
  /** Asks without waiting: a failure is nobody's to handle. */
  const tell = (request: Json) => void ask(request).catch(() => {});

  return {
    available,
    /** Results for what is typed: `{items, topHit, indexing}`. */
    query: (query: Json, options?: CallOptions) => ask({ query }, options),
    /** Calls `listener` whenever what was found may have changed; the function returned stops. The
        index watches while something listens. */
    onChange(listener: () => void): () => void {
      const first = changes.size === 0;
      changes.add(listener);
      if (first) tell({ watch: {} });
      return () => {
        if (!changes.delete(listener)) return;
        if (changes.size === 0) tell({ unwatch: {} });
      };
    },
    /** The person opened the result `id`: it ranks higher from now on. */
    used: (id: string) => tell({ used: { id } }),
    /** What this application gives the index of its own content, and takes back. */
    donate: (items: readonly Json[], options?: CallOptions) => done({ donate: { items } }, options),
    /** Takes back what this application gave the index, by its ids. */
    remove: (ids: readonly string[], options?: CallOptions) => done({ remove: { ids } }, options),
    /** Takes back everything this application gave the index. */
    removeAll: (options?: CallOptions) => done({ removeAll: {} }, options),
    /** The index's preferences (Settings alone changes them: `notPermitted` for any other). */
    preferences: (options?: CallOptions) => ask({ preferences: {} }, options),
    /** Changes the index's preferences (Settings alone); resolves with them. */
    setPreferences: (preferences: Json, options?: CallOptions) => ask({ setPreferences: preferences }, options),
    /** What the index holds and is doing, once. */
    status: (options?: CallOptions) => ask({ status: {} }, options),
    /** What the index holds and is doing (`{indexing, items, partial, rebuilding, found, ...}`),
        followed: told by the index as it changes, only while something follows it. */
    state: state as Followed<Json>,
    /** Discards the index and builds it again from the files (Settings alone); the old one answers
        until the new one is ready. */
    rebuild: (options?: CallOptions) => done({ rebuild: {} }, options),
  };
}

export type Search = ReturnType<typeof createSearch>;
