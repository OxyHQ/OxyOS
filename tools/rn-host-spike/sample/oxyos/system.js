// Installing applications on OxyOS (docs/adr/0030-installing-applications.mdx, decisions 8, 9
// and 16), over the Bloom host's `OxyOSSystem`: the session's vocabulary for the store (the `store`
// grant), Settings' Software Update (`updates`) and Privacy (`privacy`), the shell's system sheets,
// and the capabilities an application asks for at first use. The session decides; nothing here
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
  return { sheets, answer, answerPassword };
}
