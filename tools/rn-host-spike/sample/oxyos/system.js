// Installing applications on OxyOS (docs/adr/0030-installing-applications.mdx, decisions 8, 9
// and 16), over the Bloom host's `OxyOSSystem`: the session's vocabulary for the store (the `store`
// grant), Settings' Software Update (`updates`) and Privacy (`privacy`), Wi-Fi and networks
// (`network`), Bluetooth (`bluetooth`), the shell's system sheets, and the capabilities an
// application asks for at first use. The session decides; nothing here
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

/** The store's (Atlas's) requests: `store` grant. */
export const store = {
  status: (ids) => ask({ status: { ids } }),
  install: (id, onProgress) => ask({ install: { id } }, onProgress),
  uninstall: (id) => ask({ uninstall: { id } }),
  open: (id) => ask({ open: { id } }),
};

/** The account's updates: `updates` (or `store`) grant. One state, the session's. */
export const updates = {
  state: () => ask({ state: {} }),
  check: () => ask({ check: {} }),
  update: (ids) => ask({ update: { ids } }),
  updateAll: () => ask({ updateAll: {} }),
  current: state.updateState.current,
  subscribe: state.updateState.subscribe,
};

/**
 * The person's session (docs/adr/0020-oxy-account-login.mdx, decision 5): Settings (its `settings`
 * grant) signs out, citing the press or key that asked, and the session ends, back to the login
 * screen; answered `{signingOut: {}}` first, or `{refused: {reason}}` (`notPermitted`, `needsInput`).
 */
export const session = {
  signOut: () => ask({ signOut: {} }),
};

/** The account's decisions on its applications' capabilities: `privacy` grant. */
export const privacy = {
  list: () => ask({ privacy: {} }),
  decide: (id, capability, granted) => ask({ decide: { id, capability, granted } }),
};

/**
 * Wi-Fi and networks (docs/adr/0038-networks.mdx): the shell (its `shell` grant) and Settings (the
 * `network` grant) ask the session, which alone speaks to NetworkManager. The state is told as it
 * changes; a join is answered once it is done (`{joined}` or `{failed: {reason}}`), a password
 * asked in the shell's system sheet on the way. Nothing here ever holds a password but the
 * shell's own answer to that sheet.
 */
export const network = {
  state: () => ask({ networks: {} }),
  scan: () => ask({ scan: {} }),
  join: (ssid) => ask({ join: { ssid } }),
  forget: (id) => ask({ forget: { id } }),
  setWifi: (enabled) => ask({ wifi: { enabled } }),
  current: state.network.current,
  subscribe: state.network.subscribe,
};

/** The network as it changes, for a view: asked once, then followed. */
export function useNetworkState() {
  return useFollowed(network);
}

/**
 * Bluetooth (docs/adr/0055-bluetooth.mdx): the shell (its `shell` grant) and Settings (the
 * `bluetooth` grant) ask the session, which alone speaks to BlueZ and is the machine's pairing
 * agent. The state is told as it changes; a connection is answered once it is done
 * (`{connected}`, `{paired}` or `{failed: {reason}}`), any code a pairing needs asked or shown in
 * the shell's system sheets on the way. Settings holds discovery while its Bluetooth page is open
 * (`discover(true)`, then `discover(false)`); the session ends it when Settings goes.
 */
export const bluetooth = {
  state: () => ask({ bluetooth: {} }),
  setPower: (on) => ask({ bluetoothPower: { on } }),
  discover: (on) => ask({ bluetoothDiscover: { on } }),
  connect: (id) => ask({ bluetoothConnect: { id } }),
  disconnect: (id) => ask({ bluetoothDisconnect: { id } }),
  forget: (id) => ask({ bluetoothForget: { id } }),
  rename: (id, name) => ask({ bluetoothRename: { id, name } }),
  current: state.bluetooth.current,
  subscribe: state.bluetooth.subscribe,
};

/** Bluetooth as it changes, for a view: asked once, then followed. */
export function useBluetoothState() {
  return useFollowed(bluetooth);
}

/**
 * Battery and energy (docs/adr/0057-battery-and-energy.mdx): the shell (its `shell` grant: the
 * battery's menu) and Settings (`settings`: its Battery page) are told the state as it changes
 * (`{battery, health, chargeLimit, lowPower, showPercentage, criticalAction}`), and ask for the
 * charge history (`{powerHistory: {day, days}}`) and each application's energy, sampled as asked
 * (`{energy: {apps}}`). Settings alone limits the battery's charge, citing the press that asked
 * (`{chargeLimit: {}}` or `{failed: {reason}}`). Low Power Mode, the timers and the menu bar's
 * percentage are preferences, which Settings changes as it changes any.
 */
export const power = {
  state: () => ask({ power: {} }),
  history: () => ask({ powerHistory: {} }),
  energy: () => ask({ energy: {} }),
  setChargeLimit: (enabled) => ask({ chargeLimit: { enabled } }),
  current: state.power.current,
  subscribe: state.power.subscribe,
};

/** The battery and Low Power Mode as they change, for a view: asked once, then followed. */
export function usePowerState() {
  return useFollowed(power);
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

/**
 * The sound's outputs and inputs (docs/adr/0056-sound-devices.mdx): the shell (its `shell` grant)
 * and Settings (its `settings` grant) ask the session, which alone speaks to PipeWire. Asked once,
 * the state is told again as it changes (`{sound: {available, output, input}}`, each side its
 * `devices` with `id`, `name`, `kind`, `transport` and `selected`, and the chosen one's `volume`,
 * `muted` and `balance`). What changes the machine cites the press or key that asked; while locked
 * it is refused. The input's level is told only while a view meters it (`meter(true)`), and
 * nothing listens to the input once none does.
 */
export const sound = {
  state: () => ask({ sound: {} }),
  select: (direction, id) => ask({ soundDevice: { direction, id } }),
  setVolume: (direction, volume) => ask({ soundVolume: { direction, volume } }),
  setMuted: (direction, muted) => ask({ soundMuted: { direction, muted } }),
  setBalance: (balance) => ask({ soundBalance: { balance } }),
  meter: (on) => ask({ soundMeter: { on } }),
  current: state.sound.current,
  subscribe: state.sound.subscribe,
};

/** The sound as the session tells it, for a view: asked once, then followed. */
export function useSoundState() {
  return useFollowed(sound);
}

/** Follows the chosen input's level (0 to 1): metered from the first follower until the last
    stops, so nothing listens to the input while no view shows its level (ADR 0056, decision 7).
    Returns the stop. */
export function followInputLevel(listener) {
  if (!native) return () => {};
  return state.inputLevel.subscribe(listener);
}

/** The update state as it changes, for a page: asked once, then followed. */
export function useUpdateState() {
  return useFollowed(updates);
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
