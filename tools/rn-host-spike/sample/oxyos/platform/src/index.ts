// `@oxyos/platform` (docs/adr/0058-the-platform-api.mdx): OxyOS's platform API, in the runtime's
// framework. Its frameworks, each one domain, over the host's modules, which are the runtime's
// private business; only these exports are a contract.
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

import { createFrameworks } from './frameworks.ts';
import { createSession, type SystemModule } from './session.ts';

export { isPlatformError, PLATFORM_ERROR_CODES, PlatformError } from './errors.ts';
export type { PlatformDomain, PlatformErrorCode } from './errors.ts';
export type { Followed } from './followed.ts';
export type { CallOptions, Direction, ProgressOptions } from './frameworks.ts';

const runtime = TurboModuleRegistry.get('OxyOSRuntime') as { getConstants(): { api: number } } | null;
/** The API level this runtime gives (its `api`, ADR 0030, decision 2); 0 where it says none. */
export const api: number = runtime?.getConstants().api ?? 0;

/** The one connection to the session; the runtime's own (ADR 0058, decision 2), never a contract. */
export const session = createSession(TurboModuleRegistry.get('OxyOSSystem') as SystemModule | null, DeviceEventEmitter);

const frameworks = createFrameworks(session);
export const { network, bluetooth, power, audio, settings, store } = frameworks;
