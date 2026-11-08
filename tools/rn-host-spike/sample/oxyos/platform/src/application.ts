// The `application` framework's login item and background activity (docs/adr/0058-the-platform-api.mdx,
// decision 1; docs/adr/0060-login-items-and-background-activity.mdx), as a Mac's `SMAppService` gives
// an application its own: `loginItem`, the application opened at login (the person's Open at Login
// list, the dock's and Settings'), and `backgroundActivity`, its process running on in the
// background once its window closes. The session decides by the process's launch record and its
// account's decisions; nothing here grants anything. Registering a new item tells the person, who
// manages it in Settings › General › Login Items & Extensions.
import { PlatformError } from './errors.ts';
import { followed, type Followed } from './followed.ts';
import type { CallOptions } from './frameworks.ts';
import type { Events } from './session.ts';

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

/** The host's module (`OxyOSApplication`): an ask about one of the application's items, its serial
    (0: not sent), answered by `OxyOSBackgroundItemState`. */
export interface ApplicationModule {
  getConstants(): { backgroundItems?: boolean };
  backgroundItem(kind: 'loginItem' | 'background', action: 'status' | 'register' | 'unregister'): number;
}

const STATUSES = new Set<string>(['notRegistered', 'enabled', 'requiresApproval', 'notPermitted']);

export function createApplication(native: ApplicationModule | null | undefined, events: Events) {
  const available = Boolean(native?.getConstants().backgroundItems);
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
        if (!available || !native) {
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

  return {
    /** The application itself, opened at login. */
    loginItem: item('loginItem'),
    /** Its process, running on in the background once its window closes. */
    backgroundActivity: item('background'),
  };
}

export type Application = ReturnType<typeof createApplication>;
