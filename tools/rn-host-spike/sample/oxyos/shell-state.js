/**
 * The compositor's windows and workspaces as the session's shell knows them (OxyOSShell, ADR 0009),
 * and the shell's actions on them. Only a Bloom host run as the shell (`--shell`) has the module.
 *
 * The state: { windows: [{ id, appId, title, workspace, minimized, focused }], workspaces, active,
 * locked, user: { name, fullName }, capsLock, wallpaper, applications }: wallpaper the file of the
 * solar wallpaper's frame that shows the sun now (null without one); applications the session's
 * registry (ADR 0013), [{ id, name, icon: { uri } | { svg } | null, pinned: place | null }]., windows numbered by the compositor and workspaces from 0.
 * Actions are intents: the next state says what happened.
 *
 * The lock (docs/adr/0010-lock-screen.mdx): `lockSession()` locks; `authenticate(password)` sends
 * the password typed in the lock screen for the session to check, and `authentication` is the
 * latest answer, { id, outcome: 'accepted' | 'rejected' | 'busy' | 'throttled', wait } (wait: the
 * seconds until another may be tried). Only the session unlocks: `locked` then says so.
 */
import * as React from 'react';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

const native = TurboModuleRegistry.get('OxyOSShell');

export function useOxyOSShell() {
  const [state, setState] = React.useState(() => native?.getState() ?? null);
  const [authentication, setAuthentication] = React.useState(null);
  React.useEffect(() => {
    if (!native) return undefined;
    const subscription = DeviceEventEmitter.addListener('OxyOSShellChanged', setState);
    // Each answer is new, even one like the last (a second wrong password shakes again).
    const answers = DeviceEventEmitter.addListener('OxyOSShellAuthentication', ({ outcome, wait }) =>
      setAuthentication((last) => ({ id: (last?.id ?? 0) + 1, outcome, wait })),
    );
    // A change between the first read and the subscription is not lost.
    setState(native.getState());
    return () => {
      subscription.remove();
      answers.remove();
    };
  }, []);
  return {
    state,
    focusWindow: (id) => native?.focusWindow(id),
    closeWindow: (id) => native?.closeWindow(id),
    minimizeWindow: (id) => native?.minimizeWindow(id),
    moveWindowToWorkspace: (id, workspace) => native?.moveWindowToWorkspace(id, workspace),
    activateWorkspace: (workspace) => native?.activateWorkspace(workspace),
    addWorkspace: () => native?.addWorkspace(),
    removeWorkspace: (workspace) => native?.removeWorkspace(workspace),
    authentication,
    lockSession: () => native?.lockSession(),
    authenticate: (password) => native?.authenticate(password) ?? false,
  };
}
