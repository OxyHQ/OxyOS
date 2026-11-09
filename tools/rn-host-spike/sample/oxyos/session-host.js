// The machine's Oxy session, given to the Oxy SDK (docs/adr/0020-oxy-account-login.mdx, decision 6;
// oxy ADR 0058): the runtime's framework registers OxyOS as the SDK's session host in every
// application the session launched, before the application's own bundle runs, so the SDK in it
// (`OxyProvider`'s host mode) signs in to nothing: it shows the platform's account and asks the
// platform for short tokens for this application alone. Never in the shell, the login screen (which
// runs the SDK's own sign-in to set an account up) or a process the session did not start: the host
// says where it is available (the platform's `account`), and the accounts' service knows the
// asking process by its launch, never by what it says.
//
// The interface is `@oxy.so/core/host`'s (B1, contract 1), mirrored here: the runtime and the
// application each carry their own copy of the SDK, so the registration is the one global both read,
// `globalThis[Symbol.for('oxy.so/session-host')]`, made once and never replaced.
//
// Every snapshot names the system's language as `preferredLanguages` (oxy ADR 0058 R11): the
// session's locale, the one `Intl` defaults to (docs/adr/0040-language-and-region.mdx, decision 4),
// so an Oxy application speaks the system's language from its first frame, as a Mac's applications
// speak the system's preferred languages, and never waits for the account's.
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';
import { account as platformAccount, application as platformApplication } from '@oxyos/platform';

/** The contract this host speaks (`OXY_SESSION_HOST_CONTRACT`). */
export const OXY_SESSION_HOST_CONTRACT = 1;
const KEY = Symbol.for('oxy.so/session-host');
/** Settings' Oxy account page, where the account is managed, signed out and removed. */
export const ACCOUNT_SETTINGS_URL = 'oxyos-settings://oxy-account';
const ACTIONS = new Set(['sign_in', 'switch_account', 'sign_out', 'manage_account']);

/**
 * The system's language as a language tag (`es-MX`), its Unicode extensions (`-u-hc-h12`, the hour
 * cycle the session keeps with it) left out, or null where the host does not say one.
 */
function systemLanguage(locale) {
  const tag = locale?.getLocale?.();
  if (typeof tag !== 'string') return null;
  const language = tag.split(/-[ux]-/i)[0];
  return language ? language : null;
}

/**
 * The host over the platform's `account` (`@oxyos/platform`), or null where there is no session.
 * Its `revision` is its own: the SDK takes a new revision, a new status or another account's id for
 * a switch of account, so it rises only when the status or the account's id changes. A change of
 * the profile or of the network is a new snapshot of the same revision.
 */
export function createSessionHost(account = platformAccount, application = platformApplication,
  events = DeviceEventEmitter, locale = TurboModuleRegistry.get('OxyOSLocale')) {
  if (!account.available) return null;
  let revision = 0;
  let current = null;
  const listeners = new Set();
  // The account's state and the system's language now, as one snapshot: the same one when nothing
  // changed. A new language is a new snapshot of the same revision.
  const take = (state) => {
    const language = systemLanguage(locale);
    if (current && current.status === state.status && current.online === state.online
      && JSON.stringify(current.account) === JSON.stringify(state.account)
      && current.preferredLanguages?.[0] === (language ?? undefined)) {
      return current;
    }
    if (!current || current.status !== state.status || current.account?.id !== state.account?.id) revision += 1;
    current = Object.freeze({
      ...state,
      revision,
      ...(language ? { preferredLanguages: Object.freeze([language]) } : {}),
    });
    return current;
  };
  const now = () => take(account.state.current() ?? { status: 'unavailable', account: null, online: false });
  // Language & Region changed: the window has already made the new locale `Intl`'s default.
  events.addListener('OxyOSLocaleChanged', () => {
    const before = current;
    if (now() !== before) for (const listener of [...listeners]) listener();
  });
  const host = {
    contract: OXY_SESSION_HOST_CONTRACT,
    platform: 'oxyos',
    snapshot: now,
    subscribe(listener) {
      listeners.add(listener);
      const stop = account.state.subscribe(() => listener());
      return () => {
        listeners.delete(listener);
        stop();
      };
    },
    async getAccessToken(request) {
      try {
        return await account.accessToken(request?.reason);
      } catch {
        // A reason that is not one, or no session: no token, as the contract answers.
        return null;
      }
    },
    async request(action) {
      if (!ACTIONS.has(action)) throw new Error(`OxyOS's session host: no action ${String(action)}`);
      // Signing in again: the session locks, and unlocking checks the password with Oxy online.
      if (action === 'sign_in') {
        try {
          await account.lock();
        } catch {
          throw new Error('OxyOS could not lock the session');
        }
        return;
      }
      // The account is managed, signed out of and switched in Settings' Oxy account page: signing
      // out ends the session, and switching is signing out to the login screen (decision 5).
      await application.open(ACCOUNT_SETTINGS_URL);
    },
  };
  return Object.freeze(host);
}

/** Registers the host where the session launched an application; once, never replaced. */
export function registerSessionHost(global = globalThis, create = createSessionHost) {
  if (Object.prototype.hasOwnProperty.call(global, KEY)) return global[KEY];
  const host = create();
  if (!host) return null;
  Object.defineProperty(global, KEY, { value: host, writable: false, configurable: false, enumerable: false });
  return host;
}
