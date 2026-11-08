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
    /** Records the account's decision on `capability` for the application `id` (`privacy`). */
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
    /** Removes the application `id`, the person confirming in the system's sheet. */
    uninstall: (id: string, options?: CallOptions) => sto({ uninstall: { id } }, options),
    /** Opens the installed application `id`. */
    open: (id: string, options?: CallOptions) => sto({ open: { id } }, options),
  };

  return { network, bluetooth, power, audio, settings, store };
}

export type Frameworks = ReturnType<typeof createFrameworks>;
export { PlatformError };
