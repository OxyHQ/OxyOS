// Passkeys, the `keychain` framework's `passkeys` (docs/adr/0050-keychain-autofill-and-passkeys.mdx,
// decision 7; docs/adr/0058-the-platform-api.mdx, decision 1's `keychain`): OxyOS as a platform authenticator, in WebAuthn's own JSON shapes, as
// `navigator.credentials.create()` and `.get()` take and answer them with `PublicKeyCredential`'s
// `parseCreationOptionsFromJSON()` and `toJSON()` (WebAuthn Level 3, §5.1.8–§5.1.10; binary as
// base64url without padding). An application uses only relying parties associated with it (its
// Oxy registry's web origins, or passkeys of its own); the person verifies every ceremony in a
// sheet only the system draws, with their Oxy account's password, and the application never sees
// it. The client data is the platform's to make (origin `https://` and the relying party's id).
import { PlatformError, type PlatformErrorCode } from './errors.ts';
import type { CallOptions } from './frameworks.ts';

/** The host's module (`OxyOSAccountSession`), the runtime's own. */
export interface PasskeyModule {
  getConstants(): { available: boolean };
  passkey(ceremony: 'create' | 'get', options: string): Promise<unknown>;
}

type Base64URL = string;
type Descriptor = { type: 'public-key'; id: Base64URL; transports?: string[] };

/** `PublicKeyCredentialCreationOptionsJSON`; `rp.id` is required (an application has no origin). */
export interface CreationOptionsJSON {
  rp: { id: string; name: string };
  user: { id: Base64URL; name: string; displayName: string };
  challenge: Base64URL;
  pubKeyCredParams: { type: 'public-key'; alg: number }[];
  timeout?: number;
  excludeCredentials?: Descriptor[];
  authenticatorSelection?: {
    authenticatorAttachment?: 'platform' | 'cross-platform';
    residentKey?: string;
    requireResidentKey?: boolean;
    userVerification?: string;
  };
  hints?: string[];
  attestation?: string;
  extensions?: Record<string, unknown>;
}

/** `PublicKeyCredentialRequestOptionsJSON`; `rpId` is required. */
export interface RequestOptionsJSON {
  challenge: Base64URL;
  rpId: string;
  timeout?: number;
  allowCredentials?: Descriptor[];
  userVerification?: string;
  hints?: string[];
  extensions?: Record<string, unknown>;
}

/** `RegistrationResponseJSON`. */
export interface RegistrationResponseJSON {
  id: Base64URL;
  rawId: Base64URL;
  type: 'public-key';
  authenticatorAttachment: 'platform';
  response: {
    clientDataJSON: Base64URL;
    attestationObject: Base64URL;
    authenticatorData: Base64URL;
    publicKey: Base64URL;
    publicKeyAlgorithm: number;
    transports: string[];
  };
  clientExtensionResults: Record<string, never>;
}

/** `AuthenticationResponseJSON`. */
export interface AuthenticationResponseJSON {
  id: Base64URL;
  rawId: Base64URL;
  type: 'public-key';
  authenticatorAttachment: 'platform';
  response: { clientDataJSON: Base64URL; authenticatorData: Base64URL; signature: Base64URL; userHandle: Base64URL };
  clientExtensionResults: Record<string, never>;
}

/** The platform's code for WebAuthn's error, its DOMException name kept as the reason. */
const CODES: Record<string, PlatformErrorCode> = {
  NotAllowedError: 'notPermitted',
  InvalidStateError: 'exists',
  NotSupportedError: 'unavailable',
  TypeError: 'invalid',
  UnknownError: 'failed',
};

type Answer = { credential?: unknown; error?: { name?: unknown; reason?: unknown }; failed?: { reason?: unknown } };

/** A ceremony's answer as the credential, or the PlatformError it is. */
export function credentialOf(answer: unknown): unknown {
  const told = (answer ?? {}) as Answer;
  if (told.credential && typeof told.credential === 'object') return told.credential;
  if (told.error && typeof told.error.name === 'string') {
    const name = told.error.name;
    // A locked session is the platform's own code; the reason stays WebAuthn's name.
    const code = told.error.reason === 'locked' ? 'locked' : (CODES[name] ?? 'failed');
    throw new PlatformError('keychain', code, name);
  }
  if (told.failed) throw new PlatformError('keychain', 'unavailable', 'notSent');
  throw new PlatformError('keychain', 'failed', 'noAnswer');
}

export function createPasskeys(module: PasskeyModule | null) {
  const available = module?.getConstants().available ?? false;

  // Its own abort until the shared `hostCall` (platform/src/host.ts) is on main.
  async function ceremony(kind: 'create' | 'get', options: object, call: CallOptions): Promise<unknown> {
    const { signal } = call;
    if (signal?.aborted) throw new PlatformError('keychain', 'cancelled');
    if (!module || !available) throw new PlatformError('keychain', 'unavailable');
    const asked = module.passkey(kind, JSON.stringify(options));
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
    return credentialOf(answer);
  }

  /**
   * Creates a passkey for `options.rp.id` in the person's Oxy keychain (synced to their devices,
   * end-to-end encrypted), as `navigator.credentials.create({publicKey})`: the system asks the
   * person in a sheet over the application's window ("Create a passkey for example.com?") with
   * their Oxy account's password. Answers the credential as the relying party's server reads it.
   *
   * Rejects with a PlatformError of domain `keychain`, its `reason` WebAuthn's DOMException name:
   * `notPermitted` (`NotAllowedError`: the person cancelled or did not verify, the time ran out,
   * a relying party not associated with this application, or no keychain on this computer, all
   * alike), `locked` (the session is locked), `exists` (`InvalidStateError`: one of
   * `excludeCredentials` is the person's), `unavailable` (`NotSupportedError`: no algorithm
   * OxyOS makes, ES256; or not an application the session launched), `invalid` (`TypeError`: not
   * WebAuthn's shape), `failed` (`UnknownError`: it could not be saved), or `cancelled` (its
   * signal aborted; the sheet stays until the person answers it, and the answer is dropped).
   *
   * @since 1 (OxyOS's `api` that ships it)
   * @example
   * const credential = await keychain.passkeys.create(await (await fetch('/webauthn/register')).json());
   */
  const create = (options: CreationOptionsJSON, call: CallOptions = {}) =>
    ceremony('create', options, call) as Promise<RegistrationResponseJSON>;

  /**
   * Signs in with a passkey for `options.rpId`, as `navigator.credentials.get({publicKey})`: the
   * system asks the person in a sheet ("Use your passkey for example.com?"), with a chooser when
   * several match, and their Oxy account's password. Rejects as `create` does (`exists`
   * aside); no passkey there is `notPermitted` too.
   *
   * @since 1
   */
  const get = (options: RequestOptionsJSON, call: CallOptions = {}) =>
    ceremony('get', options, call) as Promise<AuthenticationResponseJSON>;

  return { available, create, get };
}

export type Passkeys = ReturnType<typeof createPasskeys>;
