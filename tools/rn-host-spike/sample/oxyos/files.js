/**
 * The host's file service (`OxyOSFiles`, files.h, docs/adr/0028-file-access.mdx) for
 * expo-file-system's module (expo-file-system.js) and the Bloom adapter's places and Trash
 * (platform-surface.js). Only an application's window has it: in the shell and the login screen
 * every operation fails, saying so.
 *
 * An operation is answered at once (`callFiles`, what expo-file-system does synchronously) or by a
 * promise (`requestFiles`); its value, or a `FilesError` with the service's code and reason. What
 * watches see and transfers' progress come as events, to the handler registered for their watch or
 * transfer.
 */
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

const native = TurboModuleRegistry.get('OxyOSFiles');

/** Whether this window reaches files. */
export const filesAvailable = native != null;

/** Why the service did not do something: its code (`refused`, `not-found`, `exists`...) and reason. */
export class FilesError extends Error {
  constructor(code, message) {
    super(message);
    this.name = 'FilesError';
    this.code = code;
  }
}

const unavailable = () =>
  new FilesError(
    'unavailable',
    "files are reached from an application's window on OxyOS, not from the shell or the login screen",
  );

/** The service's answer `{json, bytes | text}` as its value and content, or its error thrown. */
function settle(reply) {
  const answer = JSON.parse(reply.json);
  if (!answer.ok) throw new FilesError(answer.error, answer.message);
  return { value: answer.value, bytes: reply.bytes, text: reply.text };
}

/**
 * Runs `op` with `args` (and `content`: a string, an ArrayBuffer or a typed array) at once. With
 * `as` "text" its content comes as a string, else as a Uint8Array.
 */
export function callFiles(op, args = {}, content = undefined, as = undefined) {
  if (!native) throw unavailable();
  return settle(native.call(JSON.stringify({ ...args, op }), content, as));
}

let nextRequest = 0;

/** Runs `op` on the service's thread; its id (a transfer's) is given to `started` first. */
export async function requestFiles(op, args = {}, content = undefined, as = undefined, started = undefined) {
  if (!native) throw unavailable();
  const id = ++nextRequest;
  started?.(id);
  return settle(await native.request(id, JSON.stringify({ ...args, op }), content, as));
}

// Handlers of events, by watch and by transfer.
const watches = new Map();
const transfers = new Map();

if (native) {
  DeviceEventEmitter.addListener('OxyOSFilesEvent', (json) => {
    const event = JSON.parse(json);
    if (event.watch != null) {
      watches.get(event.watch)?.(event);
      if (event.ended) watches.delete(event.watch);
    } else if (event.transfer != null) {
      transfers.get(event.transfer)?.(event.progress);
    }
  });
}

/** Calls `handler` with each event of watch `id` until `unwatchFiles(id)` or its end. */
export function onWatch(id, handler) {
  watches.set(id, handler);
}

/** Ends watch `id`. */
export function unwatchFiles(id) {
  watches.delete(id);
  if (native) callFiles('unwatch', { watch: id });
}

/** Calls `handler` with transfer `id`'s progress until `offTransfer(id)`. */
export function onTransfer(id, handler) {
  transfers.set(id, handler);
}

export function offTransfer(id) {
  transfers.delete(id);
}
