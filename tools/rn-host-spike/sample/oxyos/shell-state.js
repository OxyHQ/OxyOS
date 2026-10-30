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
 * 'switcher-done' | 'switcher-cancel' | 'search'; output the one to show it on).
 * `hideApplication(window)` and `quitApplication(window)` hide or quit the application of a window.
 *
 * The login screen (docs/adr/0020-oxy-account-login.mdx): `authenticate(password, account)` sends the
 * password for an enrolled account's Unix user, and `accounts()` reads the enrolled accounts from the
 * accounts' service, [{ user, name, picture }] (null when they cannot be read); `oxyApi` in the state
 * is Oxy's API, which adding an account needs to reach.
 */
import * as React from 'react';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

const native = TurboModuleRegistry.get('OxyOSShell');

export function useOxyOSShell() {
  const [state, setState] = React.useState(() => native?.getState() ?? null);
  const [authentication, setAuthentication] = React.useState(null);
  const [shortcuts, setShortcuts] = React.useState([]);
  React.useEffect(() => {
    if (!native) return undefined;
    const subscription = DeviceEventEmitter.addListener('OxyOSShellChanged', setState);
    // Each answer is new, even one like the last (a second wrong password shakes again).
    const answers = DeviceEventEmitter.addListener('OxyOSShellAuthentication', ({ outcome, wait }) =>
      setAuthentication((last) => ({ id: (last?.id ?? 0) + 1, outcome, wait })),
    );
    // Each shortcut is new, even one like the last (Tab pressed again moves on).
    const told = DeviceEventEmitter.addListener('OxyOSShellShortcut', ({ action, output }) =>
      setShortcuts((last) => [
        ...last.slice(-15),
        { id: (last[last.length - 1]?.id ?? 0) + 1, action, output },
      ]),
    );
    // A change between the first read and the subscription is not lost.
    setState(native.getState());
    return () => {
      subscription.remove();
      answers.remove();
      told.remove();
    };
  }, []);
  return {
    state,
    focusWindow: (id) => native?.focusWindow(id),
    closeWindow: (id) => native?.closeWindow(id),
    minimizeWindow: (id) => native?.minimizeWindow(id),
    hideApplication: (id) => native?.hideApplication(id),
    quitApplication: (id) => native?.quitApplication(id),
    shortcuts,
    moveWindowToWorkspace: (id, workspace) => native?.moveWindowToWorkspace(id, workspace),
    activateWorkspace: (workspace) => native?.activateWorkspace(workspace),
    addWorkspace: () => native?.addWorkspace(),
    removeWorkspace: (workspace) => native?.removeWorkspace(workspace),
    authentication,
    lockSession: () => native?.lockSession(),
    // Asks the session for an application of its registry (ADR 0013): the request's serial (0:
    // not sent); `OxyOSShellLaunch` says whether it was started.
    launch: (id) => native?.launch(id) ?? 0,
    authenticate: (password, account) => native?.authenticate(password, account) ?? false,
    accounts: () => native?.accounts() ?? null,
    activateMenuItem: (window, serial, index) => native?.activateMenuItem(window, serial, index),
  };
}
