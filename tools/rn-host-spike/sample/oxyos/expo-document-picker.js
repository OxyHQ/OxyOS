/**
 * expo-document-picker on OxyOS: the native module its JavaScript reaches (`ExpoDocumentPicker`,
 * `getDocumentAsync`), over the system's file chooser (file-chooser.js,
 * docs/adr/0029-chromium-on-oxyos.mdx, decision 8), as its Android implementation answers.
 *
 * The panel is the session's, a sheet of this window, asked for citing the person's latest press or
 * key. Each chosen file is an asset (its URI, name, size, media type and when it last changed),
 * copied first into the application's cache unless `copyToCacheDirectory` is false, as Expo copies
 * it on every platform; a cancellation, and the session's refusal (said), answer as cancelled.
 */
import { acceptedTypes, chooseFiles } from './file-chooser';
import { callFiles, requestFiles } from './files';

const { NativeModule } = globalThis.expo;

/** A URI's last name, decoded. */
const nameOf = (uri) => decodeURIComponent(uri.replace(/\/+$/, '').split('/').pop());

/** A name no other pick of this process has: a folder of its own in the cache for each copy. */
let picks = 0;
const uniqueFolder = () => `${Date.now().toString(36)}-${++picks}`;

/** The asset of the chosen `uri`: where it is (or its copy in the cache), and its facts. */
async function asset(uri, copyToCacheDirectory) {
  const name = nameOf(uri);
  let at = uri;
  const cache = copyToCacheDirectory ? callFiles('paths').value.cacheDirectory : null;
  if (cache) {
    const folder = `${cache}DocumentPicker/${uniqueFolder()}/`;
    callFiles('createDirectory', { uri: folder, intermediates: true, overwrite: false, idempotent: true });
    at = (await requestFiles('copy', { uri, directory: false, to: folder, toDirectory: true, overwrite: false })).value;
  }
  const facts = callFiles('stat', { uri: at }).value;
  return {
    uri: at,
    name,
    size: facts.size ?? undefined,
    mimeType: callFiles('mimeType', { uri: at }).value ?? undefined,
    lastModified: facts.modified ?? Date.now(),
  };
}

const ExpoDocumentPicker = new NativeModule();
Object.assign(ExpoDocumentPicker, {
  async getDocumentAsync({ type, copyToCacheDirectory = true, multiple = false } = {}) {
    const answer = await chooseFiles({ open: { multiple: multiple === true, types: acceptedTypes(type) } });
    if (!answer?.chosen) {
      if (answer?.refused) {
        console.warn(`expo-document-picker: OxyOS's file chooser refused (${answer.refused.reason}); it answers as cancelled`);
      }
      return { canceled: true, assets: null };
    }
    const assets = [];
    for (const uri of answer.chosen) assets.push(await asset(uri, copyToCacheDirectory !== false));
    return { canceled: false, assets };
  },
});

globalThis.expo.modules.ExpoDocumentPicker = ExpoDocumentPicker;
