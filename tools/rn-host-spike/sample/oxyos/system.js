// What of the session's vocabulary has not moved to `@oxyos/platform` yet
// (docs/adr/0058-the-platform-api.mdx, step 9): signing out, the input sources, AutoFill's list,
// the shell's system sheets, the launch's capabilities and those asked at first use
// (docs/adr/0030-installing-applications.mdx, decision 8). The session decides; nothing here
// grants anything. A request that changes anything cites the latest press, as the session requires.
import * as React from 'react';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';
import { session as connection } from '@oxyos/platform';

const native = TurboModuleRegistry.get('OxyOSSystem');
// The one connection to the session is the platform API's (docs/adr/0058-the-platform-api.mdx):
// this module is its older door, kept for the callers that have not moved to `@oxyos/platform`
// yet, over the same requests and the same followed state (step 9 removes it).
const { state } = connection;

/** Whether the session can be asked here (the host has the module). */
export const available = connection.available;

/** One request of the session's vocabulary; its final answer (`{refused: …}`, `{failed: …}` too). */
export function ask(request, onProgress) {
  return connection.request(request, { onProgress });
}

/** The state `source` follows, for a view: asked for as its first follower subscribes. */
function useFollowed(source) {
  return React.useSyncExternalStore(source.subscribe, source.current);
}

/**
 * The person's session (docs/adr/0020-oxy-account-login.mdx, decision 5): Settings (its `settings`
 * grant) signs out, citing the press or key that asked, and the session ends, back to the login
 * screen; answered `{signingOut: {}}` first, or `{refused: {reason}}` (`notPermitted`, `needsInput`).
 */
export const session = {
  signOut: () => ask({ signOut: {} }),
  // Sleep, restart, shut down and log out (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx):
  // the shell's Oxy menu and the login screen's buttons, with the `shell` grant, citing the press
  // or key that asked. The session decides: it asks its own question (`confirm`), every
  // application, then logind. Answered `{confirming}`, `{ending: {action}}`, `{sleeping}` or
  // `{refused: {reason}}`.
  endSession: (action, confirm) => ask({ endSession: { action, confirm } }),
  sleep: () => ask({ sleep: {} }),
};

/**
 * Settings › Storage (docs/adr/0062-storage.mdx), with Settings' `settings` grant: the session's
 * measurement of the volume, the system's image, the Trash and each application, asked as the
 * page shows it (`{storage: {volume, system, trash, apps}}`, nothing followed); an item shown in
 * the file manager, citing the press that asked (`{revealed: {}}`); and an application removed
 * through the store's uninstall, confirmed in the shell's system sheet (`{uninstalled: {id}}` or
 * `{cancelled: {}}`). A refusal is `{refused: {reason}}`.
 */
export const storage = {
  measure: () => ask({ storage: {} }),
  reveal: (uri) => ask({ reveal: { uri } }),
  remove: (id) => ask({ uninstall: { id } }),
  // Reset… (docs/adr/0061-crashes-and-resetting-an-application.mdx): the session confirms it in
  // its sheet, quits the application and erases its own data (`{reset: {id}}`).
  reset: (id) => ask({ resetApplication: { id } }),
};

/**
 * Backups (docs/adr/0059-backups.mdx): the shell (its `shell` grant: the menu bar's backup item)
 * and Settings (`settings`: its Backups page) are told the backup service's state as it changes:
 * followed as `{backup: State}`, `{backup: null}` while the service is not there, and null before
 * the session said (State: `{automatic, destinations, excluded, running, lastBackup, nextBackup,
 * held, administrator, inMenuBar}`). Both back up now and skip a backup, citing the press that
 * asked; Settings alone lists the disks that can be added (`{backupDisks: [disk]}`), adds a disk or
 * a share (`{backupAddDisk: {id}}`, `{backupAddShare: {id}}`), removes a destination, sets the
 * excluded items and Back Up Automatically, each citing the press. A refusal is
 * `{refused: {reason}}`, what the service would not do `{failed: {reason}}`. The service decides
 * who is an administrator.
 */
export const backup = {
  state: () => ask({ backup: {} }),
  backUpNow: () => ask({ backupNow: {} }),
  stop: () => ask({ backupStop: {} }),
  disks: () => ask({ backupDisks: {} }),
  addDisk: (uuid, password) => ask({ backupAddDisk: { uuid, password } }),
  addShare: (share, sharePassword, password) => ask({ backupAddShare: { share, sharePassword, password } }),
  remove: (id) => ask({ backupRemove: { id } }),
  setExcluded: (paths) => ask({ backupExcluded: { paths } }),
  setAutomatic: (enabled) => ask({ backupAutomatic: { enabled } }),
  current: state.backup.current,
  subscribe: state.backup.subscribe,
};

/** Backups as the session tells them, for a view: asked as it is first followed. */
export function useBackupState() {
  return useFollowed(backup);
}

/**
 * The input sources (docs/adr/0041-input-methods.mdx, decision 6): the session tells Settings (its
 * `settings` grant) and the shell the sources in effect and the input methods its engine has, as
 * they change, and answers `{inputSources: {}}` with them. Settings changes them through the
 * `input_sources` and `input_source` preferences; the shell's top bar asks the session for one
 * (`select`, answered with the new state) and for the emoji picker (`showEmoji`, answered
 * `{emoji: {}}`), citing the press or key that chose it; a refusal is `{refused: {reason}}`.
 */
export const inputSources = {
  state: () => ask({ inputSources: {} }),
  select: (id) => ask({ selectInputSource: { id } }),
  showEmoji: () => ask({ showEmoji: {} }),
  current: state.inputSources.current,
  subscribe: state.inputSources.subscribe,
};

/** The input sources as the session tells them, for a view: asked once, then followed. */
export function useInputSourcesState() {
  return useFollowed(inputSources);
}

/**
 * AutoFill (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 6): the session tells the
 * shell (its `shell` grant) the list to draw at a field's caret (`{autofill: {content, items,
 * highlighted}}`, `{autofill: null}` to hide it), and the shell picks the item pressed
 * (`choose`, citing the press), answered `{chosen: {}}` or `{refused: {reason}}`. The application
 * whose field it is never sees the list.
 */
export const autofill = {
  choose: (index) => ask({ chooseAutofill: { index } }),
  current: state.autofill.current,
  subscribe: state.autofill.subscribe,
};

/** AutoFill's list as the session tells it, for a view (told unasked, never asked for). */
export function useAutofillState() {
  return React.useSyncExternalStore(autofill.subscribe, autofill.current);
}

/** What this launch may use: `{origin, capabilities, narrowed}` (an installed version is narrowed
    to what its account granted). */
export function launch() {
  return native ? native.getLaunch() : null;
}

/** Asks the session for a capability asked at first use (notifications, files, camera,
    microphone, location); whether it is granted. Granted at once, it holds for this process. */
export const requestCapability = (capability) => connection.requestCapability(capability);

/** The shell's system sheets: those waiting, in order, and how the shell answers one. */
export function useSystemSheets() {
  const [sheets, setSheets] = React.useState([]);
  React.useEffect(() => {
    if (!native) return undefined;
    const shown = DeviceEventEmitter.addListener('OxyOSSheet', ({ sheet, text }) =>
      setSheets((waiting) => (waiting.some((s) => s.sheet === sheet) ? waiting : [...waiting, { sheet, text }])),
    );
    const gone = DeviceEventEmitter.addListener('OxyOSSheetDismissed', ({ sheet }) =>
      setSheets((waiting) => waiting.filter((s) => s.sheet !== sheet)),
    );
    return () => {
      shown.remove();
      gone.remove();
    };
  }, []);
  const answer = React.useCallback((sheet, accepted) => {
    native?.answerSheet(sheet, accepted);
    setSheets((waiting) => waiting.filter((s) => s.sheet !== sheet));
  }, []);
  // A Wi-Fi password sheet's answer (docs/adr/0038-networks.mdx): the password the person typed,
  // to the session alone, citing the press or key that submitted it. The session takes it only
  // from the shell, for a password sheet it shows, and hands it to the network daemon.
  const answerPassword = React.useCallback((sheet, password) => {
    native?.request(JSON.stringify({ password: { sheet, password } }));
    setSheets((waiting) => waiting.filter((s) => s.sheet !== sheet));
  }, []);
  // A Bluetooth pairing code's answer (docs/adr/0055-bluetooth.mdx): the PIN or passkey the person
  // typed, to the session alone, citing the press or key that submitted it, as a password is.
  const answerCode = React.useCallback((sheet, code) => {
    native?.request(JSON.stringify({ bluetoothCode: { sheet, code } }));
    setSheets((waiting) => waiting.filter((s) => s.sheet !== sheet));
  }, []);
  // A passkey sheet's answer (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7): the
  // passkey chosen and the Oxy account's password the person typed, to the session alone, citing
  // the press or key that submitted it. The session takes it only from the shell, for a passkey
  // sheet it shows, and hands it to the accounts' service, never to the application.
  const answerPasskey = React.useCallback((sheet, choice, password) => {
    const passkey = choice === undefined ? { sheet, password } : { sheet, choice, password };
    native?.request(JSON.stringify({ passkey }));
    setSheets((waiting) => waiting.filter((s) => s.sheet !== sheet));
  }, []);
  // An application that quit (docs/adr/0061-crashes-and-resetting-an-application.mdx): its
  // notice's Reset… answers the notice no and asks the session to reset it, citing the press; the
  // session asks the person in a sheet of its own, and opens it again once it is reset.
  const resetApplication = React.useCallback((sheet, id) => {
    native?.answerSheet(sheet, false);
    void connection.request({ resetApplication: { id, reopen: true } });
    setSheets((waiting) => waiting.filter((s) => s.sheet !== sheet));
  }, []);
  // The power key's sheet (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx, decision 7): its
  // Restart and Sleep answer it, citing the press, the session taking them only for the power
  // key's sheet it shows.
  const choosePowerKey = React.useCallback((sheet, choice) => {
    void connection.request({ powerKeyChoice: { sheet, choice } });
    setSheets((waiting) => waiting.filter((s) => s.sheet !== sheet));
  }, []);
  // Reopen windows when logging back in, toggled in the session's question: the person's
  // `reopen_windows`, which the session keeps.
  const reopenWindows = React.useCallback((enabled) => {
    void connection.request({ reopenWindows: { enabled } });
  }, []);
  return { sheets, answer, answerPassword, answerCode, answerPasskey, resetApplication, choosePowerKey, reopenWindows };
}
