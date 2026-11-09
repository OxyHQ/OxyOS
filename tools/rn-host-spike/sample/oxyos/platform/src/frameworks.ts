// The frameworks over the session (docs/adr/0058-the-platform-api.mdx, decision 1): each one
// domain, its functions named as the ADR names them and mapped here to the session's existing
// vocabulary (decision 3: the wire keeps its names until the session changes them in its own step),
// its state followed, its calls rejecting with a PlatformError and cancellable with a signal.
import { errorOf, PlatformError, type PlatformDomain } from './errors.ts';
import type { Followed } from './followed.ts';
import type { Json, RequestOptions, Session } from './session.ts';

/** What every asynchronous call takes. */
export interface CallOptions {
  /** Aborting rejects the call with `cancelled` at once; the session's answer is dropped. */
  signal?: AbortSignal;
}
/** A call that tells its progress (an install). */
export interface ProgressOptions extends CallOptions {
  onProgress?: (progress: unknown) => void;
}

/** A session request of `domain`: its answer, or a rejection with the PlatformError it is. */
function caller(session: Session, domain: PlatformDomain) {
  return async (body: Json, options?: RequestOptions): Promise<Json> => {
    const answer = await session.request(body, options);
    const error = errorOf(domain, answer);
    if (error) throw error;
    return answer as Json;
  };
}

export type Direction = 'output' | 'input';
/** What Activity Monitor shows: each view reads what its columns need. */
export type ActivityView = 'cpu' | 'memory' | 'energy' | 'disk' | 'network';

export function createFrameworks(session: Session) {
  const call = (domain: PlatformDomain) => caller(session, domain);
  const net = call('network');
  const bt = call('bluetooth');
  const pw = call('power');
  const au = call('audio');
  const st = call('settings');
  const sto = call('store');
  const { available, state } = session;

  /** Wi-Fi and networks (docs/adr/0038-networks.mdx): the `network` grant, or `shell`. */
  const network = {
    available,
    /** Wi-Fi and the networks nearby, as NetworkManager has them (through the session). */
    wifi: state.network as Followed<Json>,
    /** Asks for the networks nearby; they arrive in `wifi`. */
    scan: (options?: CallOptions) => net({ scan: {} }, options),
    /** Joins `ssid`; a password, if it needs one, is asked in the system's sheet. */
    join: (ssid: string, options?: CallOptions) => net({ join: { ssid } }, options),
    /** Forgets the known network `id`: OxyOS no longer joins it on its own. */
    forget: (id: string, options?: CallOptions) => net({ forget: { id } }, options),
    /** Turns Wi-Fi on or off (decision 7: its one owner). */
    setWifi: (enabled: boolean, options?: CallOptions) => net({ wifi: { enabled } }, options),
    /** A known network's private Wi-Fi address; the network joined is joined again with it
        (ADR 0038, 8 November 2026). */
    setPrivateAddress: (id: string, mode: 'fixed' | 'rotating' | 'off', options?: CallOptions) =>
      net({ privateAddress: { id, mode } }, options),
    /** What resetting the network settings would remove and keep; nothing changes. */
    resetPreview: (options?: CallOptions) => net({ networkResetPreview: {} }, options),
    /** Removes the saved networks and VPNs this person may, with their passwords. */
    reset: (options?: CallOptions) => net({ networkReset: {} }, options),
    /** Wi-Fi Diagnostics, run once: the link, the router, the internet, DNS. */
    diagnose: (options?: CallOptions) => net({ wifiDiagnostics: {} }, options),
  };

  /** Bluetooth (docs/adr/0055-bluetooth.mdx): the `bluetooth` grant, or `shell`. */
  const bluetooth = {
    available,
    /** The adapter and its devices, as the session tells them, followed. */
    state: state.bluetooth as Followed<Json>,
    /** Turns Bluetooth on or off. */
    setPower: (on: boolean, options?: CallOptions) => bt({ bluetoothPower: { on } }, options),
    /** Holds discovery while `on` (Settings' Bluetooth page while it is open). */
    discover: (on: boolean, options?: CallOptions) => bt({ bluetoothDiscover: { on } }, options),
    /** Connects the device `id`; a code a pairing needs is asked in the system's sheet. */
    connect: (id: string, options?: CallOptions) => bt({ bluetoothConnect: { id } }, options),
    /** Disconnects the device `id`. */
    disconnect: (id: string, options?: CallOptions) => bt({ bluetoothDisconnect: { id } }, options),
    /** Forgets the paired device `id`. */
    forget: (id: string, options?: CallOptions) => bt({ bluetoothForget: { id } }, options),
    /** Renames the device `id`; resolves with the name it has then. */
    rename: (id: string, name: string, options?: CallOptions) => bt({ bluetoothRename: { id, name } }, options),
  };

  /** The battery and energy (docs/adr/0057-battery-and-energy.mdx): `settings` or `shell`. */
  const power = {
    available,
    /** The battery, Low Power Mode and the charge limit, as the session tells them, followed. */
    state: state.power as Followed<Json>,
    /** The battery's level and use over time, as Settings' Battery page charts them. */
    history: (options?: CallOptions) => pw({ powerHistory: {} }, options),
    /** The applications' energy use now, sampled as it is asked. */
    energy: (options?: CallOptions) => pw({ energy: {} }, options),
    /** Holds the charge at the battery's limit, or not; rejects saying why where the battery cannot. */
    setChargeLimit: (enabled: boolean, options?: CallOptions) => pw({ chargeLimit: { enabled } }, options),
  };

  /** The sound's devices (docs/adr/0056-sound-devices.mdx): `settings` or `shell`. */
  const audio = {
    available,
    /** The outputs and inputs, each side's devices and the chosen one's volume. */
    state: state.sound as Followed<Json>,
    /** The chosen input's level, 0 to 1: metered exactly while something follows it. */
    inputLevel: state.inputLevel as Followed<number>,
    /** Chooses the output or input device `id`. */
    selectDevice: (direction: Direction, id: string, options?: CallOptions) =>
      au({ soundDevice: { direction, id } }, options),
    /** Sets the chosen output's or input's volume, 0 to 1 (decision 7: its one owner). */
    setVolume: (direction: Direction, volume: number, options?: CallOptions) =>
      au({ soundVolume: { direction, volume } }, options),
    /** Mutes or unmutes the chosen output or input (decision 7: its one owner). */
    setMuted: (direction: Direction, muted: boolean, options?: CallOptions) =>
      au({ soundMuted: { direction, muted } }, options),
    /** Sets the output's balance, -1 (left) to 1 (right). */
    setBalance: (balance: number, options?: CallOptions) => au({ soundBalance: { balance } }, options),
  };

  /** The session's settings: updates (`updates`), input sources (`settings`), privacy (`privacy`). */
  const settings = {
    available,
    /** Signs the person out (docs/adr/0020-oxy-account-login.mdx, decision 5), citing the press or
        key that asked: the session ends, back to the login screen. `settings` grant. */
    signOut: (options?: CallOptions) => st({ signOut: {} }, options),
    /** The account's updates (ADR 0030, decision 16). */
    updates: state.updateState as Followed<Json>,
    /** Asks for the updates' state, which then arrives in `updates` (not asked as it is first
        followed: asking asks the install service, which an install would wait behind). */
    refreshUpdates: (options?: CallOptions) => st({ state: {} }, options),
    /** Asks the store for updates now; they arrive in `updates`. */
    checkUpdates: (options?: CallOptions) => st({ check: {} }, options),
    /** Updates the applications `ids`. */
    update: (ids: string[], options?: CallOptions) => st({ update: { ids } }, options),
    /** Updates every application that has an update. */
    updateAll: (options?: CallOptions) => st({ updateAll: {} }, options),
    /** The input sources in effect and the input methods the engine has (ADR 0041). */
    inputSources: state.inputSources as Followed<Json>,
    /** Chooses the input source `id`, citing the press or key that chose it. */
    selectInputSource: (id: string, options?: CallOptions) => st({ selectInputSource: { id } }, options),
    /** The account's decisions on its applications' capabilities (`privacy`). */
    privacy: (options?: CallOptions) => st({ privacy: {} }, options),
    /** Those decisions as told: by `privacy`'s answers, and again, unasked, when they changed
        elsewhere (an application granted a capability as it registered its login item, or one
        running in the background or no longer; docs/adr/0060-login-items-and-background-
        activity.mdx). Not asked as it is first followed: the page that shows it asks. */
    decisions: state.privacy as Followed<Json[]>,
    /** Records the account's decision on `capability` for the application `id` (`privacy`). */
    decide: (id: string, capability: string, granted: boolean, options?: CallOptions) =>
      st({ decide: { id, capability, granted } }, options),
    /** Settings › Storage (docs/adr/0062-storage.mdx): the session's measurement of the volume, the
        system's image, the Trash and each application, asked as the page shows it
        (`{storage: {volume, system, trash, apps}}`). */
    storage: (options?: CallOptions) => st({ storage: {} }, options),
    /** Shows an item in the file manager, citing the press that asked (`{revealed: {}}`). */
    reveal: (uri: string, options?: CallOptions) => st({ reveal: { uri } }, options),
    /** Removes an application through the store's uninstall, confirmed in the system's sheet
        (`{uninstalled: {id}}` or `{cancelled: {}}`). */
    uninstall: (id: string, options?: CallOptions) => st({ uninstall: { id } }, options),
    /** Reset… (docs/adr/0061-crashes-and-resetting-an-application.mdx): confirmed in the system's
        sheet, the application quit and its own data erased (`{reset: {id}}`). */
    resetApplication: (id: string, options?: CallOptions) => st({ resetApplication: { id } }, options),
  };

  const bk = call('backup');
  /**
   * Backups (docs/adr/0059-backups.mdx): the backup service's state, followed as `{backup: State}`
   * (`{backup: null}` while the service is not there; State: `{automatic, destinations, excluded,
   * running, lastBackup, nextBackup, held, administrator, inMenuBar}`), told to the shell (its
   * `shell` grant: the menu bar's backup item) and Settings (`settings`: its Backups page). Both back
   * up now and skip a backup; Settings alone lists the disks that can be added, adds a disk or a
   * share, removes a destination, sets the excluded items and Back Up Automatically. Each cites the
   * press that asked; the service decides who is an administrator.
   */
  const backup = {
    available,
    /** The backup service's state as the session tells it, asked as it is first followed. */
    state: state.backup as Followed<{ backup: Json | null }>,
    /** Backs up now, to every destination. */
    backUpNow: (options?: CallOptions) => bk({ backupNow: {} }, options),
    /** Skips the backup running. */
    stop: (options?: CallOptions) => bk({ backupStop: {} }, options),
    /** The disks that can be added as a destination (Settings alone). */
    disks: (options?: CallOptions) => bk({ backupDisks: {} }, options),
    /** Adds the disk `uuid`, encrypted with `password` (null: not encrypted). */
    addDisk: (uuid: string, password: string | null, options?: CallOptions) =>
      bk({ backupAddDisk: { uuid, password } }, options),
    /** Adds the network share `share`, signed in to with `sharePassword`, encrypted with `password`. */
    addShare: (share: string, sharePassword: string | null, password: string | null, options?: CallOptions) =>
      bk({ backupAddShare: { share, sharePassword, password } }, options),
    /** Removes the destination `id`; its backups stay on it. */
    remove: (id: string, options?: CallOptions) => bk({ backupRemove: { id } }, options),
    /** The items of the home left out of every backup. */
    setExcluded: (paths: readonly string[], options?: CallOptions) =>
      bk({ backupExcluded: { paths: [...paths] } }, options),
    /** Back Up Automatically, on or off. */
    setAutomatic: (enabled: boolean, options?: CallOptions) => bk({ backupAutomatic: { enabled } }, options),
  };

  /** Installing, updating and removing applications (ADR 0030, decision 9): the `store` grant. */
  const store = {
    available,
    /** Where packages stand, by their ids or by the store's listings (the session resolves a
        listing to the package built for this machine). */
    status: (of: { ids: string[] } | { listings: string[] }, options?: CallOptions) => sto({ status: of }, options),
    /** Installs a package by its id or a listing; resolves `{installed}`, or `{cancelled}` when
        the person declined in the system's sheet. */
    install: (what: { id: string } | { listing: string }, options?: ProgressOptions) =>
      sto({ install: what }, options),
    /** Removes the application `id`, the person confirming in the system's sheet. */
    uninstall: (id: string, options?: CallOptions) => sto({ uninstall: { id } }, options),
    /** Opens the installed application `id`. */
    open: (id: string, options?: CallOptions) => sto({ open: { id } }, options),
  };

  /**
   * The machine's use and every process's (docs/adr/0063-activity-monitor.mdx): the `activity`
   * grant (Activity Monitor). `sample` is followed: its first follower begins the session's
   * sampling of `setView`'s view, every two seconds while the follower's window is shown, and its
   * last ends it. A process is named by its pid and its start (`started`, as a sample gives it).
   */
  const act = call('activity');
  const activity = {
    available,
    /** The machine's use and every process's, sampled every two seconds while followed. */
    sample: state.activity as Followed<Json>,
    /** Which view the samples are of: what its columns need (`cpu`, `memory`, `energy`, ...). */
    setView: (view: ActivityView) => session.activityView(view),
    /** Quits one of the person's processes, citing the press that confirmed it (SIGKILL with `force`). */
    quit: (pid: number, started: number, force: boolean, options?: CallOptions) =>
      act({ activityQuit: { pid, started, force } }, options),
    /** Quits an application the session launched, as the dock does (`force`: Force Quit). */
    quitApp: (id: string, force: boolean, options?: CallOptions) => act({ activityQuitApp: { id, force } }, options),
    /** One process's details: its parent, memory, open files and ports, control group. */
    inspect: (pid: number, started: number, options?: CallOptions) =>
      act({ activityInspect: { pid, started } }, options),
  };

  return { network, bluetooth, power, audio, settings, backup, store, activity };
}

export type Frameworks = ReturnType<typeof createFrameworks>;
export { PlatformError };
