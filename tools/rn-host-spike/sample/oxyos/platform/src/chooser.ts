// `chooser` (docs/adr/0058-the-platform-api.mdx, decision 1; docs/adr/0029-chromium-on-oxyos.mdx,
// decision 8): the system's open and save panels, as a Mac's are. An application asks the session
// for files for its window, citing the latest press or key; the session starts the file manager in
// its picker mode as a sheet of that window, and the person's answer comes back to this application
// alone. The application never sees the panel. In the file manager the session started as the
// chooser, `request` is what it was asked to show and `choose` and `cancel` give the person's answer.
import { errorOf, PlatformError } from './errors.ts';
import type { CallOptions } from './frameworks.ts';
import { hostCall } from './host.ts';
import type { Events } from './session.ts';

/** Where a panel starts and what it says, for an open and a save panel alike. */
interface PanelOptions extends CallOptions {
  /**
   * The types it offers: media types (`image/png`, `image/*`) or extensions (`.pdf`). None, or
   * `*\/*`, offers every file; a type that is neither is left out rather than refusing the panel.
   */
  types?: readonly string[];
  /** Its title; none: the system's. */
  title?: string;
  /** The folder it starts in, as a `file://` URI; one the person's files do not hold is ignored. */
  folder?: string;
}
export interface OpenPanelOptions extends PanelOptions {
  /** Several files may be chosen. */
  multiple?: boolean;
  /** Folders are chosen, not files. */
  directory?: boolean;
}
export interface SavePanelOptions extends PanelOptions {
  /** The name it suggests. */
  name?: string;
}

/** What the session asked the file manager it started as the chooser to show. */
export interface ChooserRequest {
  readonly mode: 'open' | 'save';
  readonly multiple: boolean;
  readonly directory: boolean;
  readonly types: readonly string[];
  readonly title: string | null;
  readonly suggestedName: string | null;
  readonly folder: string | null;
  /** The application that asked. */
  readonly application: { readonly id: string; readonly name: string } | null;
}

/** The host's module (OxyOSFileChooser). `choose` and `answer` throw where the application has no
    window; `choose`'s answer arrives as `OxyOSFilesChosen`, naming its serial (0: not sent). */
export interface FileChooserModule {
  getConstants(): { request?: string | null };
  choose(request: string): number;
  cancel(serial: number): void;
  answer(answer: string): boolean;
}

/**
 * The types a panel offers, as the session's vocabulary takes them: media types lowercased without
 * their parameters, extensions as they are; any type, none at all. A type the vocabulary has no
 * words for is left out rather than refusing the whole request.
 */
export function acceptedTypes(given: readonly unknown[] | undefined): string[] {
  const types: string[] = [];
  for (const item of given ?? []) {
    if (typeof item !== 'string') continue;
    const type = item.startsWith('.') ? item : (item.split(';')[0] ?? '').trim().toLowerCase();
    if (type === '*/*' || type === '*') return [];
    const media = /^[a-z0-9!#$&^_.+-]+\/([a-z0-9!#$&^_.+-]+|\*)$/.test(type);
    const extension = /^(\.[A-Za-z0-9_+-]+)+$/.test(type);
    if ((media || extension) && type.length <= 128 && !types.includes(type)) types.push(type);
  }
  return types;
}

/** The request the host was started with, or null in any application but the chooser. */
function readRequest(native: FileChooserModule | null | undefined): ChooserRequest | null {
  const text = native?.getConstants().request ?? null;
  if (typeof text !== 'string') return null;
  try {
    const request = JSON.parse(text);
    return Object.freeze({
      mode: request.mode === 'save' ? 'save' : 'open',
      multiple: request.multiple === true,
      directory: request.directory === true,
      types: Object.freeze([...(request.types ?? [])]),
      title: request.title ?? null,
      suggestedName: request.suggestedName ?? null,
      folder: request.folder ?? null,
      application: request.application
        ? Object.freeze({ id: String(request.application.id), name: String(request.application.name) })
        : null,
    });
  } catch {
    console.error('OxyOS: the file chooser was asked to show what it cannot read');
    return null;
  }
}

/** `{key: value}` without the keys whose value is undefined, as the session reads a request. */
const given = (fields: Record<string, unknown>) =>
  Object.fromEntries(Object.entries(fields).filter(([, value]) => value !== undefined));

export function createChooser(native: FileChooserModule | null | undefined, events: Events) {
  const unavailable = () => new PlatformError('chooser', 'unavailable');
  const request = readRequest(native);
  const waiting = new Map<number, (answer: unknown) => void>();
  if (native) {
    events.addListener('OxyOSFilesChosen', ({ serial, answer }: { serial: number; answer: unknown }) => {
      const resolve = waiting.get(serial);
      if (!resolve) return;
      waiting.delete(serial);
      resolve(answer);
    });
  }

  /** One panel: the URIs chosen, or null where the person cancelled. */
  async function ask(body: Record<string, unknown>, options: CallOptions = {}): Promise<string[] | null> {
    if (!native) throw unavailable();
    let serial = 0;
    const answer = await hostCall(
      'chooser',
      () => {
        // Thrown where the application has no window: hostCall's `unavailable`.
        serial = native.choose(JSON.stringify(body));
        if (!serial) return Promise.resolve({ refused: { reason: 'notSent' } });
        return new Promise<unknown>((resolve) => waiting.set(serial, resolve));
      },
      options,
      // Withdrawn: the session closes the panel, and its answer finds no one waiting.
      () => {
        if (waiting.delete(serial)) native.cancel(serial);
      },
    );
    const error = errorOf('chooser', answer);
    if (error) throw error;
    const told = answer as { chosen?: unknown; cancelled?: unknown };
    if (Array.isArray(told.chosen)) return told.chosen.map(String);
    if (told.cancelled) return null;
    throw new PlatformError('chooser', 'failed', 'noAnswer');
  }

  /** The chooser's answer to the session (sent synchronously by the host), as a promise. */
  async function answer(body: Record<string, unknown>): Promise<void> {
    if (!native || !request) throw unavailable();
    let sent = false;
    try {
      sent = native.answer(JSON.stringify(body));
    } catch {
      throw unavailable();
    }
    if (!sent) throw new PlatformError('chooser', 'unavailable', 'notSent');
  }

  return {
    /** Whether this application can ask for files (it has a window the session started). */
    available: Boolean(native),
    /**
     * The open panel: resolves with the `file://` URIs the person chose (folders' ending in `/`),
     * or null where they cancelled. Rejects with `needsInput` without a recent press or key to
     * cite, `busy` while another panel of this window is open, `locked` while the session is,
     * `unavailable` where there is no file manager to show it. Aborting withdraws the panel.
     */
    open: ({ signal, multiple, directory, types, title, folder }: OpenPanelOptions = {}) =>
      ask(
        { open: given({ multiple: multiple === true, directory: directory === true, types: acceptedTypes(types), title, folder }) },
        { signal },
      ),
    /** The save panel: resolves with the `file://` URI the person chose to save to, or null where
        they cancelled; it rejects as `open` does. */
    save: async ({ signal, name, types, title, folder }: SavePanelOptions = {}): Promise<string | null> => {
      const chosen = await ask({ save: given({ name, types: acceptedTypes(types), title, folder }) }, { signal });
      return chosen?.[0] ?? null;
    },
    /** What the session asked this application to show as the chooser; null in any other. */
    request,
    /** The chooser's answer: the person's choice, citing their latest press or key. The session
        ends the chooser once it has it. */
    choose: (uris: readonly string[]) => answer({ chosen: [...uris] }),
    /** The chooser's answer: the person cancelled. */
    cancel: () => answer({ cancelled: {} }),
  };
}

export type Chooser = ReturnType<typeof createChooser>;
