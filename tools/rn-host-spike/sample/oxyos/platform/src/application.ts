// `application` (docs/adr/0058-the-platform-api.mdx, decision 1): this application, as the session
// started it: its id, what it was started to open, opening a URL elsewhere (a Mac's
// `NSWorkspace.open`), and its permissions (`requestPermission`, ADR 0030, decision 8). Every
// application may use it.
import type { ApplicationConstants, Spec } from '../specs/NativeOxyOSApplication.ts';
import { errorOf, PlatformError } from './errors.ts';
import type { CallOptions } from './frameworks.ts';
import { hostCall } from './host.ts';
import type { Session } from './session.ts';

/** The host's module (OxyOSApplication), as its spec says it. */
export type ApplicationModule = Pick<Spec, 'getConstants' | 'open'>;

/** A capability an application asks the person for at first use. */
export type Capability = 'notifications' | 'files' | 'camera' | 'microphone' | 'location';
/** Whether this application has a capability now. */
export type Permission = 'granted' | 'denied';

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
};

export function createApplication(native: ApplicationModule | null | undefined, session: Session) {
  const constants: ApplicationConstants = native?.getConstants() ?? NONE;
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
  };
}

export type Application = ReturnType<typeof createApplication>;
