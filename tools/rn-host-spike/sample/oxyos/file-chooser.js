/**
 * File choosers on OxyOS (docs/adr/0029-chromium-on-oxyos.mdx, decision 8), over the Bloom host's
 * `OxyOSFileChooser`: the open and save panels are the system's, as a Mac's are. An application asks
 * the session for files for its window (`chooseFiles`), citing the latest press or key; the session
 * starts the file manager in its picker mode as a sheet of that window, and the person's answer
 * comes back to this application alone: the chosen `file://` URIs, that they cancelled, or that the
 * session refused. The application never sees the panel.
 *
 * In the file manager the session started as the chooser, `chooserRequest` is what it was asked to
 * show and `answerChooser` gives the person's answer; Bloom's adapter offers them as
 * `PlatformFiles.chooser` (platform-surface.js).
 */
import { DeviceEventEmitter, TurboModuleRegistry } from 'react-native';

const native = TurboModuleRegistry.get('OxyOSFileChooser');

/** Requests waiting for their answer, by serial. */
const waiting = new Map();

if (native) {
  DeviceEventEmitter.addListener('OxyOSFilesChosen', ({ serial, answer }) => {
    const resolve = waiting.get(serial);
    if (!resolve) return;
    waiting.delete(serial);
    resolve(answer);
  });
}

/**
 * The accepted types an Expo picker names (`image/png`, `image/*`, `.pdf`, one or a list; the
 * wildcard of every type for any), as the session's vocabulary takes them: media types lowercased without their
 * parameters, extensions as they are; any type, none at all. A type the vocabulary has no words
 * for is left out rather than refusing the whole request.
 */
export function acceptedTypes(given) {
  const list = given == null ? [] : Array.isArray(given) ? given : [given];
  const types = [];
  for (const item of list) {
    if (typeof item !== 'string') continue;
    const type = item.startsWith('.') ? item : item.split(';')[0].trim().toLowerCase();
    if (type === '*/*' || type === '*') return [];
    const media = /^[a-z0-9!#$&^_.+-]+\/([a-z0-9!#$&^_.+-]+|\*)$/.test(type);
    const extension = /^(\.[A-Za-z0-9_+-]+)+$/.test(type);
    if ((media || extension) && type.length <= 128 && !types.includes(type)) types.push(type);
  }
  return types;
}

/** Whether this window can ask for files (the host has the module: an application's window). */
export const chooserAvailable = native != null;

/**
 * Asks the person to choose files for this window, in the session's vocabulary:
 * `{open: {multiple, directory, types, title, folder}}` or `{save: {name, types, title, folder}}`.
 * Resolves with the session's answer: `{chosen: [uri, ...]}`, `{cancelled: {}}` or
 * `{refused: {reason}}` (`unavailable` where the host cannot ask). `signal` withdraws it.
 */
export function chooseFiles(request, signal) {
  if (!native) return Promise.resolve({ refused: { reason: 'unavailable' } });
  return new Promise((resolve) => {
    const serial = native.choose(JSON.stringify(request));
    if (!serial) {
      resolve({ refused: { reason: 'unavailable' } });
      return;
    }
    waiting.set(serial, resolve);
    signal?.addEventListener('abort', () => native.cancel(serial), { once: true });
  });
}

/**
 * What the session asked this application to show as the file chooser (Bloom's
 * `PlatformFileChooserRequest`), or null in any other application.
 */
function readRequest() {
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
        ? Object.freeze({ id: request.application.id, name: request.application.name })
        : null,
    });
  } catch {
    console.error('OxyOS: the file chooser was asked to show what it cannot read');
    return null;
  }
}

export const chooserRequest = readRequest();

/** The chooser's answer: the person's choice (citing their latest press or key) or a cancellation. */
export function answerChooser(answer) {
  if (!chooserRequest) throw new Error('OxyOS: this application was not started to choose files');
  if (!native.answer(JSON.stringify(answer))) {
    console.error('OxyOS: the file chooser could not give its answer to the session');
  }
}
