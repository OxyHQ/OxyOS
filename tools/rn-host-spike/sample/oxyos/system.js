// Installing applications on OxyOS (docs/adr/0030-installing-applications.mdx, decisions 8, 9
// and 16), over the Bloom host's `OxyOSSystem`: the session's vocabulary for the store (the `store`
// grant), Settings' Software Update (`updates`) and Privacy (`privacy`), Wi-Fi and networks
// (`network`), Bluetooth (`bluetooth`), the shell's system sheets, and the capabilities an
// application asks for at first use. The session decides; nothing here
// grants anything. A request that changes anything cites the latest press, as the session requires.
import * as React from 'react';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

const native = TurboModuleRegistry.get('OxyOSSystem');

/** Requests waiting for their answer, by serial; progress goes to `onProgress`. */
const waiting = new Map();

/**
 * State the session tells as it changes (docs/adr/0058-the-platform-api.mdx, decision 3: followed
 * state): the last told (`current()`, null before the first) and who follows it (`subscribe`,
 * which returns the unsubscribe), the shape React's `useSyncExternalStore` takes.
 */
function followed() {
  let last = null;
  const listeners = new Set();
  return {
    current: () => last,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    tell(state) {
      last = state;
      for (const listener of listeners) listener(state);
    },
  };
}

/** The account's update state (decision 16). */
const updateState = followed();
/** The network (docs/adr/0038-networks.mdx). */
const networkState = followed();
/** Bluetooth (docs/adr/0055-bluetooth.mdx). */
const bluetoothState = followed();
/** The battery and Low Power Mode (docs/adr/0057-battery-and-energy.mdx). */
const powerState = followed();
/** The input sources (docs/adr/0041-input-methods.mdx). */
const inputSourcesState = followed();
/** The sound's outputs and inputs (docs/adr/0056-sound-devices.mdx). */
const soundState = followed();
/** AutoFill's list the session last told the shell to draw (docs/adr/0050-keychain-autofill-and-passkeys.mdx),
    null for none. */
const autofillState = followed();
/** Who follows the input's level, which is told only while one does. */
const inputLevelListeners = new Set();

/** What an answer, asked for or told unasked, says of the state the session follows. */
function tellAll(answer) {
  if (!answer) return;
  if (answer.updateState) updateState.tell(answer.updateState);
  if (answer.network) networkState.tell(answer.network);
  if (answer.bluetooth) bluetoothState.tell(answer.bluetooth);
  if (answer.inputSources) inputSourcesState.tell(answer.inputSources);
  if (answer.sound) soundState.tell(answer.sound);
  if (answer.power) powerState.tell(answer.power);
}

if (native) {
  DeviceEventEmitter.addListener('OxyOSSystemAnswer', ({ serial, answer }) => {
    // The state the session follows, unasked, as it changes.
    if (serial === 0) {
      tellAll(answer);
      if (answer && 'autofill' in answer) autofillState.tell(answer.autofill ?? null);
      if (answer && typeof answer.inputLevel === 'number') {
        for (const listener of inputLevelListeners) listener(answer.inputLevel);
      }
      return;
    }
    const asked = waiting.get(serial);
    if (!asked) return;
    if (answer && answer.progress !== undefined) {
      asked.onProgress?.(answer.progress);
      return;
    }
    waiting.delete(serial);
    tellAll(answer);
    asked.resolve(answer);
  });
}

/** The state `source` follows, for a view: asked once (`ask`) as the view mounts, then followed. */
function useFollowed(source, ask) {
  const state = React.useSyncExternalStore(source.subscribe, source.current);
  React.useEffect(() => {
    ask();
  }, []);
  return state;
}

/** Whether the session can be asked here (the host has the module). */
export const available = Boolean(native);

/** One request of the session's vocabulary; its final answer (`{refused: …}`, `{failed: …}` too). */
export function ask(request, onProgress) {
  if (!native) return Promise.resolve({ failed: { reason: 'unavailable' } });
  return new Promise((resolve) => {
    const serial = native.request(JSON.stringify(request));
    if (!serial) {
      resolve({ failed: { reason: 'notSent' } });
      return;
    }
    waiting.set(serial, { resolve, onProgress });
  });
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
  current: updateState.current,
  subscribe: updateState.subscribe,
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
  current: networkState.current,
  subscribe: networkState.subscribe,
};

/** The network as it changes, for a view: asked once, then followed. */
export function useNetworkState() {
  return useFollowed(network, network.state);
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
  current: bluetoothState.current,
  subscribe: bluetoothState.subscribe,
};

/** Bluetooth as it changes, for a view: asked once, then followed. */
export function useBluetoothState() {
  return useFollowed(bluetooth, bluetooth.state);
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
  current: powerState.current,
  subscribe: powerState.subscribe,
};

/** The battery and Low Power Mode as they change, for a view: asked once, then followed. */
export function usePowerState() {
  return useFollowed(power, power.state);
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
  current: inputSourcesState.current,
  subscribe: inputSourcesState.subscribe,
};

/** The input sources as the session tells them, for a view: asked once, then followed. */
export function useInputSourcesState() {
  return useFollowed(inputSources, inputSources.state);
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
  current: autofillState.current,
  subscribe: autofillState.subscribe,
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
  current: soundState.current,
  subscribe: soundState.subscribe,
};

/** The sound as the session tells it, for a view: asked once, then followed. */
export function useSoundState() {
  return useFollowed(sound, sound.state);
}

/** Follows the chosen input's level (0 to 1): metered from the first follower until the last
    stops, so nothing listens to the input while no view shows its level (ADR 0056, decision 7).
    Returns the stop. */
export function followInputLevel(listener) {
  if (!native) return () => {};
  inputLevelListeners.add(listener);
  if (inputLevelListeners.size === 1) sound.meter(true);
  return () => {
    if (!inputLevelListeners.delete(listener)) return;
    if (inputLevelListeners.size === 0) sound.meter(false);
  };
}

/** The update state as it changes, for a page: asked once, then followed. */
export function useUpdateState() {
  return useFollowed(updates, updates.state);
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
  // An application that quit (docs/adr/0061-crashes-and-resetting-an-application.mdx): its
  // notice's Reset… answers the notice no and asks the session to reset it, citing the press; the
  // session asks the person in a sheet of its own, and opens it again once it is reset.
  const resetApplication = React.useCallback((sheet, id) => {
    native?.answerSheet(sheet, false);
    native?.request(JSON.stringify({ resetApplication: { id, reopen: true } }));
    setSheets((waiting) => waiting.filter((s) => s.sheet !== sheet));
  }, []);
  return { sheets, answer, answerPassword, answerCode, resetApplication };
}
