/**
 * The OxyOS platform adapter for Bloom (`BloomProvider platform={oxyosSurfaces}`): Bloom's popups
 * become compositor popups, which may extend beyond the application window, and an application's
 * window is Bloom's `window` (docs/adr/0016-windows.mdx): Bloom draws its controls in the
 * platform's place, and they and its drag regions act through the compositor.
 *
 * `PopupSurface` renders the `OxyOSPopupView` host component. Its subtree stays in this React tree;
 * the native host draws it in a popup surface of its own, placed by the compositor from the
 * anchor's frame and Bloom's placement (side, align, offsets). The compositor's dismissal comes
 * back as `onDismiss`. A popup anchored inside another (a submenu beside its row) is that popup's
 * child. It reports `pointer` everywhere, the shell included: Bloom's controls take a desktop's
 * geometry and its menus a desktop's presentation (submenus flying out beside their rows).
 *
 * An application's window also has the `menu-bar` capability (docs/adr/0021-menu-bar.mdx): Bloom
 * gives `menuBar.setMenus` its menus whenever they change and the session's shell shows them in its
 * bar while the window is active; a command chosen there comes back through `onActivate` with the
 * id Bloom gave it. Its `keyboard` offers Bloom each of the window's key presses before the
 * focused view (`OxyOSKeyboard`), so a menu's shortcut runs its item; a key Bloom takes goes no
 * further. Its `editing` is the window's first responder for the Edit menu's standard actions,
 * the focused text input (`OxyOSEditing`). Its `files` are the person's places and the Trash (docs/adr/0028-file-access.mdx), what a
 * file manager needs beyond expo-file-system, and `files.tags` the tags on the person's items
 * (docs/adr/0049-file-tags.mdx), and `files.backups` their backups' versions and restoring
 * (docs/adr/0059-backups.mdx). Its `search` is the session's search index
 * (docs/adr/0051-search.mdx), Bloom's `PlatformSearch`, in every window, the shell's included.
 *
 * In the file manager the session started as a file chooser (docs/adr/0029-chromium-on-oxyos.mdx,
 * decision 8), `files.chooser` is what it was asked to show and how it answers; its window is the
 * asking window's sheet, so it is not Bloom's `window` (no controls of its own) and has no menus of
 * its own (the asking window's stand).
 */
import * as React from 'react';
import { DeviceEventEmitter, findNodeHandle, TurboModuleRegistry } from 'react-native';
import * as NativeComponentRegistry from 'react-native/Libraries/NativeComponent/NativeComponentRegistry';
import { homeName } from './home-name';
import {
  appearance as platformAppearance,
  chooser as platformChooser,
  drag as platformDrag,
  files as platformFiles,
  isPlatformError,
  search as platformSearch,
  sounds as platformSounds,
  windows as platformWindows,
} from '@oxyos/platform';

// The file manager the session started as a file chooser: what it was asked to show (the platform's
// `chooser.request`), null in any other application.
const chooserRequest = platformChooser.request;
const answerLost = () => console.error('OxyOS: the file chooser could not give its answer to the session');

const OxyOSPopupView = NativeComponentRegistry.get('OxyOSPopupView', () => ({
  uiViewClassName: 'OxyOSPopupView',
  validAttributes: {
    anchorTag: true,
    side: true,
    align: true,
    sideOffset: true,
    alignOffset: true,
    hasPoint: true,
    pointX: true,
    pointY: true,
    tooltip: true,
    trackingTag: true,
  },
  directEventTypes: {
    topDismiss: { registrationName: 'onDismiss' },
    topKey: { registrationName: 'onKey' },
  },
}));

function PopupSurface({
  anchorRef,
  point,
  trackingRef,
  side,
  align,
  sideOffset,
  alignOffset,
  role,
  label,
  onDismiss,
  onKey,
  testID,
  children,
}) {
  // The anchor is a mounted view; its tag is known once the trigger has committed.
  const [anchorTag, setAnchorTag] = React.useState(null);
  React.useLayoutEffect(() => {
    setAnchorTag(anchorRef.current ? findNodeHandle(anchorRef.current) : null);
  }, [anchorRef]);
  // The view that goes on taking the pointer while the popup is open (a menu bar, Bloom's
  // `trackingRef`): the host gives it the pointer's movement over it, and nothing else outside the
  // popup.
  const [trackingTag, setTrackingTag] = React.useState(-1);
  React.useLayoutEffect(() => {
    setTrackingTag(trackingRef?.current ? findNodeHandle(trackingRef.current) ?? -1 : -1);
  }, [trackingRef]);
  if (anchorTag == null) return null;
  return (
    <OxyOSPopupView
      anchorTag={anchorTag}
      side={side}
      align={align}
      sideOffset={sideOffset}
      alignOffset={alignOffset}
      // A context menu's press (Bloom's `point`): the popup is placed against that point of the
      // anchor rather than its frame.
      hasPoint={point != null}
      pointX={point?.x ?? 0}
      pointY={point?.y ?? 0}
      // A tooltip (Bloom's native Tooltip) is a hint: shown without a grab, taking no input, its
      // text read as the bubble's own (docs/adr/0034-the-dock.mdx).
      tooltip={role === 'tooltip'}
      trackingTag={trackingTag}
      accessibilityRole={role === 'tooltip' ? undefined : role}
      accessibilityLabel={label}
      onDismiss={(event) => onDismiss(event.nativeEvent.reason)}
      // Keys the compositor sends to the popup while it has keyboard focus, as W3C key names.
      onKey={onKey ? (event) => onKey(event.nativeEvent.key) : undefined}
      testID={testID}
      collapsable={false}
      // Out of the window's layout: the panel sizes itself, the compositor places it.
      style={{ position: 'absolute', left: 0, top: 0 }}
    >
      {children}
    </OxyOSPopupView>
  );
}

// The application's own window, the platform's `windows`; not the shell's (it has none), nor a host
// drawing offscreen. Bloom's `PlatformWindow`: its state as a store (`getState`, `subscribe`).
function createWindow() {
  const { state } = platformWindows;
  return {
    controls: 'bloom',
    metrics: platformWindows.metrics,
    getState: () => state.current(),
    subscribe: (listener) => state.subscribe(() => listener()),
    close: () => platformWindows.close(),
    minimize: () => platformWindows.minimize(),
    zoom: () => platformWindows.zoom(),
    // While a press is being handled: the compositor moves the window until its buttons are up.
    startMove: () => {
      platformWindows.startMove();
    },
    // The sizes the application's layout holds together at (`SetSizeLimits`): the compositor never
    // configures the window outside them, by its edges or by zoom.
    setSizeLimits: (limits) => platformWindows.setSizeLimits(limits),
  };
}

const window = platformWindows.available && !chooserRequest ? createWindow() : undefined;

// The protocol's kinds and flags, and its limits: menus past them are not sent (the compositor
// would refuse the window's menus whole).
const MENU = { menu: 1, item: 2, submenu: 3, separator: 4 };
const DISABLED = 1;
const CHECKABLE = 2;
const CHECKED = 4;
// The application's own standard commands (Bloom's `MenubarItem action`): its About and its
// Settings…, which the shell shows in the application's own menu (docs/adr/0021-menu-bar.mdx).
const ROLE = new Map([
  ['about', 1],
  ['settings', 2],
]);
const MAX_ENTRIES = 256;
const MAX_DEPTH = 4;
// How many commits' ids are kept: a choice made just before Bloom described the menus again still
// names an item by the id it had then, which Bloom keeps while the item is mounted.
const COMMITS_KEPT = 8;

/** Bloom's menus as the protocol's entries, in order, and each entry's id. */
function flatten(menus) {
  const entries = [];
  const ids = [];
  // Each role once in a commit, on a command without a check: the first declared has it.
  const roles = new Set();
  const add = (entry, id) => {
    if (entries.length >= MAX_ENTRIES) return -1;
    entries.push(entry);
    ids.push(id);
    return entries.length - 1;
  };
  const walk = (items, parent, depth) => {
    for (const item of items) {
      if (item.kind === 'separator') {
        add({ parent, kind: MENU.separator, flags: 0, label: '', shortcut: '' }, null);
        continue;
      }
      // Every entry but a separator is named; one with no label cannot be shown.
      if (!item.label) continue;
      const disabled = item.enabled === false ? DISABLED : 0;
      if (item.kind === 'submenu') {
        if (depth >= MAX_DEPTH) continue;
        const index = add({ parent, kind: MENU.submenu, flags: disabled, label: item.label, shortcut: '' }, item.id);
        if (index >= 0) walk(item.items ?? [], index, depth + 1);
        continue;
      }
      const check = typeof item.checked === 'boolean' ? CHECKABLE | (item.checked ? CHECKED : 0) : 0;
      const role = check === 0 && !roles.has(item.action) ? (ROLE.get(item.action) ?? 0) : 0;
      if (role) roles.add(item.action);
      add(
        { parent, kind: MENU.item, flags: disabled | check, role, label: item.label, shortcut: item.shortcut ?? '' },
        item.id,
      );
    }
  };
  for (const menu of menus) {
    if (!menu.label) continue;
    const index = add({ parent: -1, kind: MENU.menu, flags: 0, label: menu.label, shortcut: '' }, menu.id);
    if (index >= 0) walk(menu.items ?? [], index, 2);
  }
  return { entries, ids };
}

// Bloom's menus in the shell's bar, over the platform's `windows.menus`: the protocol's entries, and
// each commit's ids by index (the compositor and the shell see indices alone), newest last.
function createMenuBar() {
  const commits = [];
  let last = null;
  return {
    setMenus(menus) {
      const { entries, ids } = flatten(menus ?? []);
      // The same menus again are not sent again.
      const described = JSON.stringify([entries, ids]);
      if (described === last) return;
      last = described;
      const serial = platformWindows.menus.set(entries);
      if (!serial) return;
      commits.push({ serial, ids });
      if (commits.length > COMMITS_KEPT) commits.shift();
    },
    onActivate(listener) {
      return platformWindows.menus.onChoose(({ serial, index }) => {
        const id = commits.find((kept) => kept.serial === serial)?.ids[index];
        if (id != null) listener(id);
      });
    },
  };
}

const menuBar = platformWindows.menus.available && !chooserRequest ? createMenuBar() : undefined;

// The window's keys before the focused view (the platform's `windows.keys`): a listener answering
// true takes the key; and the modifiers held as they change (protocol version 38, a Mac's
// flagsChanged:, docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx): `{altKey, ctrlKey,
// metaKey, shiftKey}`.
const keyboard = platformWindows.keys.available
  ? {
      subscribe: (listener) => platformWindows.keys.onKey(listener),
      subscribeModifiers: (listener) => platformWindows.keys.onModifiers(listener),
    }
  : undefined;

// The window's first responder for the Edit menu's standard actions (docs/adr/0021-menu-bar.mdx,
// decision 3): the focused text input, whatever component draws it, as the host knows it. Bloom's
// `MenubarItem action="copy"` is enabled exactly when it can copy and copies there, as a Mac's
// Edit menu sends copy: to the first responder. The platform's `windows.editing`.
function createEditing() {
  const { state } = platformWindows.editing;
  return {
    getState: () => state.current() ?? null,
    subscribe: (listener) => state.subscribe(() => listener()),
    perform: (action) => platformWindows.editing.perform(action),
  };
}

// The file chooser's sheet has no menus of its own (the asking window's stand), so no Edit menu.
const editing = platformWindows.editing.available && !chooserRequest ? createEditing() : undefined;

// The session's appearance (docs/adr/0019-settings.mdx) beyond light and dark, which React Native's
// own Appearance carries: the accent colour, which seeds Bloom's palette, and reduce transparency,
// the platform's `appearance.drawing`. Only with a connection to the compositor, which says them.
const appearance = platformAppearance.drawing.current()
  ? {
      getState: () => platformAppearance.drawing.current(),
      subscribe: (listener) => platformAppearance.drawing.subscribe(() => listener()),
    }
  : undefined;

// The person's places and the Trash (Bloom's `PlatformFiles`, docs/adr/0028-file-access.mdx),
// through the host's file service: the home, its standard folders that exist and the volumes
// mounted for the user, and the freedesktop.org Trash every file manager on the machine shares.
// Every URI is file://, as expo-file-system takes them. Only an application's window has it.
// The platform's words for why an item was not done, as Bloom's `PlatformFileError` says them: the
// person's files not granted to the application is a denial.
const BLOOM_FILE_ERRORS = { notFound: 'not-found', exists: 'exists', refused: 'refused', denied: 'denied', notGranted: 'denied' };

/** What came of one item, as Bloom's `PlatformFileOutcome` says it. */
const outcome = (item) =>
  item.ok
    ? { uri: item.uri, ok: true, result: item.result }
    : { uri: item.uri, ok: false, error: BLOOM_FILE_ERRORS[item.reason] ?? 'io' };

/** A folder's own name, from its URI. */
const named = (uri) => decodeURIComponent(uri.replace(/\/+$/, '').split('/').pop());

// Bloom's `PlatformFiles`, over the platform's `files` (`@oxyos/platform`).
function createFiles() {
  const outcomes = async (done) => (await done).map(outcome);
  return {
    async places() {
      const { home, favourites, volumes } = await platformFiles.places();
      return [
        { kind: 'home', name: homeName(home), uri: home },
        ...favourites.map(({ key, uri }) => ({ kind: key, name: named(uri), uri })),
        ...volumes.map(({ name, uri }) => ({ kind: 'volume', name, uri })),
      ];
    },
    watchPlaces: (listener) => platformFiles.onPlacesChange(listener),
    trash: (uris) => outcomes(platformFiles.trash(uris)),
    trashed: () => platformFiles.trashed(),
    watchTrash: (listener) => platformFiles.onTrashChange(listener),
    restore: (uris) => outcomes(platformFiles.restore(uris)),
    erase: (uris) => outcomes(platformFiles.erase(uris)),
    async emptyTrash() {
      await platformFiles.emptyTrash();
    },
    // Bloom's `PlatformFileTags` (docs/adr/0049-file-tags.mdx): the items' own tags, in their
    // extended attributes as freedesktop.org and a Mac keep them, and the index of every tagged
    // item in the person's places.
    tags: Object.freeze({
      inFolder: (folderUri) => platformFiles.tags.in(folderUri),
      ofItems: (uris) => platformFiles.tags.of(uris),
      change: (uris, change) => outcomes(platformFiles.tags.change(uris, change)),
      tagged: (name) => platformFiles.tags.tagged(name),
      names: () => platformFiles.tags.names(),
      watch: (listener) => platformFiles.tags.onChange(listener),
    }),
    // Bloom's `PlatformFileBackups` (docs/adr/0059-backups.mdx): the person's backups, asked of
    // the backup service from this process as the person; only for an application granted
    // `files`.
    backups: platformFiles.backups.available ? createBackups() : undefined,
    // Bloom's `PlatformFileChooser`: present only in a file chooser the session started. The
    // session ends this process once it has the answer.
    chooser: chooserRequest
      ? Object.freeze({
          request: chooserRequest,
          choose(uris) {
            platformChooser.choose(uris).catch(answerLost);
          },
          cancel() {
            platformChooser.cancel().catch(answerLost);
          },
        })
      : undefined,
  };
}

// The codes Bloom's `PlatformBackupError` names; another of the service's is `io`.
const BACKUP_ERRORS = new Set(['not-found', 'exists', 'refused', 'denied', 'io', 'unavailable', 'not-permitted', 'cancelled']);

/** The service's word (`notPermitted`, the PlatformError's reason) as it says it (`not-permitted`). */
const serviceCode = (reason) => reason.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);

/** The platform's error as Bloom's: an Error with a `PlatformBackupError` code. */
function backupError(error) {
  if (!isPlatformError(error)) return error;
  const code = error.code === 'cancelled' ? 'cancelled' : serviceCode(error.reason ?? error.code);
  const said = new Error(error.details?.message ?? error.message);
  said.code = BACKUP_ERRORS.has(code) ? code : 'io';
  return said;
}

// Bloom's `PlatformFileBackups` (docs/adr/0059-backups.mdx), over the platform's `files.backups`.
function createBackups() {
  const backups = platformFiles.backups;
  const asked = async (done) => {
    try {
      return await done;
    } catch (error) {
      throw backupError(error);
    }
  };
  return Object.freeze({
    state: () => asked(backups.state()),
    watch: (listener) => backups.onChange(listener),
    versions: (uri) => asked(backups.versions(uri)),
    contents: (backup, folderUri) => asked(backups.contents(backup, folderUri)),
    // Written by this process as the person, beside what is there unless `replace`; its progress
    // as a transfer's, and cancelled as one when `signal` aborts.
    restore: (backup, uri, { to, replace = false, onProgress, signal } = {}) =>
      asked(backups.restore(backup, uri, { to, replace, onProgress, signal })),
  });
}

const files = platformFiles.available ? createFiles() : undefined;

// Bloom's `PlatformSearch` (docs/adr/0051-search.mdx): the session's search index, which every
// window reaches (protocol version 30), the shell's and Settings' included.
// The platform's `search` (`@oxyos/platform`), in Bloom's words.
const search = platformSearch.available
  ? Object.freeze({
      query: (query) => platformSearch.query(query),
      watch: (listener) => platformSearch.onChange(listener),
      used: (id) => platformSearch.used(id),
      index: Object.freeze({
        donate: (items) => platformSearch.donate(items),
        remove: (ids) => platformSearch.remove(ids),
        removeAll: () => platformSearch.removeAll(),
      }),
      preferences: () => platformSearch.preferences(),
      setPreferences: (preferences) => platformSearch.setPreferences(preferences),
      status: () => platformSearch.status(),
      state: platformSearch.state,
      rebuild: () => platformSearch.rebuild(),
      // Settings › Storage (docs/adr/0062-storage.mdx; Settings alone).
      storage: () => platformSearch.storage(),
      largest: (options) => platformSearch.largest(options),
    })
  : undefined;

// Drag and drop between applications (docs/adr/0035-drag-and-drop.mdx): the drop target the host
// answers the compositor for from its props (drag.h), and the application's own drags, the
// platform's `drag` (`@oxyos/platform`). The protocol's version 10 has it; an older compositor
// none, and Bloom's DropZone is then a view that takes nothing.
const dragAvailable = platformDrag.available;

const OxyOSDropTargetView = NativeComponentRegistry.get('OxyOSDropTargetView', () => ({
  uiViewClassName: 'OxyOSDropTargetView',
  validAttributes: { acceptKinds: true, dropActions: true, disabled: true, acceptFileTypes: true, dropFolder: true },
  directEventTypes: {
    topDragEnter: { registrationName: 'onNativeDragEnter' },
    topDragOver: { registrationName: 'onNativeDragOver' },
    topDragLeave: { registrationName: 'onNativeDragLeave' },
    topDrop: { registrationName: 'onNativeDrop' },
  },
}));

/** The URIs of a `text/uri-list`: one a line, `#` lines comments. */
function uriList(text) {
  return (text ?? '')
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith('#'));
}

/** A `file://` URI's file, named by its path's last part. */
function fileOfUri(uri) {
  const path = uri.replace(/^file:\/\/[^/]*/, '').replace(/\/+$/, '');
  const last = path.slice(path.lastIndexOf('/') + 1);
  let name = last;
  try {
    name = decodeURIComponent(last);
  } catch {
    // A malformed escape keeps the name as written.
  }
  return { name, uri };
}

const KIND_TYPES = {
  files: ['text/uri-list'],
  url: ['text/uri-list', 'text/x-moz-url'],
  text: ['text/plain;charset=utf-8', 'text/plain'],
  html: ['text/html'],
};

/** Bloom's DropData of what each type the host received holds, of the kinds the target takes. */
function dropData(accept, received) {
  const uris = uriList(received['text/uri-list'] ?? (received['text/x-moz-url'] ?? '').split('\n')[0]);
  const fileUris = uris.filter((uri) => uri.toLowerCase().startsWith('file:'));
  const urls = uris.filter((uri) => !uri.toLowerCase().startsWith('file:'));
  const text = received['text/plain;charset=utf-8'] ?? received['text/plain'];
  const html = received['text/html'];
  const other = {};
  for (const kind of accept) {
    if (kind in KIND_TYPES) continue;
    if (received[kind] !== undefined) other[kind] = received[kind];
  }
  return {
    files: accept.includes('files') ? fileUris.map(fileOfUri) : [],
    urls: accept.includes('url') ? urls : [],
    ...(accept.includes('text') && text !== undefined ? { text } : {}),
    ...(accept.includes('html') && html !== undefined ? { html } : {}),
    other,
  };
}

function dragEvent(nativeEvent) {
  return {
    x: nativeEvent.x,
    y: nativeEvent.y,
    action: nativeEvent.action ?? null,
    kinds: nativeEvent.kinds ?? [],
    // The types of the files it carries, as its source declared them (protocol version 26).
    fileTypes: nativeEvent.fileTypes ?? [],
  };
}

function DropTarget({
  accept,
  actions,
  acceptFileTypes,
  folder,
  disabled,
  onDragEnter,
  onDragOver,
  onDragLeave,
  onDrop,
  style,
  testID,
  children,
}) {
  return (
    <OxyOSDropTargetView
      acceptKinds={accept.join('\n')}
      dropActions={actions.join('\n')}
      // Files whose types it does not take are no files to it: the host answers the compositor so.
      acceptFileTypes={(acceptFileTypes ?? []).join('\n')}
      // Where files dropped on it go (Bloom's DropZone `folder`, protocol version 28): the host
      // prefers a move when every dragged file is on this folder's volume and a copy when one is
      // not, as Finder does, if the target allows both.
      dropFolder={folder ?? ''}
      disabled={Boolean(disabled)}
      style={style}
      testID={testID}
      onNativeDragEnter={(event) => onDragEnter?.(dragEvent(event.nativeEvent))}
      onNativeDragOver={(event) => onDragOver?.(dragEvent(event.nativeEvent))}
      onNativeDragLeave={() => onDragLeave?.()}
      onNativeDrop={(event) => {
        const { nativeEvent } = event;
        if (!nativeEvent.action) return;
        onDrop({
          ...dragEvent(nativeEvent),
          action: nativeEvent.action,
          data: dropData(accept, nativeEvent.received ?? {}),
        });
      }}
    >
      {children}
    </OxyOSDropTargetView>
  );
}

// Bloom's drag over the platform's: a drag that ended without a drop tells `onCancel` why, Bloom's
// way (`not-accepted`; `refused` for one with no press to start from), before it resolves with
// null. The window's drags (Bloom's useWindowDrag) are the platform's `onWindowDrag`.
const BLOOM_END_REASONS = { escape: 'escape', notAccepted: 'not-accepted', cancelled: 'cancelled' };
const drag = dragAvailable
  ? {
      // The dragged view itself is the image, as it looks (stacked and badged for several items).
      // `data.other` is offered by its media types after the kinds; `vanish` (protocol version 26)
      // has the image vanish in a poof where it is dropped on nothing, an item dragged out of the
      // dock.
      async start({ data, actions, previewRef, count, vanish, onCancel }) {
        const preview = previewRef?.current ? findNodeHandle(previewRef.current) : null;
        try {
          const outcome = await platformDrag.start({ data, actions: [...actions], preview, count, vanish });
          if (outcome.action === null) onCancel?.(BLOOM_END_REASONS[outcome.reason]);
          return outcome.action;
        } catch {
          // Refused (no press held), offering nothing, or no window: Bloom's `refused`.
          onCancel?.('refused');
          return null;
        }
      },
      watchWindow: (listener) => platformDrag.onWindowDrag(listener),
    }
  : undefined;

// The system's own sounds (docs/adr/0044-system-sounds.mdx, Bloom's usePlatformSounds): an
// application asks by meaning and the session plays its sound theme's, as the person's settings
// and the output allow, citing the latest input it received. The protocol's version 23 has them.
// The platform's `sounds` (`@oxyos/platform`); a sound not played (no input to cite) is silence.
const sounds = platformSounds.available
  ? {
      play(sound) {
        platformSounds.play(String(sound)).catch(() => {});
      },
    }
  : undefined;

// The platform's material (docs/adr/0069-the-platforms-material.mdx, Bloom's `material`): the
// OxyOSMaterialView host component (material.h), which Bloom mounts under a floating surface's tint.
// The host fills it with what its surface drew beneath it, blurred and saturated, and tells the
// compositor its frame and corners, where the compositor blurs the windows and the desktop beneath
// the surface. A compositor of protocol version 45 or later takes them (OxyOSMaterial's
// `available`); on an older one there is no `material`, and Bloom draws its opaque fills.
const materialAvailable = Boolean(TurboModuleRegistry.get('OxyOSMaterial')?.getConstants?.()?.available);

const OxyOSMaterialView = NativeComponentRegistry.get('OxyOSMaterialView', () => ({
  uiViewClassName: 'OxyOSMaterialView',
  validAttributes: {},
}));

// Bloom's PlatformMaterialProps: its frame and shape (`style`) and Bloom's theme (`appearance`), for
// which nothing differs here: the blur is the same in light and dark, and the tint over it Bloom's.
function Material({ style, testID }) {
  return (
    // Never flattened away: the host finds the material's frame by its view. It takes no input.
    <OxyOSMaterialView style={style} testID={testID} pointerEvents="none" collapsable={false} />
  );
}

export const oxyosSurfaces = {
  platform: 'oxyos',
  capabilities: new Set([
    'popup-surface',
    // OxyOS is a desktop a pointer drives, the shell's bar and dock as much as a window: Bloom's
    // controls take a desktop's geometry and its menus a desktop's presentation everywhere.
    'pointer',
    ...(window ? ['window'] : []),
    ...(menuBar ? ['menu-bar'] : []),
    ...(appearance ? ['appearance'] : []),
    ...(drag ? ['drag-and-drop'] : []),
    ...(materialAvailable ? ['material'] : []),
  ]),
  PopupSurface,
  window,
  menuBar,
  keyboard,
  editing,
  appearance,
  files,
  search,
  sounds,
  ...(drag ? { DropTarget, drag } : {}),
  ...(materialAvailable ? { Material } : {}),
};
