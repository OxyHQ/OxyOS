// `account` (docs/adr/0058-the-platform-api.mdx, decision 1; docs/adr/0020-oxy-account-login.mdx,
// decision 6): the machine's Oxy session, for every application the session launched: who is signed
// in (followed: the host keeps the last the accounts' service told, decision 8), a short token for
// this application alone, minted by the service, and locking the session. The service knows the
// application by its launch: nothing here claims anything. The shell and the login screen have none.
import { PlatformError } from './errors.ts';
import { followed } from './followed.ts';
import type { Followed } from './followed.ts';
import type { CallOptions } from './frameworks.ts';
import { hostCall } from './host.ts';
import type { Events } from './session.ts';

/** The host's module (OxyOSAccountSession), what an application reaches of it. `snapshot` is the
    host's last told, null before the first; `subscribe` follows it (`OxyOSAccountSessionChanged`,
    null once the service went away). */
export interface AccountModule {
  getConstants(): { available?: boolean };
  snapshot(): unknown;
  subscribe(): boolean;
  getAccessToken(reason: string): Promise<unknown>;
  lockSession(): boolean;
}

/** The signed-in account, as the service tells it. */
export interface Account {
  readonly id: string;
  readonly handle: string;
  readonly displayName?: string;
  readonly avatar?: string;
}
/** The session: signed in (with its account), signed out, or the service unavailable. */
export interface AccountState {
  readonly status: 'signed_in' | 'signed_out' | 'unavailable';
  readonly account: Account | null;
  /** Whether Oxy is reachable now (a token can be minted). */
  readonly online: boolean;
}
/** Why a token is asked: at start, before a request, or after the server answered 401 (minted
    afresh). */
export type TokenReason = 'boot' | 'preflight' | 'response-401';
export interface AccessToken {
  accessToken: string;
  expiresAt: string;
}

const UNAVAILABLE: AccountState = Object.freeze({ status: 'unavailable', account: null, online: false });
const REASONS = new Set<string>(['boot', 'preflight', 'response-401']);

/** The service's snapshot read strictly; anything that is not one is the service unavailable. */
export function accountStateOf(told: unknown): AccountState {
  if (typeof told !== 'object' || told === null) return UNAVAILABLE;
  const { status, account, online } = told as Record<string, unknown>;
  if (status !== 'signed_in' && status !== 'signed_out' && status !== 'unavailable') return UNAVAILABLE;
  if (typeof online !== 'boolean') return UNAVAILABLE;
  let kept: Account | null = null;
  if (account != null) {
    const given = account as Record<string, unknown>;
    if (typeof account !== 'object' || typeof given.id !== 'string' || typeof given.handle !== 'string') {
      return UNAVAILABLE;
    }
    kept = Object.freeze({
      id: given.id,
      handle: given.handle,
      ...(typeof given.displayName === 'string' ? { displayName: given.displayName } : {}),
      ...(typeof given.avatar === 'string' ? { avatar: given.avatar } : {}),
    });
  }
  if ((status === 'signed_in') !== (kept !== null)) return UNAVAILABLE;
  return Object.freeze({ status, account: kept, online });
}

const same = (a: AccountState, b: AccountState) =>
  a.status === b.status && a.online === b.online && JSON.stringify(a.account) === JSON.stringify(b.account);

export function createAccount(native: AccountModule | null | undefined, events: Events) {
  const available = Boolean(native?.getConstants()?.available);
  // Followed on one connection the host keeps; once it ended (the service went away), the session
  // is unavailable until it is followed again (a token asked, or a new follower).
  let following = false;
  const follow = () => {
    following = Boolean(native?.subscribe());
    if (!following) told(UNAVAILABLE);
  };
  const state = followed<AccountState>({
    start: () => {
      if (available && !following) follow();
    },
  });
  const told = (next: AccountState) => {
    const last = state.current();
    if (!last || !same(last, next)) state.tell(next);
  };
  if (available && native) {
    events.addListener('OxyOSAccountSessionChanged', (snapshot: unknown) => {
      if (snapshot === null) following = false;
      told(accountStateOf(snapshot));
    });
  }
  const current: Followed<AccountState> = {
    // The host's last told, read without asking the service; null where there is no session.
    current() {
      if (!available || !native) return null;
      // Unfollowed, the host's own is read again (it follows the service from its start).
      if (!following || state.current() === null) told(accountStateOf(native.snapshot()));
      return state.current();
    },
    subscribe: state.subscribe,
  };
  const unavailable = () => new PlatformError('account', 'unavailable');
  return {
    available,
    /** Who is signed in, followed. */
    state: current,
    /** A short token for this application alone, or null when there is none (offline, signed out).
        Rejects with `invalid` for a reason that is not one, `unavailable` without a session. */
    async accessToken(reason: TokenReason, options?: CallOptions): Promise<AccessToken | null> {
      if (!available || !native) throw unavailable();
      if (!REASONS.has(reason)) throw new PlatformError('account', 'invalid', String(reason));
      if (!following) follow();
      const token = await hostCall('account', () => native.getAccessToken(reason), options);
      if (typeof token !== 'object' || token === null) return null;
      const { accessToken, expiresAt } = token as Record<string, unknown>;
      if (typeof accessToken !== 'string' || !accessToken || typeof expiresAt !== 'string' || !expiresAt) return null;
      return { accessToken, expiresAt };
    },
    /** Locks the session (the person signs in again on the lock screen, with Oxy online): only
        while this application's window is the active one; `notPermitted` otherwise. */
    async lock(): Promise<void> {
      if (!available || !native) throw unavailable();
      if (!native.lockSession()) throw new PlatformError('account', 'notPermitted', 'notActive');
    },
  };
}

export type AccountFramework = ReturnType<typeof createAccount>;
