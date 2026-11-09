// The keychain (docs/adr/0050-keychain-autofill-and-passkeys.mdx; docs/adr/0058-the-platform-api.mdx,
// decision 1's `keychain`): what an application may ask of the person's Oxy keychain. It never
// lists, searches or reads it, and never learns whether it holds anything (decision 5): AutoFill
// fills its fields through the platform (decision 6), and it may ask to save a login it has just
// used, which the person confirms or not in a sheet only the system draws.
import { errorOf, PlatformError } from './errors.ts';
import type { CallOptions } from './frameworks.ts';
import { createPasskeys, type PasskeyModule } from './passkeys.ts';

/** The host's module (`OxyOSKeychain`), the runtime's own. */
export interface KeychainModule {
  getConstants(): { available: boolean };
  savePassword(username: string, password: string, domain: string | null): Promise<unknown>;
}

/** A login an application has just used. */
export interface Credential {
  username: string;
  password: string;
  /** The website the login is for, as a host name (`example.com`), saved as its URL. */
  domain?: string;
}

/** What the person decided: `saved`, or `notSaved` (Not Now, Escape, no answer within two
    minutes, or the application's window not the key window when asked). */
export type SaveResult = 'saved' | 'notSaved';

/** The host modules the keychain framework is made of, by what each serves: `keychain`, the
    host's `OxyOSKeychain` (saving), and `passkeys`, the host's `OxyOSAccountSession` (its passkey
    ceremonies); either absent where the runtime has none. */
export interface KeychainModules {
  keychain?: KeychainModule | null;
  passkeys?: PasskeyModule | null;
}

/**
 * ADR 0058's one `keychain` framework (credentials, AutoFill, passkeys), composed of its host
 * modules: `{ available, savePassword(credential, { signal }), passkeys: { available,
 * create(options, { signal }), get(options, { signal }) } }`.
 */
export function createKeychain(modules: KeychainModules = {}) {
  const module = modules.keychain ?? null;
  const available = module?.getConstants().available ?? false;

  /**
   * Asks to save `credential` in the person's Oxy keychain, associated with this application, as
   * a Mac's "Save Password?" after a sign-in: the system asks the person in a sheet over the
   * application's window, naming the application and the username (never showing the password),
   * and saves it only on their yes. A login of this application with the same username is
   * updated instead. Call it once the sign-in it was used for succeeded.
   *
   * Rejects with a PlatformError of domain `keychain`: `locked` (the session is locked),
   * `unavailable` (no keychain on this computer: `reason` `notMember` until the person turns it
   * on in Settings; no session to ask in; not an application the session launched), `busy` (this
   * application already waits for an answer), `invalid` (an empty or overlong username or
   * password, a username with control characters, a domain that is not a host name),
   * `notPermitted`, `cancelled` (its signal aborted: the sheet may still be answered, and the
   * answer is dropped), or `failed` (`reason` `offline`, `conflict`, …: the person said yes and it
   * could not be written).
   *
   * @since 1 (OxyOS's `api` that ships it)
   * @example
   * const result = await keychain.savePassword({ username, password, domain: 'example.com' });
   * // 'saved' or 'notSaved'
   */
  async function savePassword(credential: Credential, options: CallOptions = {}): Promise<SaveResult> {
    const { signal } = options;
    if (signal?.aborted) throw new PlatformError('keychain', 'cancelled');
    if (!module || !available) throw new PlatformError('keychain', 'unavailable');
    const asked = module.savePassword(credential.username, credential.password, credential.domain ?? null);
    // Its own abort until the shared `hostCall` (platform/src/host.ts) is on main.
    const answer = await new Promise<unknown>((resolve, reject) => {
      const abort = () => reject(new PlatformError('keychain', 'cancelled'));
      signal?.addEventListener('abort', abort, { once: true });
      asked.then(
        (value) => {
          signal?.removeEventListener('abort', abort);
          resolve(value);
        },
        (error) => {
          signal?.removeEventListener('abort', abort);
          reject(error);
        },
      );
    });
    const error = errorOf('keychain', answer);
    if (error) throw error;
    const told = answer as { saved?: unknown; notSaved?: unknown };
    if (told.saved !== undefined) return 'saved';
    if (told.notSaved !== undefined) return 'notSaved';
    throw new PlatformError('keychain', 'failed', 'noAnswer');
  }

  return { available, savePassword, passkeys: createPasskeys(modules.passkeys ?? null) };
}

export type Keychain = ReturnType<typeof createKeychain>;
