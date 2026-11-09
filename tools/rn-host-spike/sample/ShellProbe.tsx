/**
 * The OxyOS shell (apps/shell) with a small fixture in place of the compositor's snapshot. In a
 * window of the Bloom host (`--window`) its regions are drawn in place; run as the session's
 * shell (`--shell`) they are panels and an overlay of their own (docs/adr/0009-session-shell.mdx).
 */
import * as React from 'react';
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { oxyosShell } from './oxyos/shell-surface';
import { useDisplayCovered, useOxyOSShell, useWindowSizes } from './oxyos/shell-state';
import {
  backup as backupApi,
  bluetooth,
  autofill as autofillApi,
  inputSources,
  power,
  session as sessionApi,
  sound as soundApi,
  useBackupState,
  useBluetoothState,
  useAutofillState,
  useInputSourcesState,
  usePowerState,
  useSoundState,
  useSystemSheets,
} from './oxyos/system';
import { network } from '@oxyos/platform';
import { useFollowed } from '@oxyos/platform/react';
import { readSheet } from '../../../apps/shell/src/sheet';
import { readAutofill } from '../../../apps/shell/src/autofill';
import { backupStateOf } from '../../../apps/shell/src/backup';
import { Shell } from '../../../apps/shell/src/Shell';
import { useMinuteClock } from '../../../apps/shell/src/useMinuteClock';
import { useShellMessages } from '../../../apps/shell/src/useShellMessages';
import type { ShellMessages } from '../../../apps/shell/src/messages';
import { useSessionLocale } from './oxyos/locale';
import { localSignInFrom, type LocalSignIn } from '../../../apps/shell/src/local-sign-in';
import { sessionUserFrom } from '../../../apps/shell/src/session-user';
import { useShellWallpaper } from './oxyos/wallpaper';
import type {
  ShellActions,
  ShellBackupMenu,
  ShellBatteryMenu,
  ShellEnergyApp,
  ShellInputMethod,
  ShellAutofill,
  ShellLock,
  ShellMenu,
  ShellMenuItem,
  ShellNotifications,
  ShellPowerActions,
  ShellScreenshots,
  ShellSnapshot,
} from '../../../apps/shell/src/model';

/** The clipboard (docs/adr/0007-clipboard.mdx): what a sheet copies, at the person's press. */
const clipboard = TurboModuleRegistry.get('OxyOSClipboard') as { setString(text: string): Promise<boolean> } | null;

/** The active window's menus as the compositor told them (OxyOSShell's `menuBar`). */
type MenuBar = {
  window: number;
  serial: number;
  items: {
    parent: number;
    kind: 'menu' | 'item' | 'submenu' | 'separator';
    enabled: boolean;
    checkable: boolean;
    checked: boolean;
    label: string;
    shortcut: string;
  }[];
};

// A chord as OxyOS shows it: `Mod` is Ctrl (docs/adr/0021-menu-bar.mdx), Shift named in the
// session's language, the rest as declared.
const chord = (shortcut: string, messages: ShellMessages) =>
  shortcut
    .split('+')
    .map((key) => (key === 'Mod' ? 'Ctrl' : key === 'Shift' ? messages.menus.shift : key))
    .join('+');

// A command of the window's menus, by the window, the commit and its index there.
const COMMAND = /^window-menu:(\d+):(\d+):(\d+)$/;

/**
 * The bar for the active window (ADR 0021): its application's own menu, in bold (the registry's
 * name, About, Hide, Quit), then the menus it declared, their checks and chords; none when it
 * declared none, and the shell then shows its own.
 */
function windowMenus(bar: MenuBar | undefined, name: string, messages: ShellMessages): ShellMenu[] | null {
  if (!bar || bar.window === 0 || bar.items.length === 0) return null;
  const children = (parent: number): ShellMenuItem[] =>
    bar.items.flatMap((item, index): ShellMenuItem[] => {
      if (item.parent !== parent) return [];
      const id = `window-menu:${bar.window}:${bar.serial}:${index}`;
      if (item.kind === 'separator') return [{ kind: 'separator', id }];
      if (item.kind === 'submenu') {
        return [{ kind: 'submenu', id, label: item.label, enabled: item.enabled, items: children(index) }];
      }
      return [
        {
          id,
          label: item.label,
          enabled: item.enabled,
          checked: item.checkable ? item.checked : undefined,
          shortcut: item.shortcut ? chord(item.shortcut, messages) : undefined,
        },
      ];
    });
  const own: ShellMenu = {
    label: name,
    items: [
      // No about panel yet: shown, not offered.
      { id: 'window-app:about', label: messages.menus.aboutApp(name), enabled: false },
      { kind: 'separator', id: 'window-app:separator' },
      { id: `window-app:hide:${bar.window}`, label: messages.menus.hideApp(name) },
      { id: `window-app:quit:${bar.window}`, label: messages.menus.quitApp(name) },
    ],
  };
  const declared = bar.items
    .map((item, index) => ({ item, index }))
    .filter(({ item }) => item.parent === -1 && item.kind === 'menu')
    .map(({ item, index }) => ({ label: item.label, items: children(index) }));
  return [own, ...declared];
}

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
  workspaces: [],
  workspaceId: '1',
  activeAppId: 'inbox',
  menus: [],
};

/** The shell's own menus, while no window with menus of its own is active (ADR 0021). */
function shellMenus(messages: ShellMessages): ShellMenu[] {
  return [
    { label: 'OxyOS', items: [{ id: 'system.about', label: messages.menus.about, enabled: false }] },
    {
      label: messages.menus.view,
      items: [
        { id: 'view.overview', label: messages.common.showDesktops },
        { id: 'view.launcher', label: messages.menus.showApplications },
      ],
    },
  ];
}

/** The workspaces, as many as there are, named in the session's language. */
function workspaceNames(count: number, messages: ShellMessages, first = 0): ShellSnapshot['workspaces'] {
  return Array.from({ length: count }, (_, index) => ({
    id: String(index + first),
    name: messages.overview.desktop(index + 1),
  }));
}

/** The fixture in the session's language: its menus and its desktops' names are the shell's. */
function useFixture(): ShellSnapshot {
  const messages = useShellMessages();
  return React.useMemo(
    () => ({ ...snapshot, workspaces: workspaceNames(2, messages, 1), menus: shellMenus(messages) }),
    [messages],
  );
}

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

/** The fixture's machine (docs/adr/0025-hardware-keys-and-control-center.mdx), where the shell
    runs without a session: a full battery and Wi-Fi at full signal. */
const probeControls: NonNullable<React.ComponentProps<typeof Shell>['controls']> = {
  levels: {},
  power: { battery: 'discharging', percent: 100, onAC: false, minutes: 0 },
  network: { wifi: 'connected', signal: 100, ssid: 'OxyOS', wired: false },
};

/**
 * OxyOS's platform as the shell has it: never the `menu-bar` capability. The shell's top bar is
 * the session's menu bar (docs/adr/0021-menu-bar.mdx): its Bloom Menubar draws itself, and never
 * hands its menus to a platform bar, which would leave its own bar empty. The session's shell is
 * offered none; in a window (`--window`) the host offers the window's, as to any application's.
 */
const { menuBar: _windowMenuBar, ...shellPlatformBase } = oxyosSurfaces;
const shellPlatform = {
  ...shellPlatformBase,
  capabilities: new Set([...oxyosSurfaces.capabilities].filter((capability) => capability !== 'menu-bar')),
};

/** What the shell is rendered in: the safe area and Bloom, in the session's locale
    (docs/adr/0040-language-and-region.mdx). Only a new locale renders it again, and everything
    in its language with it; a change the shell shows renders the shell, never these. */
function ShellProviders({ children }: { children: React.ReactNode }) {
  const locale = useSessionLocale();
  return (
    <SafeAreaProvider>
      <BloomProvider mode="system" colorPreset="graphite" haptics={false} locale={locale} platform={shellPlatform}>
        {children}
      </BloomProvider>
    </SafeAreaProvider>
  );
}

/** The shell state's `wallpaper` (oxyos/wallpaper.js). */
type ShellStateWallpaper = { key: string; uri: string | null; fit: string; color: string; slow: boolean };

/** The shell, its regions where `platform` draws them, showing `shown` at `now`. */
function ProbeShell({
  platform,
  shown,
  act = actions,
  now = PROBE_TIME,
  lock,
  wallpaper,
  dock,
  commands,
  controls = probeControls,
  notifications,
  sheet,
  screenshots,
  inputMethod,
  battery,
  backup,
  autofill,
  powerActions,
}: {
  /** AutoFill's list at a field's caret (docs/adr/0050-keychain-autofill-and-passkeys.mdx). */
  autofill?: ShellAutofill;
  /** Sleep, restart, shut down and log out (docs/adr/0064), asked of the session. */
  powerActions?: ShellPowerActions;
  /** Screenshots (ADR 0024, decision 10): the compositor's interface and the ones saved. */
  screenshots?: ShellScreenshots;
  /** The session's system sheet to show now (ADR 0030). */
  sheet?: React.ComponentProps<typeof Shell>['sheet'];
  platform?: React.ComponentProps<typeof Shell>['platform'];
  /** The machine as the session says it is (ADR 0025). */
  controls?: React.ComponentProps<typeof Shell>['controls'];
  /** Whether this display has the dock (the primary's); the window probe's has it. */
  dock?: boolean;
  shown?: ShellSnapshot;
  act?: ShellActions;
  now?: Date;
  lock?: ShellLock;
  /** What the wallpaper shows, as the host turned the session's choice into it
      (docs/adr/0048-wallpapers.mdx); the catalog's default without. */
  wallpaper?: ShellStateWallpaper | null;
  /** The system shortcuts the shell shows (ADR 0024), as the compositor sent them. */
  commands?: React.ComponentProps<typeof Shell>['commands'];
  /** The session's notifications (ADR 0026); banners only on the display with the dock. */
  notifications?: ShellNotifications;
  /** Input methods (ADR 0041): the candidate window and the top bar's input sources. */
  inputMethod?: ShellInputMethod;
  /** The battery's menu (ADR 0057), once the session told the battery and energy. */
  battery?: ShellBatteryMenu;
  /** The menu bar's backup item (ADR 0059), once the session told the backup service's state. */
  backup?: ShellBackupMenu;
}) {
  // The same wallpaper while it draws the same: the wallpaper and what blurs it render again only
  // when it changes.
  const source = useShellWallpaper(wallpaper);
  const fixture = useFixture();
  return (
    <Shell
      snapshot={shown ?? fixture}
      actions={act}
      wallpaper={source}
      now={now}
      lock={lock}
      platform={platform}
      dock={dock}
      banners={dock !== false}
      commands={commands}
      controls={controls}
      notifications={notifications}
      sheet={sheet}
      screenshots={screenshots}
      inputMethod={inputMethod}
      battery={battery}
      backup={backup}
      autofill={autofill}
      powerActions={powerActions}
    />
  );
}

/** In a window (`--window`): every region in place. */
/** The machine's Oxy session: who the session is for (its snapshot, followed: the lock screen and
    the menu bar name the person by it, docs/adr/0020-oxy-account-login.mdx decision 1) and the
    account's local sign-in (decision 2). */
const accountSession = TurboModuleRegistry.get<{ localSignIn(): Promise<unknown>; subscribe?(): boolean }>(
  'OxyOSAccountSession',
);

/** The session's snapshot as the accounts' service last told it (`OxyOSAccountSessionChanged`),
    followed from the shell's start on the connection the host keeps; null until told, and once the
    service went away. Asked without blocking: the subscription answers it at once. */
function useAccountSnapshot(): unknown {
  const [snapshot, setSnapshot] = React.useState<unknown>(null);
  React.useEffect(() => {
    if (!accountSession?.subscribe) return undefined;
    const subscription = DeviceEventEmitter.addListener('OxyOSAccountSessionChanged', (told: unknown) =>
      setSnapshot(told ?? null),
    );
    accountSession.subscribe();
    return () => subscription.remove();
  }, []);
  return snapshot;
}

export default function ShellProbe() {
  return (
    <ShellProviders>
      <ProbeShell />
    </ShellProviders>
  );
}

/** A window as the compositor told it (OxyOSShell's state). */
type ToldWindow = {
  id: number;
  appId: string;
  title: string;
  workspace: number;
  minimized: boolean;
  focused: boolean;
  launched: boolean;
  used: number;
  hidden: boolean;
  responding?: boolean;
};
type ToldLists = { dock: { chosen: boolean; ids: string[] } | null; openAtLogin: string[] | null };
type Registered = {
  id: string;
  name: string;
  icon: { uri: string } | { svg: string } | null;
  iconBody?: number | null;
  pinned: number | null;
  /** The media types its entry declares it opens (ADR 0034, decision 11). */
  opens?: string[];
};
type Controls = NonNullable<React.ComponentProps<typeof Shell>['controls']>;
type Level = { level: number; muted: boolean; available: boolean } | null;
type ToldControls = Record<'output' | 'input' | 'display' | 'keyboard', Level> & {
  power: Controls['power'] | null;
  network: Controls['network'] | null;
  bluetooth: Controls['bluetooth'] | null;
};
type ToldNotification = {
  id: number;
  appId: string;
  title: string;
  subtitle: string;
  body: string;
  actions: { label: string; reply: string | null }[];
  postedAt: number;
  banner: boolean;
  withheld: boolean;
};
type ToldOutput = { id: number; x: number; y: number; scale: number; primary: boolean };

// The shell's window for each told one, made once for each: the state the host shares
// (oxyos/shell-state.js) keeps a window unchanged as the same object, and so is the shell's.
const shellWindows = new WeakMap<ToldWindow, ShellSnapshot['windows'][number]>();
function shellWindow(window: ToldWindow): ShellSnapshot['windows'][number] {
  let shown = shellWindows.get(window);
  if (!shown) {
    shown = {
      id: String(window.id),
      appId: window.appId,
      title: window.title,
      workspaceId: String(window.workspace),
      minimized: window.minimized,
      launched: window.launched,
      // How recently it was active, and hidden with its application (ADR 0024).
      used: window.used,
      hidden: window.hidden,
      // The active window, and whether its process answers the compositor (ADR 0034).
      focused: window.focused,
      responding: window.responding !== false,
      // Its proportions, in which its overview card is drawn (ADR 0022), are the platform's
      // useWindowSizes, read by the overview alone.
    };
    shellWindows.set(window, shown);
  }
  return shown;
}

const shellNotifications = new WeakMap<ToldNotification, ShellNotifications['list'][number]>();
function shellNotification(told: ToldNotification): ShellNotifications['list'][number] {
  let shown = shellNotifications.get(told);
  if (!shown) {
    shown = {
      id: String(told.id),
      appId: told.appId,
      title: told.title,
      subtitle: told.subtitle || undefined,
      body: told.body,
      actions: told.actions.map((action) => ({ label: action.label, reply: action.reply ?? undefined })),
      postedAt: told.postedAt,
      banner: told.banner,
      withheld: told.withheld,
    };
    shellNotifications.set(told, shown);
  }
  return shown;
}

const NO_TOLD: never[] = [];
const NO_FOLDERS: ShellSnapshot['folders'] = [];

/**
 * As the session's shell (`--shell`): the regions in panels and an overlay of their own, and the
 * compositor's windows and workspaces (ADR 0009) in place of the fixture's. The applications are
 * still the fixture's: the registry comes with the session's launch records.
 *
 * Each part the shell is given is made from the parts of the state it reads, again only when they
 * change (the host's state is shared, oxyos/shell-state.js), and the actions are the same functions
 * throughout: a state that changed nothing renders nothing, and one that changed something renders
 * what shows it.
 */
export function ShellSession() {
  return (
    <ShellProviders>
      <SessionShell />
    </ShellProviders>
  );
}

function SessionShell() {
  const shell = useOxyOSShell();
  // The session's system sheets (ADR 0030): the first waiting, on the display with the dock.
  const {
    sheets,
    answer,
    answerPassword,
    answerCode,
    answerPasskey,
    resetApplication,
    choosePowerKey,
    reopenWindows,
  } = useSystemSheets();
  // The networks as the session tells them (ADR 0038): the Control Center's Wi-Fi view.
  const networks = useFollowed(network.wifi);
  // The sound's outputs as the session tells them (ADR 0056): the Control Center's Sound view.
  const sound = useSoundState();
  // Bluetooth as the session tells it (ADR 0055): the Control Center's Bluetooth module and view.
  const bluetoothDevices = useBluetoothState();
  // The input sources as the session tells them (ADR 0041): the top bar's item.
  const sources = useInputSourcesState();
  // The battery and energy as the session tells them (ADR 0057): the battery's menu, once it did.
  const powerState = usePowerState() as { showPercentage?: boolean; lowPower?: { on?: boolean } } | null;
  // The backup service's state as the session tells it (ADR 0059): the menu bar's backup item.
  const backupTold = useBackupState();
  // One the shell cannot read is refused: it never confirms what it did not show.
  React.useEffect(() => {
    for (const waiting of sheets) if (!readSheet(waiting.text)) answer(waiting.sheet, false);
  }, [sheets, answer]);
  // OxyOS's platform, with the session's polite announcements (a banner's, ADR 0026), the windows'
  // sizes (the overview's alone) and whether a zoomed window covers this display (the dock's
  // shelf's alone, ADR 0034).
  const announcing = React.useMemo(
    () => ({
      ...oxyosShell,
      announce: (text: string, polite: boolean) => shell.announce(text, polite),
      useWindowSizes,
      useDisplayCovered,
    }),
    [],
  );
  // The session's clock is the real one.
  const now = useMinuteClock();
  const messages = useShellMessages();
  const state = shell.state;
  const toldWindows: ToldWindow[] = state?.windows ?? NO_TOLD;
  const outputs: ToldOutput[] = state?.outputs ?? NO_TOLD;
  // The dock is the primary display's (ADR 0018); none until the compositor has told this shell
  // its output, so that another display's never shows one for a moment.
  const dock = outputs.some((output) => output.id === state?.output && output.primary);
  const windows = React.useMemo(() => toldWindows.map(shellWindow), [toldWindows]);
  // The session's applications (ADR 0013): those kept in the dock by their place, then the others
  // by name, as the registry gives them. One without artwork is left out: the shell draws only an
  // application's own. The dock's places are the person's once they changed them (ADR 0034), else
  // the registry's (each entry's X-OxyOS-Pinned).
  const registry: Registered[] = state?.applications ?? NO_TOLD;
  const lists: ToldLists | null = state?.applicationLists ?? null;
  const kept = lists?.dock?.chosen ? lists.dock.ids : null;
  const applications = React.useMemo(() => {
    const place = (app: Registered): number | null => {
      if (!kept) return app.pinned;
      const index = kept.indexOf(app.id);
      return index < 0 ? null : index + 1;
    };
    return registry
      .filter((app) => app.icon !== null)
      .map((app, order) => ({ app, order, place: place(app) }))
      .sort((a, b) => (a.place ?? Infinity) - (b.place ?? Infinity) || a.order - b.order)
      .map(({ app, place }) => ({
        id: app.id,
        name: app.name,
        icon: app.icon!,
        iconBody: app.iconBody ?? undefined,
        pinned: place !== null,
        opens: app.opens,
      }));
  }, [registry, kept]);
  // Who opens at login and what the Trash holds, once the session told them (ADR 0034).
  const openAtLogin = lists?.openAtLogin ?? undefined;
  const trashItems: number | null = state?.trashItems ?? null;
  // Safe Mode, as the session told it (docs/adr/0054-recovery-and-erasing.mdx, decision 10).
  const safeMode = state?.safeMode === true;
  const trash = React.useMemo(() => (trashItems === null ? undefined : { items: trashItems }), [trashItems]);
  const workspaceCount: number = state?.workspaces ?? 0;
  const workspaces = React.useMemo(() => workspaceNames(workspaceCount, messages), [workspaceCount, messages]);
  // The active window's menus, after its application's own (ADR 0021); else the shell's.
  const bar: MenuBar | undefined = state?.menuBar;
  const active = toldWindows.find((window) => window.id === bar?.window);
  const application = active?.launched ? registry.find((app) => app.id === active.appId) : undefined;
  const menuName = application?.name || active?.title || active?.appId || '';
  const menus = React.useMemo(
    () => windowMenus(bar, menuName, messages) ?? shellMenus(messages),
    [bar, menuName, messages],
  );
  const activeAppId = toldWindows.find((window) => window.focused)?.appId;
  const workspaceId = String(state?.active ?? 0);
  const shown: ShellSnapshot = React.useMemo(
    () => ({
      ...snapshot,
      applications,
      folders: NO_FOLDERS,
      windows,
      workspaces,
      workspaceId,
      activeAppId,
      menus,
      openAtLogin,
      trash,
      safeMode,
    }),
    [applications, windows, workspaces, workspaceId, activeAppId, menus, openAtLogin, trash, safeMode],
  );
  // The actions read the state when they act, so they are made once (and again once the session
  // told the person's lists, which a session of an older protocol never does).
  const listsTold = lists !== null;
  const latest = React.useRef({ toldWindows, bar });
  latest.current = { toldWindows, bar };
  const act: ShellActions = React.useMemo(() => {
    // An application's window it used last (the newest of those never active), if it has one.
    const recent = (id: string): ToldWindow | undefined =>
      latest.current.toldWindows
        .filter((window) => window.launched && window.appId === id)
        .sort((a, b) => b.used - a.used || b.id - a.id)[0];
    const actions: ShellActions = {
      // Its window used last, shown again (restored, or with its application if hidden, ADR 0024);
      // else the session starts it (ADR 0013).
      activateApp: (id) => {
        const window = recent(id);
        if (window) shell.focusWindow(window.id);
        else shell.launch(id);
      },
      // The application switcher's H and Q (ADR 0024): the compositor hides or quits the
      // application of any of its windows.
      hideApp: (id) => {
        const window = recent(id);
        if (window) shell.hideApplication(window.id);
      },
      quitApp: (id) => {
        const window = recent(id);
        if (window) shell.quitApplication(window.id);
      },
      // The dock's application menu (ADR 0034), where the session speaks it (it told the lists):
      // the session ends a force-quit application's processes; the lists are the person's, kept by
      // the session; the Trash is shown in the file manager, Dropsea, at its Trash, and emptied by the
      // session once the person accepts its sheet.
      forceQuitApp: (id) => {
        const window = recent(id);
        if (window) shell.forceQuitApplication(window.id);
      },
      setDock: listsTold ? (ids) => shell.setApplicationList('dock', ids) : undefined,
      setOpenAtLogin: listsTold ? (ids) => shell.setApplicationList('openAtLogin', ids) : undefined,
      openTrash: () => {
        shell.openUrl('dropsea://trash');
      },
      // A desktop folder opened in Dropsea (ADR 0047), citing the press or key that opened it.
      openUrl: (url) => {
        shell.openUrl(url);
      },
      emptyTrash: () => {
        shell.emptyTrash();
      },
      // Files dropped on the dock's Trash (ADR 0035): put in it by the session, citing the drop.
      trashFiles: (uris) => {
        shell.trashFiles(uris);
      },
      // Files dropped on an application's dock icon (ADR 0034, decision 11): opened in it by the
      // session, citing the drop, where the session can.
      openFiles: shell.openFiles
        ? (id, uris) => {
            shell.openFiles(id, uris);
          }
        : undefined,
      focusWindow: (id) => shell.focusWindow(Number(id)),
      // The dock's press on the frontmost application (ADR 0034, decision 9): its windows into its
      // icon through the genie (ADR 0043).
      minimizeWindow: (id) => shell.minimizeWindow(Number(id)),
      // The top bar left from the keyboard (ADR 0024): the keyboard back with the window whose
      // menus the bar shows, the active one.
      returnKeyboard: () => {
        const window = latest.current.bar?.window;
        if (window) return shell.focusWindow(window);
        // None active (the one that was quit from the dock's menu while the dock had the keyboard,
        // docs/adr/0039-keyboard-access.mdx): the window used last that is shown, as a Mac makes
        // the next application active when the frontmost one quits.
        const next = latest.current.toldWindows
          .filter((told) => !told.minimized && !told.hidden)
          .sort((a, b) => b.used - a.used || b.id - a.id)[0];
        if (next) shell.focusWindow(next.id);
      },
      closeWindow: (id) => shell.closeWindow(Number(id)),
      activateWorkspace: (id) => shell.activateWorkspace(Number(id)),
      moveWindowToWorkspace: (window, workspace) => shell.moveWindowToWorkspace(Number(window), Number(workspace)),
      addWorkspace: () => shell.addWorkspace(),
      // The Oxy menu's System Settings opens OxyOS's Settings (docs/adr/0019-settings.mdx), as any
      // application is opened: its window if it has one, else the session starts it.
      menuCommand: (id) => {
        if (id === 'system.settings') {
          actions.activateApp('so.oxy.settings');
          return;
        }
        const command = COMMAND.exec(id);
        if (command) {
          shell.activateMenuItem(Number(command[1]), Number(command[2]), Number(command[3]));
          return;
        }
        // The application's own menu: Hide hides the application, Quit asks each of its windows
        // to close, as Super+H and Super+Q do (ADR 0024).
        const own = /^window-app:(hide|quit):(\d+)$/.exec(id);
        if (own?.[1] === 'hide') shell.hideApplication(Number(own[2]));
        if (own?.[1] === 'quit') shell.quitApplication(Number(own[2]));
      },
    };
    return actions;
  }, [listsTold]);
  // The session's lock (ADR 0010): the compositor says when it is locked, and only it unlocks.
  const locked = !!state?.locked;
  // Whom the session is for: the account's display name and username, else the full name the
  // system keeps (passwd's GECOS, written from the same profile); never the Unix user's name.
  const accountSnapshot = useAccountSnapshot();
  const systemFullName = state?.user?.fullName;
  const user = React.useMemo(
    () => sessionUserFrom(accountSnapshot, systemFullName),
    [accountSnapshot, systemFullName],
  );
  const capsLock = !!state?.capsLock;
  // The account's PIN (ADR 0020 decision 2), asked of the accounts' service as the session locks
  // and after each answer (a PIN locked out on the way): which field the lock screen shows. Not
  // read, it shows the password's, as before.
  const [local, setLocal] = React.useState<LocalSignIn>({ pin: undefined, password: true });
  const answerId = shell.authentication?.id;
  React.useEffect(() => {
    if (!locked || !accountSession) return undefined;
    let current = true;
    accountSession
      .localSignIn()
      .then((told) => current && setLocal(localSignInFrom(told)))
      .catch(() => current && setLocal({ pin: undefined, password: true }));
    return () => {
      current = false;
    };
  }, [locked, answerId]);
  const lock: ShellLock = React.useMemo(
    () => ({
      locked,
      user,
      capsLock,
      lockSession: shell.lockSession,
      authenticate: shell.authenticate,
      pin: local.pin,
      password: local.password,
      authenticatePin: (pin: string) => shell.authenticatePin(pin),
      answer: shell.authentication ?? undefined,
    }),
    [locked, user, capsLock, shell.authentication, local],
  );
  // The machine as the session says it is, and the Control Center's intents (ADR 0025): only
  // what it told is shown.
  const told: ToldControls | null = state?.controls ?? null;
  const bezel = shell.bezels[shell.bezels.length - 1];
  const controls: Controls = React.useMemo(() => {
    const levels: Controls['levels'] = {};
    for (const control of ['output', 'input', 'display', 'keyboard'] as const) {
      const level = told?.[control];
      if (level) levels[control] = level;
    }
    return {
      levels,
      power: told?.power ?? undefined,
      network: told?.network ?? undefined,
      bluetooth: told?.bluetooth ?? undefined,
      bezel,
      setLevel: shell.setControlLevel,
      setMuted: shell.setControlMuted,
      setWifi: shell.setWifiEnabled,
      setDark: shell.setAppearance,
      refresh: shell.refreshControls,
      // Wi-Fi's networks (ADR 0038): the session scans and joins, asking a password or saying what
      // failed in its own sheets. Settings has no page of its own to open at yet, so Wi-Fi
      // Settings opens Settings as the Oxy menu does.
      networks,
      // A join's failure is said in the session's own sheet; a refusal by the state it tells.
      scanNetworks: () => {
        network.scan().catch(() => {});
      },
      joinNetwork: (ssid) => {
        network.join(ssid).catch(() => {});
      },
      openWifiSettings: () => act.menuCommand('system.settings'),
      // The outputs (ADR 0056): the session plays to the one chosen. Sound Settings opens
      // Settings as Wi-Fi Settings does.
      sound,
      selectOutput: (id) => {
        soundApi.select('output', id);
      },
      openSoundSettings: () => act.menuCommand('system.settings'),
      // Bluetooth (ADR 0055): the session turns it on and off, connects and disconnects, asking
      // what a pairing needs and saying what failed in its own sheets. Bluetooth Settings opens
      // Settings at its Bluetooth page, a URL the shell opens as any other.
      bluetoothDevices,
      setBluetooth: (on) => {
        bluetooth.setPower(on);
      },
      connectBluetooth: (id) => {
        bluetooth.connect(id);
      },
      disconnectBluetooth: (id) => {
        bluetooth.disconnect(id);
      },
      openBluetoothSettings: () => {
        shell.openUrl('oxyos-settings://bluetooth');
      },
    };
  }, [told, bezel, networks, sound, bluetoothDevices, act]);
  // The session's notifications (ADR 0026): the compositor keeps them; the shell shows them and
  // says what the person did.
  const toldNotifications: ToldNotification[] = state?.notifications ?? NO_TOLD;
  const doNotDisturb = !!state?.notificationPolicy?.doNotDisturb;
  const previews = !!state?.notificationPolicy?.previews;
  const notifications: ShellNotifications = React.useMemo(
    () => ({
      list: toldNotifications.map(shellNotification),
      doNotDisturb,
      previews,
      activate: (id, action, reply) => shell.activateNotification(Number(id), action, reply),
      remove: (id) => shell.removeNotification(Number(id)),
      setDoNotDisturb: shell.setDoNotDisturb,
    }),
    [toldNotifications, doNotDisturb, previews],
  );
  // Screenshots (ADR 0024, decision 10): this display's place and scale, the interface the
  // compositor opened, the window under the pointer and the screenshots saved.
  const here = outputs.find((output) => output.id === state?.output);
  const outputId: number = state?.output ?? 0;
  const screenshots: ShellScreenshots = React.useMemo(
    () => ({
      output: { id: outputId, x: here?.x ?? 0, y: here?.y ?? 0, scale: here?.scale ?? 1 },
      active: shell.capture,
      target: shell.captureTarget,
      saved: shell.screenshots,
      capture: (kind, output, rect, window, destination) => {
        shell.captureScreen(kind, output, rect, window, destination);
      },
      end: () => {
        shell.endCapture();
      },
      open: (id) => {
        shell.openScreenshot(id);
      },
    }),
    [outputId, here, shell.capture, shell.captureTarget, shell.screenshots],
  );
  // Input methods (ADR 0041): the candidates an input method handed the system, drawn at the caret
  // by the shell of the caret's display (the compositor tells only that one); the input sources,
  // which the session switches. Keyboard settings open Settings as the Oxy menu does: it has no
  // page of its own to open at yet.
  const inputMethod: ShellInputMethod = React.useMemo(
    () => ({
      candidates: shell.candidates,
      choose: (index) => {
        shell.chooseInputCandidate(index);
      },
      turnPage: (forward) => {
        shell.turnInputCandidatesPage(forward);
      },
      sources,
      selectSource: (id) => {
        inputSources.select(id);
      },
      showEmoji: () => {
        inputSources.showEmoji();
      },
      openKeyboardSettings: () => act.menuCommand('system.settings'),
    }),
    [shell.candidates, sources, act],
  );
  // The battery's menu (ADR 0057): its percentage and Low Power Mode as the session says them, the
  // applications' energy sampled as it opens, and Battery Settings… opening Settings at its page.
  const showPercentage = !!powerState?.showPercentage;
  const lowPower = !!powerState?.lowPower?.on;
  const battery: ShellBatteryMenu | undefined = React.useMemo(
    () =>
      powerState
        ? {
            showPercentage,
            lowPower,
            energy: async () => {
              const answer = (await power.energy()) as { energy?: { apps?: ShellEnergyApp[] } };
              return answer?.energy?.apps ?? [];
            },
            openSettings: () => {
              shell.openUrl('oxyos-settings://battery');
            },
          }
        : undefined,
    [!!powerState, showPercentage, lowPower, shell.openUrl],
  );

  // The menu bar's backup item (ADR 0059): the state as the session tells it, read strictly; Back
  // Up Now and Skip This Backup asked of the session citing the press, Browse Backups opening
  // Dropsea's, Open Backups Settings… Settings at its Backups page.
  const backupMenu: ShellBackupMenu | undefined = React.useMemo(() => {
    const state = backupStateOf((backupTold as { backup?: unknown } | null)?.backup);
    return state
      ? {
          state,
          backUpNow: () => {
            backupApi.backUpNow();
          },
          skip: () => {
            backupApi.stop();
          },
          browse: () => {
            shell.openUrl('dropsea://backups');
          },
          openSettings: () => {
            shell.openUrl('oxyos-settings://backups');
          },
        }
      : undefined;
  }, [backupTold, shell.openUrl]);

  // AutoFill (docs/adr/0050-keychain-autofill-and-passkeys.mdx): the list the session tells this
  // shell to draw at a field's caret (the shell of the caret's display), and the press that picks.
  const autofillTold = useAutofillState();
  const autofill: ShellAutofill = React.useMemo(
    () => ({
      list: readAutofill(autofillTold),
      choose: (index) => {
        autofillApi.choose(index);
      },
    }),
    [autofillTold],
  );
  // Sleep, restart, shut down and log out (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx):
  // asked of the session, citing the menu's press; it decides, and asks its own question.
  const powerActions: ShellPowerActions = React.useMemo(
    () => ({
      sleep: () => void sessionApi.sleep(),
      endSession: (action, confirm) => void sessionApi.endSession(action, confirm),
    }),
    [],
  );
  const first = dock ? sheets[0] : undefined;
  const sheet = React.useMemo(() => {
    const firstText = first ? readSheet(first.text) : null;
    if (!first || !firstText) return undefined;
    const names = Object.fromEntries(registry.map((app) => [app.id, app.name]));
    return {
      id: first.sheet,
      text: firstText,
      names,
      answer: (accepted: boolean) => answer(first.sheet, accepted),
      // A Wi-Fi password sheet's answer (ADR 0038): to the session alone.
      answerPassword: (password: string) => answerPassword(first.sheet, password),
      // A Bluetooth pairing code's answer (ADR 0055): to the session alone.
      answerCode: (code: string) => answerCode(first.sheet, code),
      // A passkey sheet's answer (ADR 0050): to the session alone.
      answerPasskey: (choice: number | undefined, password: string) => answerPasskey(first.sheet, choice, password),
      // An application that quit (docs/adr/0061): Reset… asks the session to reset it.
      act: (action: string) => {
        if (action === 'reset' && firstText.kind === 'crash') resetApplication(first.sheet, firstText.id);
        // The power key's Restart and Sleep (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx).
        if ((action === 'restart' || action === 'sleep') && firstText.kind === 'power-key') {
          choosePowerKey(first.sheet, action);
        }
      },
      // Reopen windows when logging back in, toggled: the session keeps it.
      toggle: reopenWindows,
      // Its report copied, at the person's press, as any application copies (ADR 0007).
      copy: (text: string) => clipboard?.setString(text) ?? false,
    };
  }, [first, registry, answer, answerPassword, answerCode, answerPasskey, resetApplication, choosePowerKey, reopenWindows]);
  if (!state) return <ProbeShell platform={oxyosShell} now={now} />;
  return (
    <ProbeShell
      sheet={sheet}
      platform={announcing}
      shown={shown}
      act={act}
      now={now}
      lock={lock}
      wallpaper={state.wallpaper}
      dock={dock}
      commands={shell.shortcuts}
      controls={controls}
      notifications={notifications}
      screenshots={screenshots}
      inputMethod={inputMethod}
      battery={battery}
      backup={backupMenu}
      autofill={autofill}
      powerActions={powerActions}
    />
  );
}
