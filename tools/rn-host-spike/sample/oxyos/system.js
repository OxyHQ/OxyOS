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
const stateListeners = new Set();
let lastState = null;
/** The network as the session last told it (docs/adr/0038-networks.mdx), and who follows it. */
const networkListeners = new Set();
let lastNetwork = null;

/** Bluetooth as the session last told it (docs/adr/0055-bluetooth.mdx), and who follows it. */
const bluetoothListeners = new Set();
let lastBluetooth = null;

/** The battery and Low Power Mode as the session last told them (docs/adr/0057-battery-and-
    energy.mdx), and who follows them. */
const powerListeners = new Set();
let lastPower = null;

function toldPower(state) {
  lastPower = state;
  for (const listener of powerListeners) listener(state);
}

/** The input sources as the session last told them (docs/adr/0041-input-methods.mdx), and who
    follows them. */
const inputSourcesListeners = new Set();
let lastInputSources = null;

/** The sound's outputs and inputs as the session last told them (docs/adr/0056-sound-devices.mdx),
    who follows them, and who follows the input's level. */
const soundListeners = new Set();
let lastSound = null;

/** AutoFill's list the session last told the shell to draw (docs/adr/0050-keychain-autofill-and-passkeys.mdx),
    null when none, and who follows it. */
const autofillListeners = new Set();
let lastAutofill = null;

function toldAutofill(list) {
  lastAutofill = list ?? null;
  for (const listener of autofillListeners) listener(lastAutofill);
}
const inputLevelListeners = new Set();

function toldSound(state) {
  lastSound = state;
  for (const listener of soundListeners) listener(state);
}

function toldInputSources(state) {
  lastInputSources = state;
  for (const listener of inputSourcesListeners) listener(state);
}

function toldBluetooth(state) {
  lastBluetooth = state;
  for (const listener of bluetoothListeners) listener(state);
}

function toldNetwork(state) {
  lastNetwork = state;
  for (const listener of networkListeners) listener(state);
}

function told(state) {
  lastState = state;
  for (const listener of stateListeners) listener(state);
}

if (native) {
  DeviceEventEmitter.addListener('OxyOSSystemAnswer', ({ serial, answer }) => {
    // The account's update state, unasked, as it changes.
    if (serial === 0) {
      if (answer && answer.updateState) told(answer.updateState);
      if (answer && answer.network) toldNetwork(answer.network);
      if (answer && answer.bluetooth) toldBluetooth(answer.bluetooth);
      if (answer && answer.inputSources) toldInputSources(answer.inputSources);
      if (answer && answer.sound) toldSound(answer.sound);
      if (answer && 'autofill' in answer) toldAutofill(answer.autofill);
      if (answer && typeof answer.inputLevel === 'number') {
        for (const listener of inputLevelListeners) listener(answer.inputLevel);
      }
      if (answer && answer.power) toldPower(answer.power);
      return;
    }
    const asked = waiting.get(serial);
    if (!asked) return;
    if (answer && answer.progress !== undefined) {
      asked.onProgress?.(answer.progress);
      return;
    }
    waiting.delete(serial);
    if (answer && answer.updateState) told(answer.updateState);
    if (answer && answer.network) toldNetwork(answer.network);
    if (answer && answer.bluetooth) toldBluetooth(answer.bluetooth);
    if (answer && answer.inputSources) toldInputSources(answer.inputSources);
    if (answer && answer.sound) toldSound(answer.sound);
    if (answer && answer.power) toldPower(answer.power);
    asked.resolve(answer);
  });
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
  current: () => lastState,
  subscribe(listener) {
    stateListeners.add(listener);
    return () => stateListeners.delete(listener);
  },
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
  current: () => lastNetwork,
  subscribe(listener) {
    networkListeners.add(listener);
    return () => networkListeners.delete(listener);
  },
};

/** The network as it changes, for a view: asked once, then followed. */
export function useNetworkState() {
  const [state, setState] = React.useState(lastNetwork);
  React.useEffect(() => {
    const unsubscribe = network.subscribe(setState);
    network.state();
    return unsubscribe;
  }, []);
  return state;
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
  current: () => lastBluetooth,
  subscribe(listener) {
    bluetoothListeners.add(listener);
    return () => bluetoothListeners.delete(listener);
  },
};

/** Bluetooth as it changes, for a view: asked once, then followed. */
export function useBluetoothState() {
  const [state, setState] = React.useState(lastBluetooth);
  React.useEffect(() => {
    const unsubscribe = bluetooth.subscribe(setState);
    bluetooth.state();
    return unsubscribe;
  }, []);
  return state;
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
  current: () => lastPower,
  subscribe(listener) {
    powerListeners.add(listener);
    return () => powerListeners.delete(listener);
  },
};

/** The battery and Low Power Mode as they change, for a view: asked once, then followed. */
export function usePowerState() {
  const [state, setState] = React.useState(lastPower);
  React.useEffect(() => {
    const unsubscribe = power.subscribe(setState);
    power.state();
    return unsubscribe;
  }, []);
  return state;
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
  current: () => lastInputSources,
  subscribe(listener) {
    inputSourcesListeners.add(listener);
    return () => inputSourcesListeners.delete(listener);
  },
};

/** The input sources as the session tells them, for a view: asked once, then followed. */
export function useInputSourcesState() {
  const [state, setState] = React.useState(lastInputSources);
  React.useEffect(() => {
    const unsubscribe = inputSources.subscribe(setState);
    inputSources.state();
    return unsubscribe;
  }, []);
  return state;
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
  current: () => lastAutofill,
  subscribe(listener) {
    autofillListeners.add(listener);
    return () => autofillListeners.delete(listener);
  },
};

/** AutoFill's list as the session tells it, for a view. */
export function useAutofillState() {
  const [state, setState] = React.useState(lastAutofill);
  React.useEffect(() => {
    setState(lastAutofill);
    return autofill.subscribe(setState);
  }, []);
  return state;
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
  current: () => lastSound,
  subscribe(listener) {
    soundListeners.add(listener);
    return () => soundListeners.delete(listener);
  },
};

/** The sound as the session tells it, for a view: asked once, then followed. */
export function useSoundState() {
  const [state, setState] = React.useState(lastSound);
  React.useEffect(() => {
    const unsubscribe = sound.subscribe(setState);
    sound.state();
    return unsubscribe;
  }, []);
  return state;
}

/** The chosen input's level (0 to 1) while `on`: metered from when it is on until it is off or
    the view goes, so nothing listens to the input meanwhile. */
export function useInputLevel(on) {
  const [level, setLevel] = React.useState(0);
  React.useEffect(() => {
    if (!on || !native) return undefined;
    inputLevelListeners.add(setLevel);
    sound.meter(true);
    return () => {
      inputLevelListeners.delete(setLevel);
      setLevel(0);
      if (inputLevelListeners.size === 0) sound.meter(false);
    };
  }, [on]);
  return level;
}

/** The update state as it changes, for a page: asked once, then followed. */
export function useUpdateState() {
  const [state, setState] = React.useState(lastState);
  React.useEffect(() => {
    const unsubscribe = updates.subscribe(setState);
    updates.state();
    return unsubscribe;
  }, []);
  return state;
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
