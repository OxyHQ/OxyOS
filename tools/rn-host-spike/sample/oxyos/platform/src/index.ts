// `@oxyos/platform` (docs/adr/0058-the-platform-api.mdx): OxyOS's platform API, in the runtime's
// framework. Its frameworks, each one domain, over the host's modules, which are the runtime's
// private business; only these exports are a contract.
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

import { createApplication, type ApplicationModule } from './application.ts';
import { createDrag, type DragModule } from './drag.ts';
import { createFrameworks } from './frameworks.ts';
import { createKeychain, type KeychainModule } from './keychain.ts';
import type { PasskeyModule } from './passkeys.ts';
import { createPasteboard, type ClipboardModule } from './pasteboard.ts';
import { createSession, type SystemModule } from './session.ts';
import { createSounds, type SoundsModule } from './sounds.ts';

export { isPlatformError, PLATFORM_ERROR_CODES, PlatformError } from './errors.ts';
export type { PlatformDomain, PlatformErrorCode } from './errors.ts';
export type { Followed } from './followed.ts';
export type { ActivityView, CallOptions, Direction, ProgressOptions } from './frameworks.ts';
export type {
  AuthenticationResponseJSON,
  CreationOptionsJSON,
  RegistrationResponseJSON,
  RequestOptionsJSON,
} from './passkeys.ts';
export type { BackgroundItem, BackgroundItemStatus, QuitAnswer, QuitHandler, QuitReason } from './application.ts';
export type { Credential, KeychainModules, SaveResult } from './keychain.ts';
export type { DragAction, DragData, DragEndReason, DragOutcome, DragRequest, WindowDragEvent } from './drag.ts';
export { SYSTEM_SOUNDS, type SystemSound } from './sounds.ts';

const runtime = TurboModuleRegistry.get('OxyOSRuntime') as { getConstants(): { api: number } } | null;
/** The API level this runtime gives (its `api`, ADR 0030, decision 2); 0 where it says none. */
export const api: number = runtime?.getConstants().api ?? 0;

/** The one connection to the session; the runtime's own (ADR 0058, decision 2), never a contract. */
export const session = createSession(TurboModuleRegistry.get('OxyOSSystem') as SystemModule | null, DeviceEventEmitter);

const frameworks = createFrameworks(session);
export const { network, bluetooth, power, audio, settings, store, activity } = frameworks;

/** This application (docs/adr/0058-the-platform-api.mdx, decision 1): its login item and background
    activity (docs/adr/0060-login-items-and-background-activity.mdx), and its answer when the session
    asks it to quit before logging out, restarting or shutting down
    (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx). */
export const application = createApplication(
  TurboModuleRegistry.get('OxyOSApplication') as ApplicationModule | null,
  DeviceEventEmitter,
);

/** The keychain (docs/adr/0050-keychain-autofill-and-passkeys.mdx): asking to save a login, and
    passkeys, the platform's authenticator. */
export const keychain = createKeychain({
  keychain: TurboModuleRegistry.get('OxyOSKeychain') as KeychainModule | null,
  passkeys: TurboModuleRegistry.get('OxyOSAccountSession') as PasskeyModule | null,
});

/** Copying and pasting text (docs/adr/0007-clipboard.mdx). */
export const pasteboard = createPasteboard(
  TurboModuleRegistry.get('OxyOSClipboard') as ClipboardModule | null,
  DeviceEventEmitter,
);

/** The system's interface sounds (docs/adr/0044-system-sounds.mdx). */
export const sounds = createSounds(TurboModuleRegistry.get('OxyOSSounds') as SoundsModule | null);

/** Dragging content to other applications, and drags over the window (docs/adr/0035-drag-and-drop.mdx). */
export const drag = createDrag(TurboModuleRegistry.get('OxyOSDrag') as DragModule | null, DeviceEventEmitter);
