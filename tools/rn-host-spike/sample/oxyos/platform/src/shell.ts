// `shell` (docs/adr/0058-the-platform-api.mdx, decision 1): what only the session's shell does, with
// its `shell` grant (the session refuses anyone else). Over the one connection to the session: the
// system's sheets the shell draws and answers (ADR 0030's consent and permission sheets, ADR 0038's
// Wi-Fi password, ADR 0055's pairing code), AutoFill's list at a field's caret (ADR 0050, decision
// 6) and the emoji picker (ADR 0041). The person's answers go to the session alone; the
// application a sheet is about never sees them. And over the host's OxyOSShell, which only a host
// run as the shell has (docs/adr/0009-session-shell.mdx): the compositor's windows, workspaces,
// outputs, controls, notifications and lock as the shell knows them (`state`, followed), what the
// compositor tells it once (`on<Event>`), and its intents, whose outcome the next state says.
import { errorOf, PlatformError } from './errors.ts';
import { followed, type Followed } from './followed.ts';
import type { CallOptions } from './frameworks.ts';
import type { Events, Json, Session } from './session.ts';

/** The host's OxyOSShell: a host run as the shell has it; any other's calls throw. Its intents are
    the protocol's, by name (`focusWindow(window)`, `setApplicationList(list, ids)`, ...). */
export interface ShellModule {
  getState(): Json;
  [intent: string]: (...args: any[]) => any;
}

/** What the compositor tells the shell once, by the platform's name for it and the host's event. */
const SHELL_EVENTS = {
  authentication: 'OxyOSShellAuthentication',
  shortcut: 'OxyOSShellShortcut',
  bezel: 'OxyOSShellBezel',
  capture: 'OxyOSShellCapture',
  captureTarget: 'OxyOSShellCaptureTarget',
  screenshot: 'OxyOSShellScreenshot',
  candidates: 'OxyOSShellCandidates',
  launch: 'OxyOSShellLaunch',
} as const;
export type ShellEvent = keyof typeof SHELL_EVENTS;

/** The shell's intents on the compositor and the session, each the host's OxyOSShell method of the
    same name (docs/adr/0009-session-shell.mdx and the ADRs each names): fire and forget, the next
    state saying what came of it; where it answers, whether it was sent (or a serial, 0: not sent). */
export interface ShellIntents {
  /** Focuses the window `window`. */
  focusWindow(window: number): void;
  /** Closes the window `window`. */
  closeWindow(window: number): void;
  /** Minimizes the window `window`. */
  minimizeWindow(window: number): void;
  /** Hides the application of the window `window` (ADR 0024). */
  hideApplication(window: number): void;
  /** Quits the application of the window `window`. */
  quitApplication(window: number): void;
  /** Hides every application but the one of the window `window` (0: every one), as a Mac's
      Hide Others (docs/adr/0021-menu-bar.mdx). */
  hideOtherApplications(window: number): void;
  /** Shows every hidden application again, the active window staying the active one, as a
      Mac's Show All. */
  showAllApplications(): void;
  /** Has the session end the application of the window `window` (ADR 0034). */
  forceQuitApplication(window: number): boolean;
  /** Moves the window `window` to the workspace `workspace`. */
  moveWindowToWorkspace(window: number, workspace: number): void;
  /** Shows the workspace `workspace`. */
  activateWorkspace(workspace: number): void;
  /** Adds a workspace. */
  addWorkspace(): void;
  /** Removes the workspace `workspace`. */
  removeWorkspace(workspace: number): void;
  /** Replaces the person's dock or Open at Login list (ADR 0034). */
  setApplicationList(list: 'dock' | 'openAtLogin', ids: readonly string[]): boolean;
  /** Opens a URL in the application that opens its scheme, citing the latest input: its serial, 0
      if not sent; `on('launch')` says whether it was opened. */
  openUrl(url: string): number;
  /** Asks the session to empty the Trash, which asks the person first (ADR 0034). */
  emptyTrash(): boolean;
  /** Has the session put the files in the Trash, citing the drop on the dock's Trash (ADR 0035). */
  trashFiles(uris: readonly string[]): boolean;
  /** Has the session open the files with `application`, citing the drop on its icon (ADR 0034). */
  openFiles(application: string, uris: readonly string[]): boolean;
  /** Asks for the keyboard for the shell's desktop, citing the press on it (ADR 0047). */
  focusDesktop(): boolean;
  /** Locks the session (ADR 0010). */
  lockSession(): void;
  /** Starts an application of the registry (ADR 0013): its serial, 0 if not sent; `on('launch')`
      says whether it was started. */
  launch(id: string): number;
  /** Sends the password (or, with `passcode`, the account's PIN for this computer) typed in the
      lock or login screen, for `account` ("": the session's own user), ADR 0020. */
  authenticate(secret: string, account: string, options?: { passcode?: boolean }): boolean;
  /** The accounts enrolled on this computer (ADR 0020), null when they cannot be read. */
  accounts(): unknown;
  /** Chooses the entry `index` of the commit `serial` of window `window`'s menus (ADR 0021). */
  activateMenuItem(window: number, serial: number, index: number): void;
  /** Sets a control's level, 0 to 1, from the Control Center (ADR 0025): the display's and the
      keyboard's brightness, and the sound's volume until it moves to `audio` (decision 7). */
  setControlLevel(control: string, level: number): void;
  /** Mutes or unmutes a control from the Control Center: the sound's, until it moves to `audio`
      (decision 7). */
  setControlMuted(control: string, muted: boolean): void;
  /** Turns Wi-Fi on or off from the Control Center, until it moves to `network.setWifi`
      (decision 7). */
  setWifiEnabled(enabled: boolean): void;
  /** Sets the person's appearance, light or dark. */
  setAppearance(dark: boolean): void;
  /** Has the session look at the machine's controls again now. */
  refreshControls(): void;
  /** Opens a notification (`action` 0) or runs its action, with the reply typed (ADR 0026). */
  activateNotification(notification: number, action: number, reply: string): void;
  /** Removes a notification (0: all). */
  removeNotification(notification: number): void;
  /** Turns Do Not Disturb on or off from the Control Center. */
  setDoNotDisturb(enabled: boolean): void;
  /** Says `text` to assistive technologies, politely or at once. */
  announce(text: string, polite: boolean): void;
  /** Captures what the person chose in the screenshot interface `serial` (ADR 0024, decision 10). */
  capture(
    serial: number,
    kind: 'screens' | 'area' | 'window',
    output: number,
    rect: { x: number; y: number; width: number; height: number } | null,
    window: number,
    destination: 'desktop' | 'documents',
  ): boolean;
  /** Ends the screenshot interface `serial` without a capture. */
  endCapture(serial: number): boolean;
  /** Opens a saved screenshot, citing the press on its thumbnail. */
  openScreenshot(id: number): boolean;
  /** Chooses a candidate of the input method's page (ADR 0041). */
  chooseInputCandidate(index: number): boolean;
  /** Turns the input method's page of candidates forward or back. */
  turnInputCandidatesPage(forward: boolean): boolean;
}

const INTENTS: readonly (keyof ShellIntents)[] = [
  'focusWindow',
  'closeWindow',
  'minimizeWindow',
  'hideApplication',
  'quitApplication',
  'hideOtherApplications',
  'showAllApplications',
  'forceQuitApplication',
  'moveWindowToWorkspace',
  'activateWorkspace',
  'addWorkspace',
  'removeWorkspace',
  'setApplicationList',
  'openUrl',
  'emptyTrash',
  'trashFiles',
  'openFiles',
  'focusDesktop',
  'lockSession',
  'launch',
  'authenticate',
  'accounts',
  'activateMenuItem',
  'setControlLevel',
  'setControlMuted',
  'setWifiEnabled',
  'setAppearance',
  'refreshControls',
  'activateNotification',
  'removeNotification',
  'setDoNotDisturb',
  'announce',
  'capture',
  'endCapture',
  'openScreenshot',
  'chooseInputCandidate',
  'turnInputCandidatesPage',
];

/** A system sheet waiting for the person: its number, and what it asks (the session's JSON). */
export interface SystemSheet {
  readonly sheet: number;
  readonly text: string;
}

export function createShell(
  session: Session,
  answerSheet: ((sheet: number, accepted: boolean) => void) | null,
  events: Events,
  native: ShellModule | null | undefined = null,
) {
  const { available, state } = session;
  // Whether this host is the shell: OxyOSShell answers its state only there.
  const read = (): Json | null => {
    try {
      return native ? native.getState() : null;
    } catch {
      return null;
    }
  };
  const first = read();
  const isShell = first !== null;
  const shellState = followed<Json>();
  if (isShell) {
    shellState.tell(first);
    events.addListener('OxyOSShellChanged', (next: Json) => shellState.tell(next));
  }
  // Whether the shell's desktop, its background, has the keyboard (docs/adr/0047-finder-drags-and-
  // the-desktop.mdx).
  const desktopKeyboard = followed<boolean>();
  if (isShell) {
    desktopKeyboard.tell(false);
    events.addListener('OxyOSShellDesktopKeyboard', ({ focused }: { focused?: boolean }) => {
      if (Boolean(focused) !== desktopKeyboard.current()) desktopKeyboard.tell(Boolean(focused));
    });
  }
  // The shell's intents: the same functions always, so what is given them is never new for them.
  const intents = Object.fromEntries(
    INTENTS.map((intent) => [intent, (...args: unknown[]) => (isShell ? native?.[intent]?.(...args) : undefined)]),
  ) as unknown as ShellIntents;
  // The sheets waiting, in the order the session showed them, until answered or dismissed.
  const sheets = followed<readonly SystemSheet[]>();
  const waiting = () => sheets.current() ?? [];
  if (available) {
    sheets.tell([]);
    events.addListener('OxyOSSheet', ({ sheet, text }: SystemSheet) => {
      if (!waiting().some((shown) => shown.sheet === sheet)) sheets.tell([...waiting(), { sheet, text }]);
    });
    events.addListener('OxyOSSheetDismissed', ({ sheet }: { sheet: number }) => gone(sheet));
  }
  const gone = (sheet: number) => {
    if (waiting().some((shown) => shown.sheet === sheet)) sheets.tell(waiting().filter((shown) => shown.sheet !== sheet));
  };
  /** Sends one of the person's answers to the session, citing the press or key that gave it. */
  const tell = (body: Json, sheet?: number) => {
    if (!available) throw new PlatformError('shell', 'unavailable');
    if (!session.tell(body)) throw new PlatformError('shell', 'unavailable', 'notSent');
    if (sheet !== undefined) gone(sheet);
  };

  /** A request the session answers: resolved once it did, rejected with its refusal. */
  const ask = async (body: Json, options?: CallOptions): Promise<void> => {
    const error = errorOf('shell', await session.request(body, options));
    if (error) throw error;
  };

  return {
    available,
    /** Whether this host is the session's shell (only it has the state and the intents). */
    isShell,
    /** The compositor's windows, workspaces, outputs, controls, notifications and lock as the
        shell knows them, followed (shell-state.js gives its parts to the shell's views). */
    state: shellState as Followed<Json>,
    /** Whether the shell's desktop has the keyboard, followed. */
    desktopKeyboard: desktopKeyboard as Followed<boolean>,
    /** Tells `listener` each `event` the compositor tells the shell once (a shortcut, a bezel, a
        capture's step, a screenshot, the lock's answer, a launch's outcome, ...); the function
        returned stops. */
    on(event: ShellEvent, listener: (told: any) => void): () => void {
      if (!isShell) return () => {};
      const subscription = events.addListener(SHELL_EVENTS[event], listener);
      return () => subscription.remove();
    },
    /** The shell's intents on the compositor and the session; the next state says what came of
        each. Nothing is sent from a host that is not the shell. */
    intents,
    /** The system's sheets waiting for the person, in order. */
    sheets: sheets as Followed<readonly SystemSheet[]>,
    /** The person's answer to a sheet: accepted or not. */
    answerSheet(sheet: number, accepted: boolean): void {
      if (!available || !answerSheet) throw new PlatformError('shell', 'unavailable');
      answerSheet(sheet, accepted);
      gone(sheet);
    },
    /** A Wi-Fi password sheet's answer: the password the person typed, to the session alone. */
    answerPassword: (sheet: number, password: string) => tell({ password: { sheet, password } }, sheet),
    /** A Bluetooth pairing sheet's answer: the PIN or passkey the person typed, to the session
        alone. */
    answerCode: (sheet: number, code: string) => tell({ bluetoothCode: { sheet, code } }, sheet),
    /** AutoFill's list the session tells this shell to draw at a field's caret, null for none. The
        application whose field it is never sees it. */
    autofill: state.autofill as Followed<Json | null>,
    /** Picks the AutoFill item `index`, citing the press that chose it. */
    chooseAutofill: (index: number, options?: CallOptions) => ask({ chooseAutofill: { index } }, options),
    /** Shows the emoji picker, citing the press or key that asked. */
    showEmojiPicker: (options?: CallOptions) => ask({ showEmoji: {} }, options),
    /** A passkey sheet's answer (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7): the
        passkey chosen and the Oxy account's password the person typed, to the session alone, which
        hands them to the accounts' service, never to the application. */
    answerPasskey: (sheet: number, choice: number | undefined, password: string) =>
      tell({ passkey: choice === undefined ? { sheet, password } : { sheet, choice, password } }, sheet),
    /** An application that quit (docs/adr/0061-crashes-and-resetting-an-application.mdx): its
        notice's Reset… answers the notice no and asks the session to reset it, which asks the
        person in a sheet of its own and opens it again once it is reset. */
    resetApplication(sheet: number, id: string): void {
      if (!available || !answerSheet) throw new PlatformError('shell', 'unavailable');
      answerSheet(sheet, false);
      gone(sheet);
      void session.request({ resetApplication: { id, reopen: true } });
    },
    /** The power key's sheet (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx, decision 7):
        its Restart and Sleep answer it, the session taking them only for the sheet it shows. */
    choosePowerKey: (sheet: number, choice: string) => tell({ powerKeyChoice: { sheet, choice } }, sheet),
    /** Reopen windows when logging back in, as the person toggled it in the session's question:
        their `reopen_windows`, which the session keeps. */
    setReopenWindows: (enabled: boolean) => tell({ reopenWindows: { enabled } }),
    /**
     * Sleep, restart, shut down and log out (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx):
     * the shell's Oxy menu and the login screen's buttons. The session decides: it asks its own
     * question (`confirm`), every application, then logind. Resolves once it answered
     * (`{confirming}`, `{ending: {action}}`), rejects with its refusal.
     */
    endSession: (action: string, confirm: boolean, options?: CallOptions) =>
      ask({ endSession: { action, confirm } }, options),
    /** Sleeps the computer at once (`{sleeping}`). */
    sleep: (options?: CallOptions) => ask({ sleep: {} }, options),
  };
}

export type Shell = ReturnType<typeof createShell>;
