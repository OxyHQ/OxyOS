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
    forget: (id: string, options?: CallOptions) => net({ forget: { id } }, options),
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
    state: state.bluetooth as Followed<Json>,
    setPower: (on: boolean, options?: CallOptions) => bt({ bluetoothPower: { on } }, options),
    /** Holds discovery while `on` (Settings' Bluetooth page while it is open). */
    discover: (on: boolean, options?: CallOptions) => bt({ bluetoothDiscover: { on } }, options),
    connect: (id: string, options?: CallOptions) => bt({ bluetoothConnect: { id } }, options),
    disconnect: (id: string, options?: CallOptions) => bt({ bluetoothDisconnect: { id } }, options),
    forget: (id: string, options?: CallOptions) => bt({ bluetoothForget: { id } }, options),
    rename: (id: string, name: string, options?: CallOptions) => bt({ bluetoothRename: { id, name } }, options),
  };

  /** The battery and energy (docs/adr/0057-battery-and-energy.mdx): `settings` or `shell`. */
  const power = {
    available,
    state: state.power as Followed<Json>,
    history: (options?: CallOptions) => pw({ powerHistory: {} }, options),
    energy: (options?: CallOptions) => pw({ energy: {} }, options),
    setChargeLimit: (enabled: boolean, options?: CallOptions) => pw({ chargeLimit: { enabled } }, options),
  };

  /** The sound's devices (docs/adr/0056-sound-devices.mdx): `settings` or `shell`. */
  const audio = {
    available,
    /** The outputs and inputs, each side's devices and the chosen one's volume. */
    state: state.sound as Followed<Json>,
    /** The chosen input's level, 0 to 1: metered exactly while something follows it. */
    inputLevel: state.inputLevel as Followed<number>,
    selectDevice: (direction: Direction, id: string, options?: CallOptions) =>
      au({ soundDevice: { direction, id } }, options),
    setVolume: (direction: Direction, volume: number, options?: CallOptions) =>
      au({ soundVolume: { direction, volume } }, options),
    setMuted: (direction: Direction, muted: boolean, options?: CallOptions) =>
      au({ soundMuted: { direction, muted } }, options),
    setBalance: (balance: number, options?: CallOptions) => au({ soundBalance: { balance } }, options),
  };

  /** The session's settings: updates (`updates`), input sources (`settings`), privacy (`privacy`). */
  const settings = {
    available,
    /** The account's updates (ADR 0030, decision 16). */
    updates: state.updateState as Followed<Json>,
    /** Asks for the updates' state, which then arrives in `updates` (not asked as it is first
        followed: asking asks the install service, which an install would wait behind). */
    refreshUpdates: (options?: CallOptions) => st({ state: {} }, options),
    checkUpdates: (options?: CallOptions) => st({ check: {} }, options),
    update: (ids: string[], options?: CallOptions) => st({ update: { ids } }, options),
    updateAll: (options?: CallOptions) => st({ updateAll: {} }, options),
    /** The input sources in effect and the input methods the engine has (ADR 0041). */
    inputSources: state.inputSources as Followed<Json>,
    selectInputSource: (id: string, options?: CallOptions) => st({ selectInputSource: { id } }, options),
    /** The account's decisions on its applications' capabilities (`privacy`). */
    privacy: (options?: CallOptions) => st({ privacy: {} }, options),
    /** Those decisions as told: by `privacy`'s answers, and again, unasked, when they changed
        elsewhere (an application granted a capability as it registered its login item, or one
        running in the background or no longer; docs/adr/0060-login-items-and-background-
        activity.mdx). Not asked as it is first followed: the page that shows it asks. */
    decisions: state.privacy as Followed<Json[]>,
    decide: (id: string, capability: string, granted: boolean, options?: CallOptions) =>
      st({ decide: { id, capability, granted } }, options),
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
    uninstall: (id: string, options?: CallOptions) => sto({ uninstall: { id } }, options),
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
    sample: state.activity as Followed<Json>,
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

  return { network, bluetooth, power, audio, settings, store, activity };
}

export type Frameworks = ReturnType<typeof createFrameworks>;
export { PlatformError };
