/**
 * The compositor's windows and workspaces as the session's shell knows them (OxyOSShell, ADR 0009),
 * and the shell's actions on them. Only a Bloom host run as the shell (`--shell`) has the module.
 *
 * The state: { windows: [{ id, appId, title, workspace, minimized, focused, launched, width, height, used, hidden }],
 * workspaces, active,
 * locked, user: { name, fullName }, capsLock, wallpaper, applications }: wallpaper the file of the
 * solar wallpaper's frame that shows the sun now (null without one); applications the session's
 * registry (ADR 0013), [{ id, name, icon: { uri } | { svg } | null, pinned: place | null }]., windows numbered by the compositor and workspaces from 0.
 * And the outputs (ADR 0018): `output` the one this shell runs on, `outputs` every one, [{ id, x, y,
 * width, height, scale, primary }], in the global space's logical pixels; the primary has the dock.
 * Actions are intents: the next state says what happened.
 *
 * The menu bar (docs/adr/0021-menu-bar.mdx): `menuBar` is the active window's menus, { window,
 * serial, items: [{ parent, kind: 'menu' | 'item' | 'submenu' | 'separator', enabled, checkable,
 * checked, label, shortcut }] } (window 0: none is active; no items: it declared none; parent an
 * earlier item's index, -1 for a menu of the bar). `activateMenuItem(window, serial, index)` chooses
 * one; the compositor gives it to the application only while those menus are the active window's.
 *
 * The lock (docs/adr/0010-lock-screen.mdx): `lockSession()` locks; `authenticate(password)` sends
 * the password typed in the lock screen for the session to check, and `authentication` is the
 * latest answer, { id, outcome: 'accepted' | 'rejected' | 'busy' | 'throttled', wait } (wait: the
 * seconds until another may be tried). Only the session unlocks: `locked` then says so.
 *
 * The system shortcuts (docs/adr/0024-system-shortcuts.mdx): `shortcuts` are the latest the
 * compositor took from the keyboard for the shell to show, in order, [{ id, action, output }] (a
 * rising `id` for each, as several may come before the shell draws again; action 'switcher-next' | 'switcher-previous' | 'switcher-quit' | 'switcher-hide' |
 * 'switcher-done' | 'switcher-cancel' | 'search' | 'menu-bar'; output the one to show it on).
 * `hideApplication(window)` and `quitApplication(window)` hide or quit the application of a window.
 *
 * Screenshots (docs/adr/0024-system-shortcuts.mdx, decision 10): `capture` is the screenshot
 * interface the compositor opened, { serial, mode: 'selection' | 'window' | 'toolbar', output, confirm }
 * (Super+Shift+4's crosshair on every display, Space switching it to picking a window and back;
 * Super+Shift+5's toolbar on `output`'s display; `confirm` counting Return pressed in the toolbar),
 * null once it ended (a capture, Escape, another display's capture, the lock); `captureTarget` the
 * window under the pointer while it is open, { window, x, y, width, height } in the global space
 * (window 0: none); `screenshots` the latest saved, in order, [{ id, output, width, height, path }]
 * (each on the display it is of). `capture(kind, output, rect, window, destination)` captures citing
 * the interface's serial (kind 'screens' | 'area' | 'window', rect { x, y, width, height } global or
 * null, destination 'desktop' | 'documents'); `endCapture()` ends it without one;
 * `openScreenshot(id)` opens a saved one, citing the latest input (the press on its thumbnail).
 *
 * The hardware keys and the Control Center (docs/adr/0025-hardware-keys-and-control-center.mdx):
 * `controls` in the state is what the session says of the machine, { output, input, display,
 * keyboard: { level (0 to 1), muted, available } | null, power: { battery, percent, onAC, minutes }
 * | null, network: { wifi, signal, ssid, wired } | null, bluetooth: { state, connected } | null };
 * `bezels` are the latest levels a hardware key acted on, for the bezel, in order, [{ id, control }]
 * (control 'output' | 'input' | 'display' | 'keyboard'). `setControlLevel(control, level)`,
 * `setControlMuted(control, muted)`, `setWifiEnabled(enabled)`, `setAppearance(dark)` and
 * `refreshControls()` are the Control Center's intents: the next state says what happened.
 *
 * The login screen (docs/adr/0020-oxy-account-login.mdx): `authenticate(password, account)` sends the
 * password for an enrolled account's Unix user, and `accounts()` reads the enrolled accounts from the
 * accounts' service, [{ user, name, picture }] (null when they cannot be read); `oxyApi` in the state
 * is Oxy's API, which adding an account needs to reach.
 *
 * Notifications (docs/adr/0026-notifications.mdx): `notifications` in the state, [{ id, appId,
 * title, subtitle, body, actions: [{ label, reply }], postedAt, banner, withheld }] (reply: a reply
 * action's placeholder, null for a button), and `notificationPolicy`, { doNotDisturb, previews }.
 * `activateNotification(id, action, reply)` opens one (action 0) or runs its action, citing the
 * latest input; `removeNotification(id)` removes one (0: all); `setDoNotDisturb(enabled)` is the
 * Control Center's (a preference change, as `setAppearance`); `announce(text, polite)` speaks.
 */
import * as React from 'react';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

const native = TurboModuleRegistry.get('OxyOSShell');

/**
 * `next` with every part equal to `previous`'s (deeply, as JSON) being `previous`'s own: the state
 * the host told again is the same object, and of a new one only what changed is new. React then
 * renders nothing for a state that changed nothing, and of one that changed something only what
 * reads the parts that changed (each part compared by identity: a window's size changing leaves the
 * applications, the menus and the other windows as they were).
 */
export function sharedWith(previous, next) {
  if (previous === next) return previous;
  if (typeof previous !== 'object' || typeof next !== 'object' || previous === null || next === null) return next;
  if (Array.isArray(previous)) {
    if (!Array.isArray(next)) return next;
    let same = previous.length === next.length;
    const shared = next.map((value, index) => {
      const kept = index < previous.length ? sharedWith(previous[index], value) : value;
      if (kept !== previous[index]) same = false;
      return kept;
    });
    return same ? previous : shared;
  }
  if (Array.isArray(next)) return next;
  const keys = Object.keys(next);
  let same = keys.length === Object.keys(previous).length;
  const shared = {};
  for (const key of keys) {
    const kept = Object.prototype.hasOwnProperty.call(previous, key) ? sharedWith(previous[key], next[key]) : next[key];
    if (kept !== previous[key]) same = false;
    shared[key] = kept;
  }
  return same ? previous : shared;
}

// The shell's actions on the compositor: intents, the same functions in every render, so that what
// is given them is never new because of them.
const actions = {
  focusWindow: (id) => native?.focusWindow(id),
  closeWindow: (id) => native?.closeWindow(id),
  minimizeWindow: (id) => native?.minimizeWindow(id),
  hideApplication: (id) => native?.hideApplication(id),
  quitApplication: (id) => native?.quitApplication(id),
  moveWindowToWorkspace: (id, workspace) => native?.moveWindowToWorkspace(id, workspace),
  activateWorkspace: (workspace) => native?.activateWorkspace(workspace),
  addWorkspace: () => native?.addWorkspace(),
  removeWorkspace: (workspace) => native?.removeWorkspace(workspace),
  lockSession: () => native?.lockSession(),
  // Asks the session for an application of its registry (ADR 0013): the request's serial (0:
  // not sent); `OxyOSShellLaunch` says whether it was started.
  launch: (id) => native?.launch(id) ?? 0,
  authenticate: (password, account) => native?.authenticate(password, account) ?? false,
  accounts: () => native?.accounts() ?? null,
  activateMenuItem: (window, serial, index) => native?.activateMenuItem(window, serial, index),
  setControlLevel: (control, level) => native?.setControlLevel(control, level),
  setControlMuted: (control, muted) => native?.setControlMuted(control, muted),
  setWifiEnabled: (enabled) => native?.setWifiEnabled(enabled),
  setAppearance: (dark) => native?.setAppearance(dark),
  refreshControls: () => native?.refreshControls(),
  activateNotification: (id, action, reply) => native?.activateNotification(id, action, reply ?? ''),
  removeNotification: (id) => native?.removeNotification(id),
  setDoNotDisturb: (enabled) => native?.setDoNotDisturb(enabled),
  announce: (text, polite) => native?.announce(text, polite),
  openScreenshot: (id) => native?.openScreenshot(id) ?? false,
};

export function useOxyOSShell() {
  const [state, setState] = React.useState(() => native?.getState() ?? null);
  const [authentication, setAuthentication] = React.useState(null);
  const [shortcuts, setShortcuts] = React.useState([]);
  const [bezels, setBezels] = React.useState([]);
  const [capture, setCapture] = React.useState(null);
  const [captureTarget, setCaptureTarget] = React.useState(null);
  const [screenshots, setScreenshots] = React.useState([]);
  React.useEffect(() => {
    if (!native) return undefined;
    const told = (next) => setState((previous) => sharedWith(previous, next));
    const subscription = DeviceEventEmitter.addListener('OxyOSShellChanged', told);
    // Each answer is new, even one like the last (a second wrong password shakes again).
    const answers = DeviceEventEmitter.addListener('OxyOSShellAuthentication', ({ outcome, wait }) =>
      setAuthentication((last) => ({ id: (last?.id ?? 0) + 1, outcome, wait })),
    );
    // Each shortcut is new, even one like the last (Tab pressed again moves on).
    const shortcut = DeviceEventEmitter.addListener('OxyOSShellShortcut', ({ action, output }) =>
      setShortcuts((last) => [
        ...last.slice(-15),
        { id: (last[last.length - 1]?.id ?? 0) + 1, action, output },
      ]),
    );
    // Each bezel is new, even one like the last (a key pressed at the end of its range shows it).
    const bezel = DeviceEventEmitter.addListener('OxyOSShellBezel', ({ control }) =>
      setBezels((last) => [...last.slice(-15), { id: (last[last.length - 1]?.id ?? 0) + 1, control }]),
    );
    // The screenshot interface: opened, switched, confirmed, ended; one interface per serial.
    const captured = DeviceEventEmitter.addListener('OxyOSShellCapture', ({ mode, serial, output }) => {
      if (mode === 'ended') {
        setCapture(null);
        setCaptureTarget(null);
        return;
      }
      setCapture((last) => {
        const same = last && last.serial === serial;
        if (mode === 'confirm') return same ? { ...last, confirm: last.confirm + 1 } : last;
        return { serial, mode, output, confirm: same ? last.confirm : 0 };
      });
    });
    // The window under the pointer, kept while it is the same one at the same place.
    const targeted = DeviceEventEmitter.addListener('OxyOSShellCaptureTarget', (target) =>
      setCaptureTarget((last) => sharedWith(last, target.window ? target : null)),
    );
    const saved = DeviceEventEmitter.addListener('OxyOSShellScreenshot', (screenshot) =>
      setScreenshots((last) => [...last.slice(-15), screenshot]),
    );
    // A change between the first read and the subscription is not lost.
    told(native.getState());
    return () => {
      subscription.remove();
      answers.remove();
      shortcut.remove();
      bezel.remove();
      captured.remove();
      targeted.remove();
      saved.remove();
    };
  }, []);
  // Capturing cites the interface open now: read when asked, so the function stays the same.
  const open = React.useRef(capture);
  open.current = capture;
  const capturing = React.useMemo(
    () => ({
      captureScreen: (kind, output, rect, window, destination) => {
        const now = open.current;
        return now ? (native?.capture(now.serial, kind, output, rect ?? null, window ?? 0, destination) ?? false) : false;
      },
      endCapture: () => (open.current ? (native?.endCapture(open.current.serial) ?? false) : false),
    }),
    [],
  );
  return React.useMemo(
    () => ({
      ...actions,
      ...capturing,
      state,
      shortcuts,
      authentication,
      bezels,
      capture,
      captureTarget,
      screenshots,
    }),
    [capturing, state, shortcuts, authentication, bezels, capture, captureTarget, screenshots],
  );
}
