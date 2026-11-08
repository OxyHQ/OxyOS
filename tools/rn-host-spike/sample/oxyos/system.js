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
};

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
export function requestCapability(capability) {
  if (!native) return Promise.resolve(false);
  return new Promise((resolve) => {
    const serial = native.requestCapability(capability);
    if (!serial) {
      resolve(false);
      return;
    }
    const subscription = DeviceEventEmitter.addListener('OxyOSCapabilityResult', (result) => {
      if (result.serial !== serial) return;
      subscription.remove();
      resolve(result.granted);
    });
  });
}

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
  return { sheets, answer, answerPassword, answerCode };
}
