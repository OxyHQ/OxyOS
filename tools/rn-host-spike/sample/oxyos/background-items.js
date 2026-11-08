/**
 * Login items and background activity (docs/adr/0060-login-items-and-background-activity.mdx),
 * as a Mac's `SMAppService` gives an application its own: `loginItem`, the application opened at
 * login (the person's Open at Login list, the dock's and Settings'), and `backgroundActivity`, its
 * process running on in the background once its window closes. Each is the platform API's shape
 * (docs/adr/0058-the-platform-api.mdx, decision 3), over the Bloom host's `OxyOSApplication`, and
 * becomes `@oxyos/platform/application`'s when that package lands (its step 6):
 *
 * - `available`: the session can be asked (protocol version 32);
 * - `status()`: how it stands, `'enabled'`, `'notRegistered'`, `'requiresApproval'` (the person
 *   turned it off) or `'notPermitted'` (the platform gives this application none);
 * - `register()` and `unregister()`: resolve with how it then stands, or reject with a
 *   `PlatformError` (`domain` `'application'`, `code` `notPermitted`, its `reason`
 *   `'requiresApproval'` when the person turned it off; `failed` when it was not registered;
 *   `unavailable`);
 * - followed state: `current()` (null before the session said) and `subscribe(listener)`, told as
 *   the session answers and as the person changes it (removing it from Open at Login, turning its
 *   background activity off or on). Following asks once, with the first subscriber.
 *
 * The session decides by the process's launch record and its account's decisions; nothing here
 * grants anything. Registering a new item tells the person, who manages it in Settings › General ›
 * Login Items & Extensions.
 */
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

const native = TurboModuleRegistry.get('OxyOSApplication');
const available = Boolean(native?.getConstants?.().backgroundItems);

/** An error of the platform API's one kind (docs/adr/0058-the-platform-api.mdx, decision 3). */
function platformError(code, message, reason) {
  return Object.assign(new Error(message), { name: 'PlatformError', domain: 'application', code, reason });
}

/** Asks waiting for their answer, by serial. */
const waiting = new Map();
const items = {};

function item(kind) {
  let last = null;
  const listeners = new Set();
  const tell = (status) => {
    last = status;
    for (const listener of listeners) listener(status);
  };
  const ask = (action) => {
    if (!available) return Promise.reject(platformError('unavailable', 'This session has no login items or background activity'));
    const serial = native.backgroundItem(kind, action);
    if (!serial) return Promise.reject(platformError('unavailable', 'The session could not be asked'));
    return new Promise((resolve) => waiting.set(serial, resolve));
  };
  const registered = (status) => {
    if (status === 'enabled') return status;
    if (status === 'requiresApproval') {
      throw platformError('notPermitted', 'The person turned it off in Login Items & Extensions', 'requiresApproval');
    }
    if (status === 'notPermitted') throw platformError('notPermitted', 'The platform gives this application none');
    throw platformError('failed', 'It was not registered', status);
  };
  const followed = {
    available,
    status: () => ask('status'),
    register: () => ask('register').then(registered),
    unregister: () => ask('unregister'),
    current: () => last,
    subscribe(listener) {
      listeners.add(listener);
      if (listeners.size === 1 && last === null && available) followed.status().catch(() => {});
      return () => listeners.delete(listener);
    },
  };
  items[kind] = tell;
  return followed;
}

if (available) {
  DeviceEventEmitter.addListener('OxyOSBackgroundItemState', ({ serial, kind, status }) => {
    items[kind]?.(status);
    const resolve = waiting.get(serial);
    if (!resolve) return;
    waiting.delete(serial);
    resolve(status);
  });
}

/** The application itself, opened at login. */
export const loginItem = item('loginItem');
/** Its process, running on in the background once its window closes. */
export const backgroundActivity = item('background');
