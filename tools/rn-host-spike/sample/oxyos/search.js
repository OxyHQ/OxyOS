/**
 * The session's search index (docs/adr/0051-search.mdx) over the Bloom host's `OxyOSSearch`: Bloom's
 * `PlatformSearch` for an application's window, and the shell's Spotlight and Settings' Spotlight
 * page. Every request is one object of the index's vocabulary, `{query: {...}}`, `{watch: {}}` and
 * so on, sent as JSON with a serial of this window's; the index answers once, naming it
 * (`OxyOSSearchText`), with one object: `{refused: {reason}}` rejects with an Error whose `code` is
 * the reason, and any other (`{results: {...}}`, `{preferences: {...}}`, `{done: {}}`) resolves with
 * its value. Serial 0 is the index's own: `{changed: {}}`, told while this window watches, calls the
 * watch's listeners; `{status: {...}}`, told while this window follows the index's state
 * (docs/adr/0058-the-platform-api.mdx's followed state), is the state's new value. The compositor
 * tells the index who asks (this application, the shell, Settings); nothing here says it, and only
 * Settings may change the preferences or rebuild the index.
 */
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

const native = TurboModuleRegistry.get('OxyOSSearch');

/** Whether this window reaches the search index (the compositor has it: protocol version 30). */
export const searchAvailable = Boolean(native?.getConstants()?.available);

/** Why the index did not answer as asked: its reason (`unavailable`, `notPermitted`, `busy`...). */
export class SearchError extends Error {
  constructor(code, message) {
    super(message ?? `search: ${code}`);
    this.name = 'SearchError';
    this.code = code;
  }
}

/** Requests waiting for their answer, by serial. */
const waiting = new Map();
/** Who follows the index's changes; the index is asked to watch while there is one. */
const listeners = new Set();
/** Who follows the index's state; the index tells it while there is one. */
const stateListeners = new Set();
/** The index's state as last told; null before the first, and once nobody follows it. */
let stateLast = null;

function tellState(state) {
  if (stateListeners.size === 0) return;
  stateLast = state;
  for (const listener of [...stateListeners]) listener(state);
}

if (native) {
  DeviceEventEmitter.addListener('OxyOSSearchText', ({ serial, text }) => {
    let answer;
    try {
      answer = JSON.parse(text);
    } catch {
      answer = null;
    }
    if (serial === 0) {
      if (answer && answer.changed !== undefined) {
        for (const listener of [...listeners]) listener();
      }
      if (answer && answer.status) tellState(answer.status);
      return;
    }
    const asked = waiting.get(serial);
    if (!asked) return;
    waiting.delete(serial);
    const keys = answer && typeof answer === 'object' && !Array.isArray(answer) ? Object.keys(answer) : [];
    if (keys.length !== 1) {
      asked.reject(new SearchError('invalid', 'search: an answer that is not one object of one key'));
    } else if (keys[0] === 'refused') {
      asked.reject(new SearchError(String(answer.refused?.reason ?? 'refused')));
    } else {
      asked.resolve(answer[keys[0]]);
    }
  });
}

/** One request of the index's vocabulary: the value of its answer, or its refusal thrown. */
function ask(request) {
  if (!searchAvailable) return Promise.reject(new SearchError('unavailable'));
  return new Promise((resolve, reject) => {
    const serial = native.request(JSON.stringify(request));
    if (!serial) {
      reject(new SearchError('unavailable'));
      return;
    }
    waiting.set(serial, { resolve, reject });
  });
}

/** A request whose answer only says it was done, or that it was refused. */
function told(request) {
  return ask(request).then(() => undefined);
}

/** Asks without waiting: a failure is nobody's to handle. */
function tell(request) {
  ask(request).catch(() => {});
}

/** The index for this window: `PlatformSearch` and Settings' preferences and status. */
export function createSearch() {
  return Object.freeze({
    /** Results for what is typed: `{items, topHit, indexing}`. */
    query(query) {
      return ask({ query });
    },
    /** `listener` called whenever what was found may have changed; returns the unsubscribe. The
        first listener asks the index to watch, and the last one gone asks it to stop. */
    watch(listener) {
      const first = listeners.size === 0;
      listeners.add(listener);
      if (first) tell({ watch: {} });
      return () => {
        if (!listeners.delete(listener)) return;
        if (listeners.size === 0) tell({ unwatch: {} });
      };
    },
    /** The person opened the result `id`: it ranks higher from now on. */
    used(id) {
      tell({ used: { id } });
    },
    /** What this application gives the index of its own content, and takes back. */
    index: Object.freeze({
      donate: (items) => told({ donate: { items } }),
      remove: (ids) => told({ remove: { ids } }),
      removeAll: () => told({ removeAll: {} }),
    }),
    /** The index's preferences (Settings alone changes them). */
    preferences: () => ask({ preferences: {} }),
    setPreferences: (preferences) => ask({ setPreferences: preferences }),
    /** What the index holds and is doing, once. */
    status: () => ask({ status: {} }),
    /** What the index holds and is doing, followed (`@oxyos/platform`'s `Followed`: `current`,
        `subscribe`, what React's `useSyncExternalStore` takes): `{indexing, items, partial, rebuilding, found, ...}`, told by
        the index as it changes, and only while someone follows it. The first subscriber asks the
        index to tell it; the last one gone asks it to stop. */
    state: Object.freeze({
      current: () => stateLast,
      subscribe(listener) {
        const first = stateListeners.size === 0;
        stateListeners.add(listener);
        if (first) ask({ followStatus: {} }).then(tellState, () => {});
        return () => {
          if (!stateListeners.delete(listener)) return;
          if (stateListeners.size === 0) {
            stateLast = null;
            tell({ unfollowStatus: {} });
          }
        };
      },
    }),
    /** Discards the index and builds it again from the files (Settings alone); the old one answers
        until the new one is ready. */
    rebuild: () => told({ rebuild: {} }),
  });
}
