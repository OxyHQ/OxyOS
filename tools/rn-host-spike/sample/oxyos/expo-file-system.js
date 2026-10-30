/**
 * expo-file-system on OxyOS (docs/adr/0028-file-access.mdx): the native module its JavaScript
 * reaches (`FileSystem`: `File`, `Directory`, `Paths`, file handles, watchers, downloads and
 * uploads), as its Android implementation behaves, over the host's file service (files.js). The
 * same application code runs on Android, iOS and OxyOS.
 *
 * - What a URI may name is the platform's rule: the home, the volumes mounted for the user, the
 *   application's own folders (`Paths.document` is `$XDG_DATA_HOME/oxyos/apps/ID` and
 *   `Paths.cache` `$XDG_CACHE_HOME/oxyos/apps/ID`, where docs/adr/0030-installing-applications.mdx
 *   keeps an application's data, ID the id the session's launch gave it; an application the session
 *   did not start has none, application.h) and its bundle's folder
 *   (`Paths.bundle`, read-only); never a Trash's insides, never the home, a volume's root or the
 *   application's folders themselves changed. Anything else fails as Android's missing permission
 *   does, with the reason.
 * - Synchronous methods are synchronous (on the JavaScript thread, as on Android); asynchronous ones
 *   run on the service's thread.
 * - What OxyOS cannot do yet fails loudly: the system's file and folder pickers (there are none),
 *   and previewing a file (opening it in another application is not there yet: `canPreview` is
 *   false). `expo-file-system/legacy` (`ExponentFileSystem`) is its folders and its uploads, as
 *   the Oxy SDK's native upload uses them; its other methods fail, naming the API to use instead.
 */
import {
  FilesError,
  callFiles,
  filesAvailable,
  offTransfer,
  onTransfer,
  onWatch,
  requestFiles,
  unwatchFiles,
} from './files';

const { NativeModule, SharedObject } = globalThis.expo;

/** An error as expo-file-system's native side throws one: its message, and Android's code. */
function codedError(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}

/** A service error as expo-file-system says it, for `what` done to `uri`. */
function expoError(error, what, uri) {
  if (!(error instanceof FilesError)) return error;
  const reason = error.message;
  switch (error.code) {
    case 'refused':
      return codedError('ERR_INVALID_PERMISSION', `OxyOS does not let an application reach ${uri}: ${reason}`);
    case 'not-found':
      return what === 'delete'
        ? codedError('ERR_UNABLE_TO_DELETE', `Unable to delete file or directory: uri '${uri}' does not exist`)
        : codedError('ERR_FILE_NOT_FOUND', `Unable to ${what} '${uri}': nothing is there`);
    case 'exists':
      return what === 'create'
        ? codedError('ERR_UNABLE_TO_CREATE', `Unable to create file or directory: '${uri}' already exists`)
        : codedError('ERR_DESTINATION_ALREADY_EXISTS', 'Destination already exists');
    case 'no-destination':
      return codedError('ERR_DESTINATION_DOES_NOT_EXIST', 'The destination path does not exist');
    case 'not-a-file':
      return codedError('ERR_INVALID_TYPE_FILE', 'A folder with the same name already exists in the file location');
    case 'not-a-folder':
      return codedError('ERR_INVALID_TYPE_FOLDER', 'A file with the same name already exists in the folder location');
    case 'invalid-name':
      return codedError('ERR_UNABLE_TO_CREATE', 'Unable to create file or directory: child name must be a single path segment');
    case 'cancelled':
      return what === 'upload'
        ? codedError('ERR_UPLOAD_CANCELLED', 'Upload was cancelled')
        : codedError('ERR_DOWNLOAD_CANCELLED', 'Download was cancelled');
    case 'network':
      return codedError(what === 'upload' ? 'ERR_UNABLE_TO_UPLOAD' : 'ERR_UNABLE_TO_DOWNLOAD', reason);
    case 'unavailable':
      return codedError('ERR_FILE_SYSTEM_UNAVAILABLE', `expo-file-system: ${reason}`);
    default:
      return codedError(`ERR_FILE_SYSTEM_${error.code.toUpperCase().replace(/-/g, '_')}`, `Unable to ${what} '${uri}': ${reason}`);
  }
}

/** `op` at once, its errors as Expo's. */
function sync(what, uri, op, args, content, as) {
  try {
    return callFiles(op, { uri, ...args }, content, as);
  } catch (error) {
    throw expoError(error, what, uri);
  }
}

/** `op` on the service's thread, its errors as Expo's. */
async function later(what, uri, op, args, content, as) {
  try {
    return await requestFiles(op, { uri, ...args }, content, as);
  } catch (error) {
    throw expoError(error, what, uri);
  }
}

const unsupported = (what, reason) => codedError('ERR_FILE_SYSTEM_UNSUPPORTED', `${what} is not available on OxyOS: ${reason}`);
const NO_PICKER = 'OxyOS has no system file picker yet';
const NO_PREVIEW = 'opening a file in another application is not there yet on OxyOS';

const URI = Symbol('uri');
const withoutSlash = (uri) => (uri.length > 'file:///'.length ? uri.replace(/\/+$/, '') : uri);
const withSlash = (uri) => (uri.endsWith('/') ? uri : `${uri}/`);

function checked(uri) {
  if (typeof uri !== 'string' || !/^file:/i.test(uri)) {
    throw codedError('ERR_INVALID_URI', `OxyOS reaches files by file:// URIs, not ${String(uri)}`);
  }
  return uri;
}

/** What a write's content and encoding are, as the service takes them. */
function content(given, options) {
  if (typeof given === 'string') {
    return { content: given, encoding: options?.encoding === 'base64' ? 'base64' : 'utf8' };
  }
  if (given instanceof ArrayBuffer || ArrayBuffer.isView(given)) return { content: given, encoding: 'bytes' };
  throw new TypeError('expo-file-system: write takes a string, a Uint8Array or an ArrayBuffer');
}

/** The facts of what a URI reaches, or null when the scope refuses it (a getter's "cannot be read"). */
function facts(uri) {
  try {
    return callFiles('stat', { uri }).value;
  } catch (error) {
    if (error instanceof FilesError && error.code !== 'unavailable') return null;
    throw expoError(error, 'read', uri);
  }
}

/** Where a copy or a move goes: its destination's URI and kind. */
function destination(to) {
  if (to instanceof FileSystemDirectory) return { to: to.uri, toDirectory: true };
  if (to instanceof FileSystemFile) return { to: to.uri, toDirectory: false };
  throw new TypeError('expo-file-system: a destination is a File or a Directory');
}

/** An operation on an open file, at once; its errors as Android's handle's (`code`). */
function onHandle(handle, op, args, bytes, code) {
  if (handle.closed) throw codedError(code, "Unable to read from a file handle: 'file handle is closed'");
  try {
    return callFiles(op, { handle: handle.handle, ...args }, bytes);
  } catch (error) {
    throw codedError(code, error.message);
  }
}

class FileSystemFileHandle extends SharedObject {
  constructor(handle) {
    super();
    this.handle = handle;
    this.closed = false;
  }

  readBytesSync(length) {
    return onHandle(this, 'handleRead', { length }, undefined, 'ERR_UNABLE_TO_READ_HANDLE').bytes;
  }

  async readBytes(length) {
    if (this.closed) return this.readBytesSync(length);
    try {
      return (await requestFiles('handleRead', { handle: this.handle, length })).bytes;
    } catch (error) {
      throw codedError('ERR_UNABLE_TO_READ_HANDLE', `Unable to read from a file handle: '${error.message}'`);
    }
  }

  writeBytesSync(bytes) {
    onHandle(this, 'handleWrite', {}, bytes, 'ERR_UNABLE_TO_WRITE_HANDLE');
  }

  async writeBytes(bytes) {
    if (this.closed) return this.writeBytesSync(bytes);
    try {
      await requestFiles('handleWrite', { handle: this.handle }, bytes);
    } catch (error) {
      throw codedError('ERR_UNABLE_TO_WRITE_HANDLE', `Unable to write to a file handle: '${error.message}'`);
    }
  }

  get offset() {
    return this.closed ? null : onHandle(this, 'handleOffset', {}, undefined, 'ERR_UNABLE_TO_READ_HANDLE').value;
  }

  set offset(offset) {
    if (offset == null || this.closed) return;
    onHandle(this, 'handleSeek', { offset }, undefined, 'ERR_UNABLE_TO_READ_HANDLE');
  }

  get size() {
    return this.closed ? null : onHandle(this, 'handleSize', {}, undefined, 'ERR_UNABLE_TO_READ_HANDLE').value;
  }

  close() {
    if (this.closed) return;
    this.closed = true;
    callFiles('handleClose', { handle: this.handle });
  }

  release() {
    this.close();
  }
}

class FileSystemFile extends SharedObject {
  constructor(uri) {
    super();
    this[URI] = withoutSlash(String(uri));
  }

  get uri() {
    return this[URI];
  }

  validatePath() {
    checked(this[URI]);
  }

  get exists() {
    const found = facts(this[URI]);
    return !!found?.exists && !found.directory;
  }

  get size() {
    const found = facts(this[URI]);
    return found?.exists && !found.directory ? found.size : 0;
  }

  /** @deprecated as in Expo: `digest('MD5')`. */
  get md5() {
    try {
      return this.exists ? callFiles('digest', { uri: this[URI], algorithm: 'MD5' }).value : null;
    } catch {
      return null;
    }
  }

  get lastModified() {
    const found = sync('read', this[URI], 'stat').value;
    if (found.exists && found.directory) throw expoError(new FilesError('not-a-file', ''), 'read', this[URI]);
    return found.exists ? found.modified : null;
  }

  get modificationTime() {
    return this.lastModified;
  }

  get creationTime() {
    const found = facts(this[URI]);
    return found?.exists ? found.created : null;
  }

  get type() {
    return sync('read', this[URI], 'mimeType').value ?? '';
  }

  textSync() {
    return sync('read', this[URI], 'read', { encoding: 'utf8' }, undefined, 'text').text;
  }

  async text() {
    return (await later('read', this[URI], 'read', { encoding: 'utf8' }, undefined, 'text')).text;
  }

  base64Sync() {
    return sync('read', this[URI], 'read', { encoding: 'base64' }, undefined, 'text').text;
  }

  async base64() {
    return (await later('read', this[URI], 'read', { encoding: 'base64' }, undefined, 'text')).text;
  }

  bytesSync() {
    return sync('read', this[URI], 'read', { encoding: 'bytes' }).bytes;
  }

  async bytes() {
    return (await later('read', this[URI], 'read', { encoding: 'bytes' })).bytes;
  }

  writeSync(given, options) {
    const { content: written, encoding } = content(given, options);
    sync('write', this[URI], 'write', { encoding, append: !!options?.append }, written);
  }

  async write(given, options) {
    const { content: written, encoding } = content(given, options);
    await later('write', this[URI], 'write', { encoding, append: !!options?.append }, written);
  }

  delete() {
    sync('delete', this[URI], 'delete');
  }

  info(options) {
    const found = sync('read', this[URI], 'stat').value;
    if (found.exists && found.directory) throw expoError(new FilesError('not-a-file', ''), 'read', this[URI]);
    if (!found.exists) return { exists: false, uri: this[URI] };
    const info = {
      exists: true,
      uri: this[URI],
      size: found.size,
      modificationTime: found.modified,
      creationTime: found.created,
    };
    if (options?.md5) info.md5 = sync('read', this[URI], 'digest', { algorithm: 'MD5' }).value;
    return info;
  }

  create(options) {
    sync('create', this[URI], 'createFile', { intermediates: !!options?.intermediates, overwrite: !!options?.overwrite });
  }

  async digest(algorithm) {
    return (await later('read', this[URI], 'digest', { algorithm })).value;
  }

  copySync(to, options) {
    sync('copy', this[URI], 'copy', { directory: false, ...destination(to), overwrite: !!options?.overwrite });
  }

  async copy(to, options) {
    await later('copy', this[URI], 'copy', { directory: false, ...destination(to), overwrite: !!options?.overwrite });
  }

  moveSync(to, options) {
    this[URI] = withoutSlash(
      sync('move', this[URI], 'move', { directory: false, ...destination(to), overwrite: !!options?.overwrite }).value,
    );
  }

  async move(to, options) {
    const moved = await later('move', this[URI], 'move', {
      directory: false,
      ...destination(to),
      overwrite: !!options?.overwrite,
    });
    this[URI] = withoutSlash(moved.value);
  }

  rename(name) {
    this[URI] = withoutSlash(sync('rename', this[URI], 'rename', { directory: false, name: String(name) }).value);
  }

  open(mode) {
    return new FileSystemFileHandle(sync('open', this[URI], 'open', { mode: mode ?? 'rw' }).value);
  }

  async canPreview() {
    return false;
  }

  async preview() {
    throw unsupported('File.preview', NO_PREVIEW);
  }
}

class FileSystemDirectory extends SharedObject {
  constructor(uri) {
    super();
    this[URI] = withSlash(String(uri));
  }

  get uri() {
    return this[URI];
  }

  validatePath() {
    checked(this[URI]);
  }

  get exists() {
    const found = facts(this[URI]);
    return !!found?.exists && !!found.directory;
  }

  get size() {
    try {
      return callFiles('size', { uri: this[URI] }).value;
    } catch (error) {
      if (error instanceof FilesError && error.code !== 'unavailable') return null;
      throw expoError(error, 'read', this[URI]);
    }
  }

  info() {
    const found = sync('read', this[URI], 'stat').value;
    if (found.exists && !found.directory) throw expoError(new FilesError('not-a-folder', ''), 'read', this[URI]);
    if (!found.exists) return { exists: false, uri: this[URI] };
    const listed = sync('list', this[URI], 'list').value;
    return {
      exists: true,
      uri: this[URI],
      size: sync('read', this[URI], 'size').value,
      modificationTime: found.modified,
      creationTime: found.created,
      files: listed.map(({ uri }) => decodeURIComponent(withoutSlash(uri).split('/').pop())),
    };
  }

  delete() {
    sync('delete', this[URI], 'delete');
  }

  create(options) {
    sync('create', this[URI], 'createDirectory', {
      intermediates: !!options?.intermediates,
      overwrite: !!options?.overwrite,
      idempotent: !!options?.idempotent,
    });
  }

  createFile(name, _mimeType) {
    return new FileSystemFile(sync('create', this[URI], 'createChild', { name: String(name), directory: false }).value);
  }

  createDirectory(name) {
    return new FileSystemDirectory(sync('create', this[URI], 'createChild', { name: String(name), directory: true }).value);
  }

  copySync(to, options) {
    sync('copy', this[URI], 'copy', { directory: true, ...destination(to), overwrite: !!options?.overwrite });
  }

  async copy(to, options) {
    await later('copy', this[URI], 'copy', { directory: true, ...destination(to), overwrite: !!options?.overwrite });
  }

  moveSync(to, options) {
    this[URI] = withSlash(
      sync('move', this[URI], 'move', { directory: true, ...destination(to), overwrite: !!options?.overwrite }).value,
    );
  }

  async move(to, options) {
    const moved = await later('move', this[URI], 'move', {
      directory: true,
      ...destination(to),
      overwrite: !!options?.overwrite,
    });
    this[URI] = withSlash(moved.value);
  }

  rename(name) {
    this[URI] = withSlash(sync('rename', this[URI], 'rename', { directory: true, name: String(name) }).value);
  }

  listAsRecords() {
    return sync('list', this[URI], 'list').value;
  }
}

/** A typed watch on a folder or a file: `change` events as Android's watcher emits them. */
class FileSystemWatcher extends SharedObject {
  constructor(uri, options) {
    super();
    this.watchedUri = checked(String(uri));
    this.debounce = options?.debounce ?? 100;
    this.watch = null;
  }

  start() {
    if (this.watch != null) return;
    let watch;
    try {
      watch = callFiles('watchPath', { uri: this.watchedUri, debounce: this.debounce }).value;
    } catch (error) {
      if (error instanceof FilesError && error.code === 'not-found') {
        throw codedError('ERR_WATCHER_PATH_NOT_FOUND', `Path does not exist: '${this.watchedUri}'`);
      }
      if (error instanceof FilesError && error.code === 'refused') {
        throw codedError('ERR_WATCHER_PERMISSION', `No permission to watch path '${this.watchedUri}': ${error.message}`);
      }
      throw expoError(error, 'watch', this.watchedUri);
    }
    this.watch = watch;
    onWatch(watch, ({ events, ended }) => {
      for (const event of events) {
        this.emit('change', {
          type: event.type,
          path: event.uri,
          isDirectory: event.isDirectory,
          nativeEventFlags: event.flags,
          ...(event.newUri ? { newPath: event.newUri, newPathIsDirectory: event.newIsDirectory } : {}),
        });
      }
      if (ended) this.watch = null;
    });
  }

  stop() {
    if (this.watch == null) return;
    const watch = this.watch;
    this.watch = null;
    unwatchFiles(watch);
  }

  release() {
    this.stop();
  }
}

/** A request's headers as the service takes them: names to string values. */
const headersOf = (headers) =>
  Object.fromEntries(Object.entries(headers ?? {}).map(([name, value]) => [name, String(value)]));

/** Starts a download, its progress to `progress`; its result's value (the file's URI, or null when paused). */
async function download(args, progress, started) {
  let id;
  try {
    const { value } = await requestFiles('download', args, undefined, undefined, (transfer) => {
      id = transfer;
      started?.(transfer);
      if (progress) onTransfer(transfer, progress);
    });
    return value;
  } catch (error) {
    throw expoError(error, 'download', args.to);
  } finally {
    if (id != null) offTransfer(id);
  }
}

/** Starts an upload, its progress to `progress`; `{body, status, headers}`. */
async function upload(args, progress, started) {
  let id;
  try {
    const { value } = await requestFiles('upload', args, undefined, undefined, (transfer) => {
      id = transfer;
      started?.(transfer);
      if (progress) onTransfer(transfer, progress);
    });
    return value;
  } catch (error) {
    throw expoError(error, 'upload', args.uri);
  } finally {
    if (id != null) offTransfer(id);
  }
}

class FileSystemUploadTask extends SharedObject {
  async start(url, file, options) {
    const startedAt = (transfer) => {
      this.transfer = transfer;
    };
    try {
      return await upload(
        {
          uri: file.uri,
          url: String(url),
          method: options?.httpMethod ?? 'POST',
          headers: headersOf(options?.headers),
          uploadType: options?.uploadType ?? 0,
          fieldName: options?.fieldName ?? 'file',
          mimeType: options?.mimeType,
          parameters: options?.parameters,
        },
        (progress) => this.emit('progress', progress),
        startedAt,
      );
    } finally {
      this.transfer = null;
    }
  }

  cancel() {
    if (this.transfer != null) callFiles('cancelTransfer', { transfer: this.transfer });
  }

  release() {
    this.cancel();
  }
}

/** A download task's run, from the start or from `resume` bytes on. */
async function runTask(task, url, to, options, resume) {
  try {
    return await download(
      {
        url: String(url),
        ...destination(to),
        headers: headersOf(options?.headers),
        // A task's download writes over a file there, as Android's does.
        idempotent: true,
        ...(resume != null ? { resume } : {}),
      },
      (progress) => task.emit('progress', progress),
      (transfer) => {
        task.transfer = transfer;
      },
    );
  } finally {
    task.transfer = null;
  }
}

class FileSystemDownloadTask extends SharedObject {
  start(url, to, options) {
    return runTask(this, url, to, options, undefined);
  }

  pause() {
    if (this.transfer == null) return { resumeData: null };
    const { resumeData } = callFiles('pauseTransfer', { transfer: this.transfer }).value;
    return { resumeData: resumeData != null ? String(resumeData) : null };
  }

  resume(url, to, resumeData, options) {
    const offset = Number(resumeData);
    if (!Number.isInteger(offset) || offset < 0) {
      return Promise.reject(codedError('ERR_INVALID_RESUME_DATA', 'Invalid resume data provided'));
    }
    return runTask(this, url, to, options, offset);
  }

  async acknowledgeBackgroundCompletionAsync() {
    // OxyOS has no background sessions: nothing waits for an acknowledgment.
  }

  cancel() {
    if (this.transfer != null) callFiles('cancelTransfer', { transfer: this.transfer });
  }

  release() {
    this.cancel();
  }
}

/** The application's folders, asked once: URIs ending in `/`, or null. */
let folders;
function folder(name) {
  if (!filesAvailable) return null;
  folders ??= callFiles('paths').value;
  return folders[name];
}

/** A folder for `Paths`: there, or a clear reason it is not. */
function requiredFolder(name) {
  if (!filesAvailable) {
    throw codedError('ERR_FILE_SYSTEM_UNAVAILABLE', "expo-file-system: files are reached from an application's window on OxyOS");
  }
  const uri = folder(name);
  if (uri == null) {
    throw codedError(
      'ERR_FILE_SYSTEM_UNAVAILABLE',
      `expo-file-system: this window has no ${name}: the session did not start it as an application, so it has no folders of its own`,
    );
  }
  return uri;
}

const space = () => {
  try {
    return callFiles('diskSpace').value;
  } catch (error) {
    throw expoError(error, 'read', 'the file system');
  }
};

// The downloads `downloadFileAsync` started with a uuid, by it: their transfer, for cancelling.
const downloads = new Map();

const FileSystem = new NativeModule();
Object.defineProperties(FileSystem, {
  documentDirectory: { get: () => requiredFolder('documentDirectory'), enumerable: true },
  cacheDirectory: { get: () => requiredFolder('cacheDirectory'), enumerable: true },
  bundleDirectory: { get: () => requiredFolder('bundleDirectory'), enumerable: true },
  totalDiskSpace: { get: () => space().total, enumerable: true },
  availableDiskSpace: { get: () => space().available, enumerable: true },
});
Object.assign(FileSystem, {
  FileSystemFile,
  FileSystemDirectory,
  FileSystemFileHandle,
  FileSystemWatcher,
  FileSystemUploadTask,
  FileSystemDownloadTask,
  info(uri) {
    const found = facts(String(uri));
    return found?.exists ? { exists: true, isDirectory: !!found.directory } : { exists: false, isDirectory: null };
  },
  async downloadFileAsync(url, to, options, uuid) {
    try {
      return await download(
        { url: String(url), ...destination(to), headers: headersOf(options?.headers), idempotent: !!options?.idempotent },
        uuid ? (data) => FileSystem.emit('downloadProgress', { uuid, data }) : undefined,
        uuid ? (transfer) => downloads.set(uuid, transfer) : undefined,
      );
    } finally {
      if (uuid) downloads.delete(uuid);
    }
  },
  cancelDownloadAsync(uuid) {
    const transfer = downloads.get(uuid);
    if (transfer != null) callFiles('cancelTransfer', { transfer });
  },
  async canPreview() {
    return false;
  },
  async preview() {
    throw unsupported('File.preview', NO_PREVIEW);
  },
  async pickDirectoryAsync() {
    throw unsupported('Directory.pickDirectoryAsync', NO_PICKER);
  },
  async pickFileAsync() {
    // File.pickFileAsync answers any failure as a cancelled pick: the reason is said here too.
    console.warn(`expo-file-system: File.pickFileAsync is not available on OxyOS: ${NO_PICKER}; it answers as cancelled`);
    throw unsupported('File.pickFileAsync', NO_PICKER);
  },
});

// expo-file-system/legacy: its folders and its uploads (the Oxy SDK's native upload creates an
// upload task); the rest of it fails, naming the API to use instead.
const uploads = new Map();
const ExponentFileSystem = new NativeModule();
const legacyFolder = (name) => {
  try {
    return folder(name) ?? null;
  } catch {
    return null;
  }
};
Object.defineProperties(ExponentFileSystem, {
  documentDirectory: { get: () => legacyFolder('documentDirectory'), enumerable: true },
  cacheDirectory: { get: () => legacyFolder('cacheDirectory'), enumerable: true },
  bundleDirectory: { get: () => legacyFolder('bundleDirectory'), enumerable: true },
});
/** A legacy upload: its options as the legacy API gives them; a cancelled one resolves null. */
async function legacyUpload(url, fileUri, options, uuid) {
  const args = {
    uri: String(fileUri),
    url: String(url),
    method: String(options?.httpMethod ?? 'POST'),
    headers: headersOf(options?.headers),
    uploadType: options?.uploadType ?? 0,
    // Android's legacy upload names the field after the file unless told.
    fieldName: options?.fieldName ?? decodeURIComponent(String(fileUri).split('/').pop()),
    mimeType: options?.mimeType,
    parameters: options?.parameters,
  };
  try {
    const result = await upload(
      args,
      uuid
        ? ({ bytesSent, totalBytes }) =>
            ExponentFileSystem.emit('expo-file-system.uploadProgress', {
              uuid,
              data: { totalBytesSent: bytesSent, totalBytesExpectedToSend: totalBytes },
            })
        : undefined,
      uuid ? (transfer) => uploads.set(uuid, transfer) : undefined,
    );
    return { ...result, mimeType: null };
  } catch (error) {
    if (error.code === 'ERR_UPLOAD_CANCELLED') return null;
    throw error;
  } finally {
    if (uuid) uploads.delete(uuid);
  }
}
Object.assign(ExponentFileSystem, {
  uploadAsync: (url, fileUri, options) => legacyUpload(url, fileUri, options, undefined),
  uploadTaskStartAsync: (url, fileUri, uuid, options) => legacyUpload(url, fileUri, options, uuid),
  async networkTaskCancelAsync(uuid) {
    const transfer = uploads.get(uuid);
    if (transfer != null) callFiles('cancelTransfer', { transfer });
  },
});
for (const name of [
  'getInfoAsync',
  'readAsStringAsync',
  'writeAsStringAsync',
  'deleteAsync',
  'moveAsync',
  'copyAsync',
  'makeDirectoryAsync',
  'readDirectoryAsync',
  'downloadAsync',
  'downloadResumableStartAsync',
  'downloadResumablePauseAsync',
  'getContentUriAsync',
  'getFreeDiskStorageAsync',
  'getTotalDiskCapacityAsync',
  'requestDirectoryPermissionsAsync',
  'readSAFDirectoryAsync',
  'makeSAFDirectoryAsync',
  'createSAFFileAsync',
]) {
  ExponentFileSystem[name] = async () => {
    throw unsupported(
      `expo-file-system/legacy's ${name}`,
      "OxyOS gives the legacy API its folders and uploads; use expo-file-system's File, Directory and Paths",
    );
  };
}

Object.assign(globalThis.expo.modules, { FileSystem, ExponentFileSystem });
