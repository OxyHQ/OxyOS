/**
 * The compositor's windows and workspaces as the session's shell knows them (OxyOSShell, ADR 0009),
 * and the shell's actions on them. Only a Bloom host run as the shell (`--shell`) has the module.
 *
 * The state: { windows: [{ id, appId, title, workspace, minimized, focused }], workspaces, active },
 * windows numbered by the compositor and workspaces from 0. Actions are intents: the next state
 * says what happened.
 */
import * as React from 'react';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

const native = TurboModuleRegistry.get('OxyOSShell');

export function useOxyOSShell() {
  const [state, setState] = React.useState(() => native?.getState() ?? null);
  React.useEffect(() => {
    if (!native) return undefined;
    const subscription = DeviceEventEmitter.addListener('OxyOSShellChanged', setState);
    // A change between the first read and the subscription is not lost.
    setState(native.getState());
    return () => subscription.remove();
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
  };
}
