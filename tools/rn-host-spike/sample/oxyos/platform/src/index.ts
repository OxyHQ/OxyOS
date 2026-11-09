// `@oxyos/platform` (docs/adr/0058-the-platform-api.mdx): OxyOS's platform API, in the runtime's
// framework. Its frameworks, each one domain, over the host's modules, which are the runtime's
// private business; only these exports are a contract.
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

import NativeOxyOSApplication from '../specs/NativeOxyOSApplication.ts';
import NativeOxyOSClipboard from '../specs/NativeOxyOSClipboard.ts';
import NativeOxyOSNotifications from '../specs/NativeOxyOSNotifications.ts';
import NativeOxyOSSounds from '../specs/NativeOxyOSSounds.ts';
import { createAccount, type AccountModule } from './account.ts';
import { createApplication } from './application.ts';
import { createChooser, type FileChooserModule } from './chooser.ts';
import { createDrag, type DragModule } from './drag.ts';
import { createFiles, createFileService, type FilesModule } from './files.ts';
import { createFrameworks } from './frameworks.ts';
import { createKeychain, type KeychainModule } from './keychain.ts';
import type { PasskeyModule } from './passkeys.ts';
import { createNotifications } from './notifications.ts';
import { createPasteboard } from './pasteboard.ts';
import { createSearch, type SearchModule } from './search.ts';
import { createSession, type SystemModule } from './session.ts';
import { createSounds } from './sounds.ts';

export type { AccessToken, Account, AccountState, TokenReason } from './account.ts';
export type { Capability, Permission } from './application.ts';
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
export type { ChooserRequest, OpenPanelOptions, SavePanelOptions } from './chooser.ts';
export type {
  BackupRef,
  FileEvent,
  FileInfo,
  FileTag,
  FolderItem,
  ItemOutcome,
  Places,
  TaggedItem,
  TrashedItem,
} from './files.ts';
export type { DragAction, DragData, DragEndReason, DragOutcome, DragRequest, WindowDragEvent } from './drag.ts';
export {
  MAX_NOTIFICATION_ACTIONS,
  MAX_NOTIFICATION_DATA,
  type Notification,
  type NotificationAction,
  type NotificationResponse,
} from './notifications.ts';
export { SYSTEM_SOUNDS, type SystemSound } from './sounds.ts';

const runtime = TurboModuleRegistry.get('OxyOSRuntime') as { getConstants(): { api: number } } | null;
/** The API level this runtime gives (its `api`, ADR 0030, decision 2); 0 where it says none. */
export const api: number = runtime?.getConstants().api ?? 0;

/** The one connection to the session; the runtime's own (ADR 0058, decision 2), never a contract. */
export const session = createSession(TurboModuleRegistry.get('OxyOSSystem') as SystemModule | null, DeviceEventEmitter);

const frameworks = createFrameworks(session);
export const { network, bluetooth, power, audio, settings, store, activity } = frameworks;

/** The keychain (docs/adr/0050-keychain-autofill-and-passkeys.mdx): asking to save a login, and
    passkeys, the platform's authenticator. */
export const keychain = createKeychain({
  keychain: TurboModuleRegistry.get('OxyOSKeychain') as KeychainModule | null,
  passkeys: TurboModuleRegistry.get('OxyOSAccountSession') as PasskeyModule | null,
});

/** Copying and pasting text (docs/adr/0007-clipboard.mdx). */
export const pasteboard = createPasteboard(NativeOxyOSClipboard, DeviceEventEmitter);

/** The system's interface sounds (docs/adr/0044-system-sounds.mdx). */
export const sounds = createSounds(NativeOxyOSSounds);

/** Dragging content to other applications, and drags over the window (docs/adr/0035-drag-and-drop.mdx). */
export const drag = createDrag(TurboModuleRegistry.get('OxyOSDrag') as DragModule | null, DeviceEventEmitter);

/** The system's open and save panels (docs/adr/0029-chromium-on-oxyos.mdx, decision 8). */
export const chooser = createChooser(
  TurboModuleRegistry.get('OxyOSFileChooser') as FileChooserModule | null,
  DeviceEventEmitter,
);

/** Posting notifications and their responses (docs/adr/0026-notifications.mdx). */
export const notifications = createNotifications(NativeOxyOSNotifications, DeviceEventEmitter);

/** The connection to the file service; the runtime's own (ADR 0058, decision 2), never a contract:
    the framework below and expo-file-system's module are written over it. */
export const fileService = createFileService(
  TurboModuleRegistry.get('OxyOSFiles') as FilesModule | null,
  DeviceEventEmitter,
  session.requestCapability,
);

/** Items, places, the Trash, watching and tags (docs/adr/0028-file-access.mdx). */
export const files = createFiles(fileService);

/** This application (docs/adr/0058-the-platform-api.mdx, decision 1): what the session started it
    as and to open, opening a URL elsewhere, and its permissions (docs/adr/0030-installing-
    applications.mdx, decision 8); its login item and background activity
    (docs/adr/0060-login-items-and-background-activity.mdx), and its answer when the session asks it
    to quit before logging out, restarting or shutting down
    (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx). */
export const application = createApplication(NativeOxyOSApplication, session, DeviceEventEmitter);

/** The machine's Oxy session, for an application the session launched
    (docs/adr/0020-oxy-account-login.mdx, decision 6). */
export const account = createAccount(
  TurboModuleRegistry.get('OxyOSAccountSession') as AccountModule | null,
  DeviceEventEmitter,
);

/** The session's search index (docs/adr/0051-search.mdx). */
export const search = createSearch(TurboModuleRegistry.get('OxyOSSearch') as SearchModule | null, DeviceEventEmitter);
