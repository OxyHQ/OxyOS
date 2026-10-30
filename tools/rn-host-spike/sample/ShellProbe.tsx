/**
 * The OxyOS shell (apps/shell) with a small fixture in place of the compositor's snapshot. In a
 * window of the Bloom host (`--window`) its regions are drawn in place; run as the session's
 * shell (`--shell`) they are panels and an overlay of their own (docs/adr/0009-session-shell.mdx).
 */
import * as React from 'react';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { oxyosShell } from './oxyos/shell-surface';
import { useOxyOSShell } from './oxyos/shell-state';
import { Shell } from '../../../apps/shell/src/Shell';
import { useMinuteClock } from '../../../apps/shell/src/useMinuteClock';
import type { ShellActions, ShellLock, ShellSnapshot } from '../../../apps/shell/src/model';

const icon = (name: string) => {
  switch (name) {
    case 'astro':
      return require('../../shell-preview/assets/apps/astro.png');
    case 'inbox':
      return require('../../shell-preview/assets/apps/inbox.png');
    case 'mention':
      return require('../../shell-preview/assets/apps/mention.png');
    default:
      return require('../../shell-preview/assets/apps/allo.png');
  }
};

// One vector icon, as the application registry may give one.
const ALIA =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="40" fill="#d269e6"/></svg>';

const snapshot: ShellSnapshot = {
  applications: [
    { id: 'astro', name: 'Astro', icon: icon('astro'), pinned: true },
    { id: 'inbox', name: 'Inbox', icon: icon('inbox'), pinned: true },
    { id: 'mention', name: 'Mention', icon: icon('mention'), pinned: true },
    { id: 'alia', name: 'Alia', icon: { svg: ALIA }, pinned: true },
    { id: 'allo', name: 'Allo', icon: icon('allo') },
  ],
  folders: [],
  windows: [
    {
      id: 'inbox-1',
      appId: 'inbox',
      title: 'Inbox',
      workspaceId: '1',
      thumbnail: require('../../shell-preview/assets/surfaces/inbox.png'),
      thumbnailSize: { width: 1600, height: 1066 },
    },
  ],
  workspaces: [
    { id: '1', name: 'Escritorio 1' },
    { id: '2', name: 'Escritorio 2' },
  ],
  workspaceId: '1',
  activeAppId: 'inbox',
  menus: [
    { label: 'OxyOS', items: [{ id: 'system.about', label: 'Acerca de OxyOS', enabled: false }] },
    {
      label: 'Ver',
      items: [
        { id: 'view.overview', label: 'Ver escritorios' },
        { id: 'view.launcher', label: 'Ver aplicaciones' },
      ],
    },
  ],
};

const actions: ShellActions = {
  activateApp: () => {},
  focusWindow: () => {},
  closeWindow: () => {},
  activateWorkspace: () => {},
  moveWindowToWorkspace: () => {},
  addWorkspace: () => {},
  menuCommand: () => {},
};

// The window probe's fixed time: its regressions compare what the top bar shows.
const PROBE_TIME = new Date(2026, 9, 29, 9, 41);

/** The shell, its regions where `platform` draws them, showing `snapshot` at `now`. */
function ProbeShell({
  platform,
  shown = snapshot,
  act = actions,
  now = PROBE_TIME,
  lock,
  wallpaper,
  dock,
}: {
  platform?: React.ComponentProps<typeof Shell>['platform'];
  /** Whether this display has the dock (the primary's); the window probe's has it. */
  dock?: boolean;
  shown?: ShellSnapshot;
  act?: ShellActions;
  now?: Date;
  lock?: ShellLock;
  /** The wallpaper the system chose (the solar wallpaper's frame now); the fixture's without. */
  wallpaper?: string;
}) {
  return (
      <SafeAreaProvider>
        <BloomProvider mode="light" colorPreset="graphite" haptics={false} locale="es" platform={oxyosSurfaces}>
          <Shell
            snapshot={shown}
            actions={act}
            wallpaper={wallpaper ? { uri: wallpaper } : require('../../../assets/wallpapers/solar-gradients/frames/frame-08.jpg')}
            now={now}
            lock={lock}
            platform={platform}
            dock={dock}
          />
        </BloomProvider>
      </SafeAreaProvider>
  );
}

/** In a window (`--window`): every region in place. */
export default function ShellProbe() {
  return <ProbeShell />;
}

/**
 * As the session's shell (`--shell`): the regions in panels and an overlay of their own, and the
 * compositor's windows and workspaces (ADR 0009) in place of the fixture's. The applications are
 * still the fixture's: the registry comes with the session's launch records.
 */
export function ShellSession() {
  const shell = useOxyOSShell();
  // The session's clock is the real one.
  const now = useMinuteClock();
  const state = shell.state;
  if (!state) return <ProbeShell platform={oxyosShell} now={now} />;
  // The dock is the primary display's (ADR 0018); none until the compositor has told this shell
  // its output, so that another display's never shows one for a moment.
  type Told = { id: number; primary: boolean };
  const dock = (state.outputs ?? []).some((output: Told) => output.id === state.output && output.primary);
  const windows = state.windows.map(
    (window: { id: number; appId: string; title: string; workspace: number; minimized: boolean; launched: boolean }) => ({
      id: String(window.id),
      appId: window.appId,
      title: window.title,
      workspaceId: String(window.workspace),
      minimized: window.minimized,
      launched: window.launched,
    }),
  );
  // The session's applications (ADR 0013): those kept in the dock by their place, then the others
  // by name, as the registry gives them. One without artwork is left out: the shell draws only an
  // application's own.
  type Registered = { id: string; name: string; icon: { uri: string } | { svg: string } | null; pinned: number | null };
  const registry: Registered[] = state.applications ?? [];
  const applications = registry
    .filter((app) => app.icon !== null)
    .map((app, order) => ({ app, order }))
    .sort((a, b) => (a.app.pinned ?? Infinity) - (b.app.pinned ?? Infinity) || a.order - b.order)
    .map(({ app }) => ({ id: app.id, name: app.name, icon: app.icon!, pinned: app.pinned !== null }));
  const shown: ShellSnapshot = {
    ...snapshot,
    applications,
    folders: [],
    windows,
    workspaces: Array.from({ length: state.workspaces }, (_, index) => ({
      id: String(index),
      name: `Escritorio ${index + 1}`,
    })),
    workspaceId: String(state.active),
    activeAppId: state.windows.find((window: { focused: boolean }) => window.focused)?.appId,
  };
  const act: ShellActions = {
    // The application's newest window, if it has one; else the session starts it (ADR 0013).
    activateApp: (id) => {
      const newest = state.windows
        .filter((window: { appId: string; launched: boolean }) => window.launched && window.appId === id)
        .sort((a: { id: number }, b: { id: number }) => b.id - a.id)[0];
      if (newest) shell.focusWindow(newest.id);
      else shell.launch(id);
    },
    focusWindow: (id) => shell.focusWindow(Number(id)),
    closeWindow: (id) => shell.closeWindow(Number(id)),
    activateWorkspace: (id) => shell.activateWorkspace(Number(id)),
    moveWindowToWorkspace: (window, workspace) => shell.moveWindowToWorkspace(Number(window), Number(workspace)),
    addWorkspace: () => shell.addWorkspace(),
    // The Oxy menu's System Settings opens OxyOS's Settings (docs/adr/0019-settings.mdx), as any
    // application is opened: its window if it has one, else the session starts it.
    menuCommand: (id) => {
      if (id === 'system.settings') act.activateApp('so.oxy.settings');
    },
  };
  // The session's lock (ADR 0010): the compositor says when it is locked, and only it unlocks.
  const lock: ShellLock = {
    locked: !!state.locked,
    user: state.user ?? { name: '' },
    capsLock: !!state.capsLock,
    lockSession: shell.lockSession,
    authenticate: shell.authenticate,
    answer: shell.authentication ?? undefined,
  };
  return (
    <ProbeShell
      platform={oxyosShell}
      shown={shown}
      act={act}
      now={now}
      lock={lock}
      wallpaper={state.wallpaper ?? undefined}
      dock={dock}
    />
  );
}
