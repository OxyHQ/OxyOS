// `application` (docs/adr/0058-the-platform-api.mdx, decision 1): this application, as the session
// started it: its id, what it was started to open, opening a URL elsewhere (a Mac's
// `NSWorkspace.open`), and its permissions (`requestPermission`, ADR 0030, decision 8). Every
// application may use it.
//
// Its login item and background activity (docs/adr/0060-login-items-and-background-activity.mdx),
// as a Mac's `SMAppService` gives an application its own: `loginItem`, the application opened at
// login (the person's Open at Login list, the dock's and Settings'), and `backgroundActivity`, its
// process running on in the background once its window closes. The session decides by the
// process's launch record and its account's decisions; nothing here grants anything. Registering a
// new item tells the person, who manages it in Settings › General › Login Items & Extensions. And
// its answer when the session asks it to quit before it ends
// (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx).
import type { ApplicationConstants, Spec } from '../specs/NativeOxyOSApplication.ts';
import { errorOf, PlatformError } from './errors.ts';
import { followed, type Followed } from './followed.ts';
import type { CallOptions } from './frameworks.ts';
import { hostCall } from './host.ts';
import type { Events, Session } from './session.ts';

/** The host's module (OxyOSApplication), as its spec says it. Its background items and quitting
    are asked only where its constants say the host has them. */
export type ApplicationModule = Pick<Spec, 'getConstants' | 'open'> &
  Partial<Pick<Spec, 'backgroundItem' | 'handlesQuit' | 'replyToQuit'>>;

/** A capability an application asks the person for at first use. */
export type Capability = 'notifications' | 'files' | 'camera' | 'microphone' | 'location';
/** Whether this application has a capability now. */
export type Permission = 'granted' | 'denied';

/** How an item stands, as `SMAppService.Status` says. */
export type BackgroundItemStatus =
  /** Not registered. */
  | 'notRegistered'
  /** Registered and allowed. */
  | 'enabled'
  /** Registered, but the person turned it off: registering again changes nothing until they turn
      it on in Login Items & Extensions. */
  | 'requiresApproval'
  /** The platform gives this application none: not a process the session launched, a development
      entry, or an application that did not declare it. */
  | 'notPermitted';

/** One of the application's background items: how it stands (followed), and registering it. */
export interface BackgroundItem extends Followed<BackgroundItemStatus> {
  /** Whether the session can be asked (protocol version 33). */
  readonly available: boolean;
  /** How it stands now. */
  status(options?: CallOptions): Promise<BackgroundItemStatus>;
  /** Registers it: resolves `'enabled'`, or rejects (`notPermitted`, with `reason`
      `'requiresApproval'` when the person turned it off; `failed` when it was not registered;
      `unavailable`; `cancelled`). */
  register(options?: CallOptions): Promise<'enabled'>;
  /** Unregisters it: how it then stands. */
  unregister(options?: CallOptions): Promise<BackgroundItemStatus>;
}

/** Why the session asks the application to quit (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx). */
export type QuitReason = 'logOut' | 'restart' | 'shutDown';
/** An application's answer: quit (it then ends), or cancel the whole log out, restart or shut down. */
export type QuitAnswer = 'quit' | 'cancel';
/** Asked before the session logs out, restarts or shuts down, as a Mac's
    `applicationShouldTerminate:`: an unsaved document may be saved first (the promise), or the end
    cancelled. */
export type QuitHandler = (reason: QuitReason) => QuitAnswer | Promise<QuitAnswer>;

const REASONS = new Set<string>(['logOut', 'restart', 'shutDown']);

/** The handlers' answer, asked in order: any cancel cancels, and so does a handler that throws or
    rejects (an unsaved document is never lost to a bug); none cancelling, quit. */
export async function answerQuit(handlers: readonly QuitHandler[], reason: QuitReason): Promise<QuitAnswer> {
  for (const handler of handlers) {
    let answer: QuitAnswer;
    try {
      answer = await handler(reason);
    } catch {
      return 'cancel';
    }
    if (answer !== 'quit') return 'cancel';
  }
  return 'quit';
}

const STATUSES = new Set<string>(['notRegistered', 'enabled', 'requiresApproval', 'notPermitted']);

const NONE: ApplicationConstants = {
  id: null,
  recorded: false,
  module: '',
  manifest: null,
  dataDirectory: null,
  cacheDirectory: null,
  deviceName: '',
  systemVersion: '',
  launchURL: null,
  backgroundItems: false,
  quitRequests: false,
};

export function createApplication(native: ApplicationModule | null | undefined, session: Session, events: Events) {
  const constants: ApplicationConstants = native?.getConstants() ?? NONE;
  const available = Boolean(constants.backgroundItems && native?.backgroundItem);
  const quitRequests = Boolean(constants.quitRequests && native?.handlesQuit && native?.replyToQuit);
  const quitHandlers: QuitHandler[] = [];
  const waiting = new Map<number, (status: BackgroundItemStatus) => void>();
  const items = new Map<string, (status: BackgroundItemStatus) => void>();

  if (available) {
    events.addListener(
      'OxyOSBackgroundItemState',
      ({ serial, kind, status }: { serial: number; kind: string; status: string }) => {
        if (!STATUSES.has(status)) return;
        const told = status as BackgroundItemStatus;
        // An answer and a change the person made alike tell the item's followers.
        items.get(kind)?.(told);
        const resolve = waiting.get(serial);
        if (!resolve) return;
        waiting.delete(serial);
        resolve(told);
      },
    );
  }

  function item(kind: 'loginItem' | 'background'): BackgroundItem {
    const ask = (action: 'status' | 'register' | 'unregister', options: CallOptions = {}) =>
      new Promise<BackgroundItemStatus>((resolve, reject) => {
        const { signal } = options;
        if (!available || !native?.backgroundItem) {
          reject(new PlatformError('application', 'unavailable'));
          return;
        }
        if (signal?.aborted) {
          reject(new PlatformError('application', 'cancelled'));
          return;
        }
        const serial = native.backgroundItem(kind, action);
        if (!serial) {
          reject(new PlatformError('application', 'unavailable', 'notSent'));
          return;
        }
        // Aborted, the ask is forgotten: its answer still tells the item's followers.
        const abort = () => {
          if (!waiting.delete(serial)) return;
          reject(new PlatformError('application', 'cancelled'));
        };
        signal?.addEventListener('abort', abort, { once: true });
        waiting.set(serial, (status) => {
          signal?.removeEventListener('abort', abort);
          resolve(status);
        });
      });
    const state = followed<BackgroundItemStatus>({
      // Followed from its first subscriber: how it stands is asked once, then told as it changes.
      start: () => void ask('status').catch(() => {}),
    });
    items.set(kind, state.tell);
    return {
      available,
      current: state.current,
      subscribe: state.subscribe,
      status: (options) => ask('status', options),
      register: async (options) => {
        const status = await ask('register', options);
        if (status === 'enabled') return status;
        if (status === 'requiresApproval') throw new PlatformError('application', 'notPermitted', 'requiresApproval');
        if (status === 'notPermitted') throw new PlatformError('application', 'notPermitted');
        throw new PlatformError('application', 'failed', 'notRegistered');
      },
      unregister: (options) => ask('unregister', options),
    };
  }

  if (quitRequests) {
    events.addListener('OxyOSQuitRequested', ({ serial, reason }: { serial: number; reason: string }) => {
      if (typeof serial !== 'number' || !REASONS.has(reason)) return;
      // The handlers as they are now: one registered while they answer is asked next time.
      void answerQuit([...quitHandlers], reason as QuitReason).then((answer) =>
        native?.replyToQuit?.(serial, answer === 'quit'),
      );
    });
  }

  /** Asked before the session logs out, restarts or shuts down (protocol version 38): `'quit'`,
      and the application ends; `'cancel'`, and nothing ends ("Pages canceled restart."). Every
      handler is asked in order; any cancel cancels. With none registered the platform quits it at
      once. Returns what unregisters it. Where the session never asks (`quitRequests` false), it
      is kept and never called. */
  function onQuitRequest(handler: QuitHandler): () => void {
    quitHandlers.push(handler);
    if (quitRequests && quitHandlers.length === 1) native?.handlesQuit?.(true);
    let registered = true;
    return () => {
      if (!registered) return;
      registered = false;
      const at = quitHandlers.indexOf(handler);
      if (at >= 0) quitHandlers.splice(at, 1);
      if (quitRequests && quitHandlers.length === 0) native?.handlesQuit?.(false);
    };
  }

  const permission = (capability: Capability): Permission => {
    const launch = session.launch();
    // A system or development application is not narrowed: it has every capability.
    return !launch || !launch.narrowed || launch.capabilities.includes(capability) ? 'granted' : 'denied';
  };
  return {
    available: Boolean(native),
    /** The id the session gave it (its registry entry's), null where it did not start it. */
    id: constants.id,
    /** The module it registers with AppRegistry. */
    module: constants.module,
    /** What the session started it to open (a file, or a URL of its own scheme), null for none. */
    launchURL: constants.launchURL,
    /** Its own folders: its documents and its cache, null where the session did not start it. */
    directories: { document: constants.dataDirectory, cache: constants.cacheDirectory },
    /** Its embedded app config (expo-constants' manifest), null for none. */
    manifest: constants.manifest as Readonly<Record<string, unknown>> | null,
    /** The computer's name and the system's version. */
    device: { name: constants.deviceName, systemVersion: constants.systemVersion },
    /** The system grants the image's policy gave it (`store`, `settings`...): the platform's own
        frameworks open only with theirs. */
    get grants(): readonly string[] {
      return session.launch()?.grants ?? [];
    },
    /**
     * Opens `url` with the application whose entry declares its scheme (the platform's Settings at
     * a page for `oxyos-settings:`), citing the person's latest press or key. Rejects with
     * `needsInput` without one, `invalid` for a file (opened by its type) or text that names no
     * scheme, `failed` (reason `notOpened`) where the session opened nothing, `unavailable` where
     * this window cannot ask.
     */
    async open(url: string, options?: CallOptions): Promise<void> {
      if (!native) throw new PlatformError('application', 'unavailable');
      const answer = await hostCall('application', () => native.open(String(url)), options);
      const error = errorOf('application', answer);
      if (error) throw error;
    },
    /** Whether it has `capability` now, without asking. */
    permission,
    /** Asks the person for `capability` at first use, in the system's sheet (only after an input
        of theirs in the application); answers from the account's decision once it was made. */
    async requestPermission(capability: Capability, options?: CallOptions): Promise<Permission> {
      if (permission(capability) === 'granted') return 'granted';
      const granted = await hostCall('application', () => session.requestCapability(capability), options);
      return granted ? 'granted' : 'denied';
    },
    /** Whether the session asks this application before it ends (protocol version 38). */
    quitRequests,
    /** Asked before the session logs out, restarts or shuts down: `'quit'`, and the application
        ends; `'cancel'`, and nothing ends. Returns what unregisters it. */
    onQuitRequest,
    /** The application itself, opened at login. */
    loginItem: item('loginItem'),
    /** Its process, running on in the background once its window closes. */
    backgroundActivity: item('background'),
  };
}

export type Application = ReturnType<typeof createApplication>;
