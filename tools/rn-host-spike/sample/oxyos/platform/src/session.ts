// The one connection to the session's vocabulary (the Bloom host's `OxyOSSystem`): requests by
// serial and their answers, and the state the session tells unasked as it changes. The frameworks
// are written over it; it is the runtime's own, never a contract (ADR 0058, decision 2), and its
// raw `request` is used outside the package only by `oxyos/system.js` until its last caller moves
// (ADR 0058, step 9).
import { followed, type Told } from './followed.ts';

/** The host's module: a request, as the session's JSON, sent with a serial (0: not sent); a
    capability asked at first use, its serial (0: not sent), answered by `OxyOSCapabilityResult`. */
export interface SystemModule {
  request(json: string): number;
  requestCapability?(capability: string): number;
  getLaunch?(): Launch | null;
}

/** What this launch may use (docs/adr/0030-installing-applications.mdx, decision 8), as the host
    holds it: an installed version (`narrowed`) only the capabilities its account granted, and
    those granted at first use since; the system grants the image's policy gave it. */
export interface Launch {
  origin: string;
  narrowed: boolean;
  capabilities: string[];
  grants: string[];
}
export interface Subscription {
  remove(): void;
}
/** Where the host's events arrive (React Native's DeviceEventEmitter). */
export interface Events {
  addListener(event: string, listener: (event: any) => void): Subscription;
}

export type Json = Record<string, unknown>;
export interface RequestOptions {
  /** Aborting answers `{failed: {reason: 'cancelled'}}` at once; the session's answer is dropped. */
  signal?: AbortSignal;
  /** A long request's progress (an install's), as the session tells it. */
  onProgress?: (progress: unknown) => void;
}

/**
 * What the session tells as it changes, by the key its answers carry it under. Each is asked for
 * as its first follower subscribes, and told again by the session as it changes.
 */
export interface SessionState {
  updateState: Told<Json>;
  network: Told<Json>;
  bluetooth: Told<Json>;
  power: Told<Json>;
  /** Backups (docs/adr/0059-backups.mdx), as the session's answer carries them: `{backup: State}`,
      or `{backup: null}` while the backup service is not there (null before the session said). */
  backup: Told<{ backup: Json | null }>;
  inputSources: Told<Json>;
  sound: Told<Json>;
  /** The account's applications and their capabilities' decisions, and which run in the
      background now (`privacy`; ADR 0030, decision 8; docs/adr/0060-login-items-and-background-
      activity.mdx): told by the answers to `{privacy}` and again, unasked, when it changed
      elsewhere. Asked by the page that shows it, as the updates are. */
  privacy: Told<Json[]>;
  /** AutoFill's list the session tells the shell to draw, null for none (told only unasked). */
  autofill: Told<Json | null>;
  /** The chosen input's level (0 to 1): metered from the first follower until the last stops, so
      nothing listens to the input while nothing shows its level (ADR 0056, decision 7). */
  inputLevel: Told<number>;
  /** Activity Monitor's samples (the `activity` grant, docs/adr/0063-activity-monitor.mdx), of the
      view `activityView` chose: followed from the first follower until the last stops, and sampled
      by the session only while the follower's window is shown. */
  activity: Told<Json>;
}

export interface Session {
  /** Whether the host has the module: without it, every request answers `unavailable`. */
  available: boolean;
  /** One request; its final answer, `{refused}` and `{failed}` among them (never a rejection). */
  request(body: Json, options?: RequestOptions): Promise<unknown>;
  /**
   * Asks for a capability asked at first use (`notifications`, `files`, `camera`, `microphone`,
   * `location`; docs/adr/0030-installing-applications.mdx, decision 8): whether it is granted. The
   * session answers from the account's decision, or asks the person in the system's sheet when
   * the request follows an input of theirs in the application. One question at a time per
   * capability: those asked meanwhile share its answer.
   */
  requestCapability(capability: string): Promise<boolean>;
  /** What this launch may use, as the host holds it now; null without the module. */
  launch(): Launch | null;
  state: SessionState;
  /** The view Activity Monitor's samples are of (`cpu`, `memory`, `energy`, `disk`, `network`):
      kept until following begins, and told the session at once while it is followed. */
  activityView(view: string): void;
}

/**
 * Each followed state asked for as it is first followed, by its key, and the request that asks
 * for it. The account's updates are not: asking for them asks the install service, which the
 * store's installer follows as it starts, and an install would wait behind it; they are asked by
 * the page that shows them (`settings.checkUpdates`, or `{state}` through system.js).
 */
const ASKED_BY = {
  network: { networks: {} },
  bluetooth: { bluetooth: {} },
  inputSources: { inputSources: {} },
  sound: { sound: {} },
  power: { power: {} },
  backup: { backup: {} },
} as const;
const FOLLOWED_KEYS = ['updateState', 'activity', ...Object.keys(ASKED_BY).filter((key) => key !== 'backup')] as (
  | 'updateState'
  | 'activity'
  | Exclude<keyof typeof ASKED_BY, 'backup'>
)[];

export function createSession(native: SystemModule | null | undefined, events: Events): Session {
  const waiting = new Map<number, { resolve: (answer: unknown) => void; onProgress?: (progress: unknown) => void }>();
  // Hoisted: the followed states ask through it as they are first followed.
  let request: (body: Json, options?: RequestOptions) => Promise<unknown>;
  const asked = <T = Json>(key: keyof typeof ASKED_BY) => followed<T>({ start: () => void request(ASKED_BY[key]) });
  let activity = { view: 'cpu', followed: false };
  const state: SessionState = {
    updateState: followed(),
    network: asked('network'),
    bluetooth: asked('bluetooth'),
    power: asked('power'),
    backup: asked<{ backup: Json | null }>('backup'),
    inputSources: asked('inputSources'),
    sound: asked('sound'),
    privacy: followed(),
    autofill: followed(),
    inputLevel: followed({
      start: () => void request({ soundMeter: { on: true } }),
      stop: () => void request({ soundMeter: { on: false } }),
    }),
    activity: followed({
      start() {
        activity = { ...activity, followed: true };
        void request({ activityFollow: { view: activity.view } });
      },
      stop() {
        activity = { ...activity, followed: false };
        void request({ activityFollow: { view: null } });
      },
    }),
  };

  const tellAll = (answer: unknown) => {
    if (!answer || typeof answer !== 'object') return;
    const told = answer as Json;
    for (const key of FOLLOWED_KEYS) {
      if (told[key]) state[key].tell(told[key] as Json);
    }
    // Null is a state of its own: no backup service.
    if ('backup' in told) state.backup.tell({ backup: (told.backup as Json | null) ?? null });
    if (Array.isArray(told.privacy)) state.privacy.tell(told.privacy as Json[]);
  };

  if (native) {
    events.addListener('OxyOSSystemAnswer', (event: { serial: number; answer?: unknown; text?: string }) => {
      const { serial } = event;
      // A long answer (SystemData, docs/adr/0063-activity-monitor.mdx) comes as the session's
      // text, which JSON.parse reads far faster than the host could build its values.
      let answer = event.answer;
      if (typeof event.text === 'string') {
        try {
          answer = JSON.parse(event.text);
        } catch {
          answer = { failed: { reason: 'invalidAnswer' } };
        }
      }
      // The state the session follows, told unasked as it changes.
      if (serial === 0) {
        tellAll(answer);
        const told = (answer ?? {}) as Json;
        if ('autofill' in told) state.autofill.tell((told.autofill as Json | null) ?? null);
        if (typeof told.inputLevel === 'number') state.inputLevel.tell(told.inputLevel);
        return;
      }
      const asked = waiting.get(serial);
      if (!asked) return;
      const told = answer as Json | null;
      if (told && told.progress !== undefined) {
        asked.onProgress?.(told.progress);
        return;
      }
      waiting.delete(serial);
      tellAll(answer);
      asked.resolve(answer);
    });
  }

  const cancelled = { failed: { reason: 'cancelled' } };
  request = (body: Json, options: RequestOptions = {}): Promise<unknown> => {
    if (!native) return Promise.resolve({ failed: { reason: 'unavailable' } });
    const { signal, onProgress } = options;
    if (signal?.aborted) return Promise.resolve(cancelled);
    return new Promise((resolve) => {
      const serial = native.request(JSON.stringify(body));
      if (!serial) {
        resolve({ failed: { reason: 'notSent' } });
        return;
      }
      // Aborted, the request is forgotten: its answer, when it comes, finds no one waiting.
      const abort = () => {
        if (!waiting.delete(serial)) return;
        resolve(cancelled);
      };
      signal?.addEventListener('abort', abort, { once: true });
      waiting.set(serial, {
        resolve(answer) {
          signal?.removeEventListener('abort', abort);
          resolve(answer);
        },
        onProgress,
      });
    });
  };

  const asking = new Map<string, Promise<boolean>>();
  const capabilityAnswers = new Map<number, (granted: boolean) => void>();
  if (native?.requestCapability) {
    events.addListener('OxyOSCapabilityResult', ({ serial, granted }: { serial: number; granted: boolean }) => {
      capabilityAnswers.get(serial)?.(Boolean(granted));
      capabilityAnswers.delete(serial);
    });
  }
  const requestCapability = (capability: string): Promise<boolean> => {
    const known = asking.get(capability);
    if (known) return known;
    const asked = new Promise<boolean>((resolve) => {
      let serial = 0;
      try {
        serial = native?.requestCapability?.(capability) ?? 0;
      } catch {
        // No connection to the session: nothing is granted.
      }
      if (!serial) resolve(false);
      else capabilityAnswers.set(serial, resolve);
    }).finally(() => asking.delete(capability));
    asking.set(capability, asked);
    return asked;
  };

  return {
    available: Boolean(native),
    request,
    requestCapability,
    launch: () => native?.getLaunch?.() ?? null,
    state,
    activityView(view) {
      if (view === activity.view) return;
      activity = { ...activity, view };
      if (activity.followed) void request({ activityFollow: { view } });
    },
  };
}
