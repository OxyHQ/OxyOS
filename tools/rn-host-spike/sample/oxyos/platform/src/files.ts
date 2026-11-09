// `files` (docs/adr/0058-the-platform-api.mdx, decision 1; docs/adr/0028-file-access.mdx): items,
// places, the Trash, watching, transfers and tags, through the host's file service (`OxyOSFiles`,
// crates/oxyos-files): the person's home and volumes and the application's own folders, by `file://`
// URIs. An application's window has it, and the session's shell for its desktop; the login screen
// does not.
//
// The person's files (the home and their volumes) are an installed version's only once its account
// granted `files`, or the items the person gave it (chose for it, dropped on it, opened with it):
// the service refuses the rest (`notPermitted`, reason `notGranted`). Asked at first use
// (docs/adr/0030-installing-applications.mdx, decision 8): a call so refused asks the person once,
// in the system's sheet, and is made again if they grant it.
import { PlatformError, type PlatformErrorCode } from './errors.ts';
import type { CallOptions } from './frameworks.ts';
import type { Events } from './session.ts';

/** What the host's module answers: the service's JSON, and its bytes or text. */
export interface FilesReplyText {
  json: string;
  bytes?: Uint8Array;
  text?: string;
}
/** A write's content, beside a request. */
export type FileContent = string | ArrayBuffer | ArrayBufferView;

/** The host's module (OxyOSFiles): a request answered at once (`call`), or on the service's thread
    (`request`, as `id`); `as` "text" gives the content as a string. Absent where no window is. */
export interface FilesModule {
  /** `backups`: whether the application may ask the person's backups (granted `files` at launch). */
  getConstants?(): { backups?: boolean };
  call(json: string, content?: FileContent, as?: 'text'): FilesReplyText;
  request(id: number, json: string, content?: FileContent, as?: 'text'): Promise<FilesReplyText>;
}

/** An answer of the service: its value, and the content read. */
export interface FileReply {
  value: any;
  bytes?: Uint8Array;
  text?: string;
}

/** The platform's code for each of the service's (crates/oxyos-files' `Error::code`). */
const CODES: Record<string, PlatformErrorCode> = {
  'not-found': 'notFound',
  'no-destination': 'notFound',
  exists: 'exists',
  refused: 'notPermitted',
  'not-granted': 'notPermitted',
  'not-permitted': 'notPermitted',
  denied: 'notPermitted',
  invalid: 'invalid',
  'invalid-name': 'invalid',
  'not-a-folder': 'invalid',
  'not-a-file': 'invalid',
  cancelled: 'cancelled',
  unavailable: 'unavailable',
  unsupported: 'unavailable',
};

/** The service's code in the platform's words: `not-granted` is `notGranted`. */
const word = (code: string) => code.replace(/-([a-z])/g, (_, letter: string) => letter.toUpperCase());

/** The PlatformError the service's error is: its code, its own word for it (`reason`) and what it
    said (`details.message`). */
export function fileError(code: string, message?: string): PlatformError {
  return new PlatformError('files', CODES[code] ?? 'failed', word(code), {
    refused: code === 'refused' || code === 'not-granted' || code === 'not-permitted',
    details: message ? { message } : {},
  });
}

const notGranted = (error: unknown) => error instanceof PlatformError && error.reason === 'notGranted';

/** The service's answer as its value and content, or its error thrown. */
function settle(reply: FilesReplyText): FileReply {
  const answer = JSON.parse(reply.json);
  if (!answer.ok) throw fileError(String(answer.error), answer.message);
  return { value: answer.value, bytes: reply.bytes, text: reply.text };
}

/** A request's content and how its content comes back. */
export interface FileRequestOptions extends CallOptions {
  content?: FileContent;
  as?: 'text';
  /** Told the request's id first: a transfer's, to follow or cancel it. */
  started?: (id: number) => void;
}

/**
 * The connection to the file service, the runtime's own (ADR 0058, decision 2): its vocabulary
 * (crates/oxyos-files' `service.rs`), for the framework below and the Expo module the runtime
 * implements (expo-file-system, whose synchronous methods Expo defines so: `call` answers at once).
 */
export function createFileService(
  native: FilesModule | null | undefined,
  events: Events,
  requestCapability: (capability: string) => Promise<boolean>,
) {
  const unavailable = () =>
    fileError('unavailable', "files are reached from an application's window or the session's shell, not the login screen");
  const watches = new Map<number, (event: any) => void>();
  const transfers = new Map<number, (progress: any) => void>();
  if (native) {
    events.addListener('OxyOSFilesEvent', (json: string) => {
      const event = JSON.parse(json);
      if (event.watch != null) {
        watches.get(event.watch)?.(event);
        if (event.ended) watches.delete(event.watch);
      } else if (event.transfer != null) {
        transfers.get(event.transfer)?.(event.progress);
      }
    });
  }
  let nextRequest = 0;

  /** `op` with `args`, at once (on the JavaScript thread); its errors thrown. */
  function call(op: string, args: Record<string, unknown> = {}, content?: FileContent, as?: 'text'): FileReply {
    if (!native) throw unavailable();
    return settle(native.call(JSON.stringify({ ...args, op }), content, as));
  }

  /** `op` with `args` on the service's thread. Refused the person's files, it asks for them once
      and is made again if they are granted. Aborted, it rejects with `cancelled` at once. */
  async function request(op: string, args: Record<string, unknown> = {}, options: FileRequestOptions = {}) {
    if (!native) throw unavailable();
    const { content, as, signal, started } = options;
    const once = async (): Promise<FileReply> => {
      if (signal?.aborted) throw fileError('cancelled');
      const id = ++nextRequest;
      started?.(id);
      const asked = native.request(id, JSON.stringify({ ...args, op }), content, as);
      if (!signal) return settle(await asked);
      return new Promise<FileReply>((resolve, reject) => {
        const abort = () => reject(fileError('cancelled'));
        signal.addEventListener('abort', abort, { once: true });
        asked.then(
          (reply) => {
            signal.removeEventListener('abort', abort);
            try {
              resolve(settle(reply));
            } catch (error) {
              reject(error);
            }
          },
          (error) => {
            signal.removeEventListener('abort', abort);
            reject(error);
          },
        );
      });
    };
    try {
      return await once();
    } catch (error) {
      if (!notGranted(error) || !(await requestCapability('files'))) throw error;
      return once();
    }
  }

  return {
    available: Boolean(native),
    call,
    request,
    /** Calls `handler` with each event of watch `id` until `unwatch(id)` or its end. */
    onWatch(id: number, handler: (event: any) => void) {
      watches.set(id, handler);
    },
    unwatch(id: number) {
      watches.delete(id);
      if (native) call('unwatch', { watch: id });
    },
    /** Asks the service to stop the transfer or restore running as request `id`. */
    cancelTransfer(id: number) {
      if (native) call('cancelTransfer', { transfer: id });
    },
    /** Whether the person's backups may be asked (docs/adr/0059-backups.mdx): only by an
        application granted `files`; the service refuses the rest (`notPermitted`). */
    backupsAvailable: native?.getConstants?.().backups === true,
    /** Calls `handler` with transfer `id`'s progress until `offTransfer(id)`. */
    onTransfer(id: number, handler: (progress: any) => void) {
      transfers.set(id, handler);
    },
    offTransfer(id: number) {
      transfers.delete(id);
    },
    /** Whether the person's files may be asked for again: `files` granted at first use. */
    requestCapability,
  };
}

export type FileService = ReturnType<typeof createFileService>;

/** What a path reaches. */
export interface FileInfo {
  exists: boolean;
  directory: boolean;
  size: number | null;
  /** When it last changed and was made, in milliseconds since the epoch. */
  modified: number | null;
  created: number | null;
}
/** An item of a folder. */
export interface FolderItem {
  uri: string;
  isDirectory: boolean;
}
/** A tag: its name, and one of Finder's seven colours or none (docs/adr/0049-file-tags.mdx). */
export interface FileTag {
  name: string;
  color: 'red' | 'orange' | 'yellow' | 'green' | 'blue' | 'purple' | 'grey' | null;
}
/** What came of one item of several: its result (its URI after), or why not. */
export type ItemOutcome =
  | { uri: string; ok: true; result: string }
  | { uri: string; ok: false; code: PlatformErrorCode; reason: string };
/** The person's places: the home, the favourites (their standard folders) and the volumes. */
export interface Places {
  home: string;
  favourites: { key: string; uri: string }[];
  volumes: { name: string; uri: string }[];
  /** How many items the Trash holds. */
  trash: number;
}
/** An item in the Trash. */
export interface TrashedItem {
  uri: string;
  name: string;
  isDirectory: boolean;
  originalUri: string | null;
  deletedAt: number | null;
  size: number | null;
  lastModified: number | null;
  tags: FileTag[];
}
/** An item the index of tagged items found. */
export interface TaggedItem {
  uri: string;
  name: string;
  isDirectory: boolean;
  size: number | null;
  lastModified: number | null;
  tags: FileTag[];
}
/** A backup, as the backup service names it (docs/adr/0059-backups.mdx). */
export interface BackupRef {
  destination: string;
  id: string;
}

/** What a watch of a folder saw. */
export interface FileEvent {
  type: string;
  uri: string;
  isDirectory: boolean;
  flags: number;
  newUri?: string;
  newIsDirectory?: boolean;
}

export function createFiles(service: FileService) {
  const ask = (op: string, args?: Record<string, unknown>, options?: FileRequestOptions) =>
    service.request(op, args, options);
  const value = async (op: string, args?: Record<string, unknown>, options?: FileRequestOptions) =>
    (await ask(op, args, options)).value;

  /** Each item's outcome; those refused the person's files are asked for once and done again. */
  async function each(op: string, uris: readonly string[], more: Record<string, unknown> = {}, options?: CallOptions) {
    const told = (items: any[]): ItemOutcome[] =>
      items.map((item) =>
        item.ok
          ? { uri: item.uri, ok: true, result: item.result }
          : { uri: item.uri, ok: false, code: CODES[item.error] ?? 'failed', reason: word(String(item.error)) },
      );
    const outcomes = told(await value(op, { uris: [...uris], ...more }, options));
    const refused = outcomes.filter((outcome) => !outcome.ok && outcome.reason === 'notGranted').map(({ uri }) => uri);
    if (refused.length === 0 || !(await service.requestCapability('files'))) return outcomes;
    const again = new Map(told(await value(op, { uris: refused, ...more }, options)).map((outcome) => [outcome.uri, outcome]));
    return outcomes.map((outcome) => again.get(outcome.uri) ?? outcome);
  }

  /** A watch the service starts with `op`, its events told to `listener` until the function
      returned is called. */
  function watching(op: string, args: Record<string, unknown>, listener: (event: any) => void): () => void {
    let watch: number | null = null;
    let ended = false;
    ask(op, args).then(
      ({ value: id }) => {
        if (ended) return service.unwatch(id);
        watch = id;
        service.onWatch(id, listener);
      },
      (error) => console.error(`OxyOS could not watch (${op}): ${error.message}`),
    );
    return () => {
      ended = true;
      if (watch != null) service.unwatch(watch);
    };
  }

  return {
    available: service.available,
    /** What `uri` reaches. */
    info: (uri: string, options?: CallOptions): Promise<FileInfo> => value('stat', { uri }, options),
    /** A folder's items. */
    list: (folder: string, options?: CallOptions): Promise<FolderItem[]> => value('list', { uri: folder }, options),
    /** A file's content: text (UTF-8), base64, or its bytes. */
    async read(uri: string, as: 'text' | 'base64' | 'bytes' = 'text', options?: CallOptions): Promise<string | Uint8Array> {
      const encoding = as === 'text' ? 'utf8' : as;
      const reply = await ask('read', { uri, encoding }, { ...options, as: as === 'bytes' ? undefined : 'text' });
      return as === 'bytes' ? (reply.bytes ?? new Uint8Array()) : (reply.text ?? '');
    },
    /** Writes `content` (text as UTF-8, or bytes) to `uri`, after what is there with `append`. */
    async write(uri: string, content: string | Uint8Array, options: CallOptions & { append?: boolean } = {}) {
      const encoding = typeof content === 'string' ? 'utf8' : 'bytes';
      await ask('write', { uri, encoding, append: Boolean(options.append) }, { signal: options.signal, content });
    },
    /** Makes the folder `uri`, and those above it with `intermediates`. */
    async createFolder(uri: string, options: CallOptions & { intermediates?: boolean } = {}) {
      await ask('createDirectory', { uri, intermediates: Boolean(options.intermediates), overwrite: false, idempotent: true }, options);
    },
    /** Deletes `uri` for good (`trash` puts it in the Trash). */
    async delete(uri: string, options?: CallOptions) {
      await ask('delete', { uri }, options);
    },
    /** Copies `uri` into the folder `to`; resolves with the copy's URI. */
    copy: (uri: string, to: string, options: CallOptions & { overwrite?: boolean } = {}): Promise<string> =>
      value('copy', { uri, directory: uri.endsWith('/'), to, toDirectory: true, overwrite: Boolean(options.overwrite) }, options),
    /** Moves `uri` into the folder `to`; resolves with its URI there. */
    move: (uri: string, to: string, options: CallOptions & { overwrite?: boolean } = {}): Promise<string> =>
      value('move', { uri, directory: uri.endsWith('/'), to, toDirectory: true, overwrite: Boolean(options.overwrite) }, options),
    /** Renames `uri` to `name` in its folder; resolves with its new URI. */
    rename: (uri: string, name: string, options?: CallOptions): Promise<string> =>
      value('rename', { uri, directory: uri.endsWith('/'), name }, options),
    /** The media type its name says, or null. */
    mediaType: (uri: string, options?: CallOptions): Promise<string | null> => value('mimeType', { uri }, options),
    /** The space of the volume holding the application's folders, in bytes. */
    diskSpace: (options?: CallOptions): Promise<{ total: number; available: number }> => value('diskSpace', {}, options),
    /** Follows what changes in the folder `uri`; the function returned stops following. */
    watch(uri: string, listener: (events: FileEvent[]) => void, options: { debounce?: number } = {}): () => void {
      return watching('watchPath', { uri, debounce: options.debounce ?? 0 }, (event) => listener(event.events ?? []));
    },

    /** The person's places. */
    places: (options?: CallOptions): Promise<Places> => value('places', {}, options),
    /** Follows the places (a volume mounted, a favourite renamed). */
    onPlacesChange: (listener: () => void) => watching('watchPlaces', {}, () => listener()),
    /** Puts the items in the Trash: each one's outcome, its result its URI in the Trash. */
    trash: (uris: readonly string[], options?: CallOptions) => each('trash', uris, {}, options),
    /** What the Trash holds. */
    trashed: (options?: CallOptions): Promise<TrashedItem[]> => value('trashed', {}, options),
    /** Puts items in the Trash back where they were. */
    restore: (uris: readonly string[], options?: CallOptions) => each('restore', uris, {}, options),
    /** Removes items in the Trash for good. */
    erase: (uris: readonly string[], options?: CallOptions) => each('erase', uris, {}, options),
    /** Empties the Trash; resolves with how many items went. */
    emptyTrash: (options?: CallOptions): Promise<number> => value('emptyTrash', {}, options),
    /** Follows the Trash. */
    onTrashChange: (listener: () => void) => watching('watchTrash', {}, () => listener()),

    /** Tags (docs/adr/0049-file-tags.mdx). */
    tags: {
      /** The tags of the items of the folder `uri` that have any, by their URIs. */
      in: (folder: string, options?: CallOptions): Promise<Record<string, FileTag[]>> =>
        value('tagsIn', { uri: folder }, options),
      /** Each item's tags; null for one not there or not reached. */
      of: (uris: readonly string[], options?: CallOptions): Promise<(FileTag[] | null)[]> =>
        value('tagsOf', { uris: [...uris] }, options),
      /** Removes the tags named `remove`, then adds `add`, on each item. */
      change: (
        uris: readonly string[],
        change: { add?: readonly { name: string; color?: FileTag['color'] }[]; remove?: readonly string[] },
        options?: CallOptions,
      ) =>
        each(
          'changeTags',
          uris,
          {
            add: (change.add ?? []).map(({ name, color }) => ({ name, color: color ?? null })),
            remove: [...(change.remove ?? [])],
          },
          options,
        ),
      /** The items tagged `name`, from the index of the person's tagged items. */
      tagged: (name: string, options?: CallOptions): Promise<TaggedItem[]> => value('tagged', { name }, options),
      /** Every tag's name, and how many items have it. */
      names: (options?: CallOptions): Promise<{ name: string; count: number }[]> => value('tagNames', {}, options),
      /** Follows the index of tagged items. */
      onChange: (listener: () => void) => watching('watchTags', {}, () => listener()),
    },

    /**
     * The person's backups (docs/adr/0059-backups.mdx), asked of the backup service from this
     * process as the person: only by an application granted `files` (`available`); the others are
     * refused with `notPermitted`.
     */
    backups: {
      available: service.backupsAvailable,
      /** The service's state: its destinations, the last and next backups. */
      state: (options?: CallOptions): Promise<any> => value('backups', {}, options),
      /** Follows the service's state; the function returned stops following. */
      onChange: (listener: () => void) => watching('watchBackups', {}, () => listener()),
      /** The backups that hold a version of `uri`. */
      versions: (uri: string, options?: CallOptions): Promise<any[]> => value('versions', { uri }, options),
      /** What the folder `uri` held in `backup`. */
      contents: (backup: BackupRef, folder: string, options?: CallOptions): Promise<any[]> =>
        value('versionContents', { backup: { destination: backup.destination, id: backup.id }, uri: folder }, options),
      /**
       * Restores `uri` as `backup` held it, written by this process as the person: beside what is
       * there (or in the folder `to`) unless `replace`; resolves with what was written. Its
       * progress is told as a transfer's; aborted, the service stops it and it rejects with
       * `cancelled`.
       */
      async restore(
        backup: BackupRef,
        uri: string,
        options: CallOptions & { to?: string | null; replace?: boolean; onProgress?: (progress: any) => void } = {},
      ): Promise<any> {
        const { signal, onProgress } = options;
        let running: number | null = null;
        const stop = () => {
          if (running != null) service.cancelTransfer(running);
        };
        signal?.addEventListener('abort', stop, { once: true });
        try {
          const args = {
            backup: { destination: backup.destination, id: backup.id },
            uri,
            to: options.to ?? null,
            replace: Boolean(options.replace),
          };
          return await value('restoreVersion', args, {
            signal,
            started: (id) => {
              if (running != null) service.offTransfer(running);
              running = id;
              if (onProgress) service.onTransfer(id, onProgress);
            },
          });
        } finally {
          signal?.removeEventListener('abort', stop);
          if (running != null) service.offTransfer(running);
        }
      },
    },
  };
}

export type Files = ReturnType<typeof createFiles>;
