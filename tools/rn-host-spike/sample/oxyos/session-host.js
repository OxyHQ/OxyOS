// The machine's Oxy session, given to the Oxy SDK (docs/adr/0020-oxy-account-login.mdx, decision 6;
// oxy ADR 0058): the runtime's framework registers OxyOS as the SDK's session host in every
// application the session launched, before the application's own bundle runs, so the SDK in it
// (`OxyProvider`'s host mode) signs in to nothing: it shows the platform's account and asks the
// platform for short tokens for this application alone. Never in the shell, the login screen (which
// runs the SDK's own sign-in to set an account up) or a process the session did not start: the host
// says where it is available (`OxyOSAccountSession.available`), and the accounts' service knows the
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
import { DeviceEventEmitter, Linking, TurboModuleRegistry } from 'react-native';

/** The contract this host speaks (`OXY_SESSION_HOST_CONTRACT`). */
export const OXY_SESSION_HOST_CONTRACT = 1;
const KEY = Symbol.for('oxy.so/session-host');
/** Settings' Oxy account page, where the account is managed, signed out and removed. */
export const ACCOUNT_SETTINGS_URL = 'oxyos-settings://oxy-account';
const REASONS = new Set(['boot', 'preflight', 'response-401']);
const ACTIONS = new Set(['sign_in', 'switch_account', 'sign_out', 'manage_account']);

/** The account's languages (locales, primary first), kept only as a list of strings. */
const isLanguages = (languages) => Array.isArray(languages)
  && languages.every((language) => typeof language === 'string');

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

/** A snapshot of the service's, kept only if it is one; anything else is none. */
function snapshotOf(told) {
  if (typeof told !== 'object' || told === null) return null;
  const { status, account, online } = told;
  if (status !== 'signed_in' && status !== 'signed_out' && status !== 'unavailable') return null;
  if (typeof online !== 'boolean') return null;
  let kept = null;
  if (account != null) {
    if (typeof account !== 'object' || typeof account.id !== 'string' || typeof account.handle !== 'string') return null;
    kept = Object.freeze({
      id: account.id,
      handle: account.handle,
      ...(typeof account.displayName === 'string' ? { displayName: account.displayName } : {}),
      ...(typeof account.avatar === 'string' ? { avatar: account.avatar } : {}),
      ...(isLanguages(account.languages) ? { languages: Object.freeze([...account.languages]) } : {}),
    });
  }
  if ((status === 'signed_in') !== (kept !== null)) return null;
  return { status, account: kept, online };
}

/**
 * The host over the Bloom host's `OxyOSAccountSession`, or null where it is not available. Its
 * `revision` is its own: the SDK takes a new revision, a new status or another account's id for a
 * switch of account, so it rises only when the status or the account's id changes, whatever the
 * service numbered (a service started again numbers from the start). A change of the profile or of
 * the network is a new snapshot of the same revision.
 */
export function createSessionHost(native = TurboModuleRegistry.get('OxyOSAccountSession'), events = DeviceEventEmitter,
  linking = Linking, locale = TurboModuleRegistry.get('OxyOSLocale')) {
  if (!native || !native.getConstants().available) return null;
  let revision = 0;
  let told = null;
  let current = null;
  const listeners = new Set();
  // The service's last snapshot and the system's language now, as one snapshot; false when nothing
  // changed. A new language is a new snapshot of the same revision.
  const update = () => {
    const next = told ?? { status: 'unavailable', account: null, online: false };
    const language = systemLanguage(locale);
    if (current && current.status === next.status && current.online === next.online
      && JSON.stringify(current.account) === JSON.stringify(next.account)
      && current.preferredLanguages?.[0] === (language ?? undefined)) {
      return false;
    }
    if (!current || current.status !== next.status || current.account?.id !== next.account?.id) revision += 1;
    current = Object.freeze({
      ...next,
      revision,
      ...(language ? { preferredLanguages: Object.freeze([language]) } : {}),
    });
    return true;
  };
  const take = (snapshot) => {
    told = snapshotOf(snapshot);
    return update();
  };
  take(native.snapshot());
  const tell = () => {
    for (const listener of [...listeners]) listener();
  };
  // Language & Region changed: the window has already made the new locale `Intl`'s default.
  events.addListener('OxyOSLocaleChanged', () => {
    if (update()) tell();
  });
  // Followed on one connection the service keeps. The host follows it again by itself when it ends
  // (the service restarted under the application), and says null only if it stays unfollowed for
  // more than a few seconds: the session is unavailable then, until a snapshot comes again.
  let following = false;
  const follow = () => {
    following = native.subscribe();
    if (!following && take(null)) tell();
  };
  events.addListener('OxyOSAccountSessionChanged', (told) => {
    following = told !== null;
    if (take(told)) tell();
  });
  follow();
  const host = {
    contract: OXY_SESSION_HOST_CONTRACT,
    platform: 'oxyos',
    snapshot: () => current,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    async getAccessToken(request) {
      const reason = request?.reason;
      if (!REASONS.has(reason)) return null;
      if (!following) follow();
      const token = await native.getAccessToken(reason);
      if (typeof token !== 'object' || token === null) return null;
      const { accessToken, expiresAt } = token;
      if (typeof accessToken !== 'string' || !accessToken || typeof expiresAt !== 'string' || !expiresAt) return null;
      return { accessToken, expiresAt };
    },
    async request(action) {
      if (!ACTIONS.has(action)) throw new Error(`OxyOS's session host: no action ${String(action)}`);
      // Signing in again: the session locks, and unlocking checks the password with Oxy online.
      if (action === 'sign_in') {
        if (!native.lockSession()) throw new Error('OxyOS could not lock the session');
        return;
      }
      // The account is managed, signed out of and switched in Settings' Oxy account page: signing
      // out ends the session, and switching is signing out to the login screen (decision 5).
      await linking.openURL(ACCOUNT_SETTINGS_URL);
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
