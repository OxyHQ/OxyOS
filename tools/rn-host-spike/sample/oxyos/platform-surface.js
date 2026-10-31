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
 * further. Its `files` are the person's places and the Trash (docs/adr/0028-file-access.mdx), what a
 * file manager needs beyond expo-file-system.
 *
 * In the file manager the session started as a file chooser (docs/adr/0029-chromium-on-oxyos.mdx,
 * decision 8), `files.chooser` is what it was asked to show and how it answers; its window is the
 * asking window's sheet, so it is not Bloom's `window` (no controls of its own) and has no menus of
 * its own (the asking window's stand).
 */
import * as React from 'react';
import { DeviceEventEmitter, findNodeHandle, TurboModuleRegistry } from 'react-native';
import * as NativeComponentRegistry from 'react-native/Libraries/NativeComponent/NativeComponentRegistry';
import { answerChooser, chooserRequest } from './file-chooser';
import { filesAvailable, onWatch, requestFiles, unwatchFiles } from './files';

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
  },
  directEventTypes: {
    topDismiss: { registrationName: 'onDismiss' },
    topKey: { registrationName: 'onKey' },
  },
}));

function PopupSurface({
  anchorRef,
  point,
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
      accessibilityRole={role}
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

// The application's own window; not the shell's (it has none), nor a host drawing offscreen.
const nativeWindow = TurboModuleRegistry.get('OxyOSWindow');
const windowConstants = nativeWindow?.getConstants();

function createWindow() {
  // One state object per change, so a store snapshot changes only when the window does.
  let state = nativeWindow.getState();
  const listeners = new Set();
  DeviceEventEmitter.addListener('OxyOSWindowChanged', (next) => {
    state = next;
    for (const listener of listeners) listener();
  });
  return {
    controls: 'bloom',
    metrics: windowConstants.metrics,
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    close: () => nativeWindow.close(),
    minimize: () => nativeWindow.minimize(),
    zoom: () => nativeWindow.zoom(),
    // While a press is being handled: the compositor moves the window until its buttons are up.
    startMove: () => {
      nativeWindow.startMove();
    },
    // The sizes the application's layout holds together at (`SetSizeLimits`): the compositor never
    // configures the window outside them, by its edges or by zoom.
    setSizeLimits: ({ minWidth, minHeight, maxWidth, maxHeight }) => {
      nativeWindow.setSizeLimits(minWidth, minHeight, maxWidth, maxHeight);
    },
  };
}

const window = windowConstants?.available && !chooserRequest ? createWindow() : undefined;

// The protocol's kinds and flags, and its limits: menus past them are not sent (the compositor
// would refuse the window's menus whole).
const MENU = { menu: 1, item: 2, submenu: 3, separator: 4 };
const DISABLED = 1;
const CHECKABLE = 2;
const CHECKED = 4;
const MAX_ENTRIES = 256;
const MAX_DEPTH = 4;
// How many commits' ids are kept: a choice made just before Bloom described the menus again still
// names an item by the id it had then, which Bloom keeps while the item is mounted.
const COMMITS_KEPT = 8;

/** Bloom's menus as the protocol's entries, in order, and each entry's id. */
function flatten(menus) {
  const entries = [];
  const ids = [];
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
      add({ parent, kind: MENU.item, flags: disabled | check, label: item.label, shortcut: item.shortcut ?? '' }, item.id);
    }
  };
  for (const menu of menus) {
    if (!menu.label) continue;
    const index = add({ parent: -1, kind: MENU.menu, flags: 0, label: menu.label, shortcut: '' }, menu.id);
    if (index >= 0) walk(menu.items ?? [], index, 2);
  }
  return { entries, ids };
}

const nativeMenuBar = TurboModuleRegistry.get('OxyOSMenuBar');

function createMenuBar() {
  // Each commit's ids by index (the compositor and the shell see indices alone), newest last.
  const commits = [];
  let last = null;
  const listeners = new Set();
  DeviceEventEmitter.addListener('OxyOSMenuActivated', ({ serial, index }) => {
    const commit = commits.find((kept) => kept.serial === serial);
    const id = commit?.ids[index];
    if (id == null) return;
    for (const listener of [...listeners]) listener(id);
  });
  return {
    setMenus(menus) {
      const { entries, ids } = flatten(menus ?? []);
      // The same menus again are not sent again.
      const described = JSON.stringify([entries, ids]);
      if (described === last) return;
      last = described;
      const serial = nativeMenuBar.setMenus(entries);
      if (!serial) return;
      commits.push({ serial, ids });
      if (commits.length > COMMITS_KEPT) commits.shift();
    },
    onActivate(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

const menuBar = nativeMenuBar?.getConstants()?.available && !chooserRequest ? createMenuBar() : undefined;

const nativeKeyboard = TurboModuleRegistry.get('OxyOSKeyboard');

function createKeyboard() {
  const listeners = new Set();
  // The host calls this with each key press before the focused view, and waits for the answer.
  nativeKeyboard.setListener((event) => [...listeners].some((listener) => listener(event) === true));
  return {
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

const keyboard = nativeKeyboard?.getConstants()?.available ? createKeyboard() : undefined;

// The session's appearance (docs/adr/0019-settings.mdx) beyond light and dark, which React Native's
// own Appearance carries: the accent colour, which seeds Bloom's palette, and reduce transparency.
// Only with a connection to the compositor, whose first events say them.
const nativePreferences = TurboModuleRegistry.get('OxyOSPreferences');

function appearanceOf(drawing) {
  return { accent: drawing.accent || null, reduceTransparency: !!drawing.reduceTransparency };
}

function createAppearance() {
  const drawing = nativePreferences?.getState();
  if (!drawing) return undefined;
  // One state object per change, so a store snapshot changes only when the appearance does.
  let state = appearanceOf(drawing);
  const listeners = new Set();
  DeviceEventEmitter.addListener('OxyOSPreferencesChanged', (next) => {
    state = appearanceOf(next);
    for (const listener of listeners) listener();
  });
  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

const appearance = createAppearance();

// The person's places and the Trash (Bloom's `PlatformFiles`, docs/adr/0028-file-access.mdx),
// through the host's file service: the home, its standard folders that exist and the volumes
// mounted for the user, and the freedesktop.org Trash every file manager on the machine shares.
// Every URI is file://, as expo-file-system takes them. Only an application's window has it.
const PLATFORM_ERRORS = new Set(['not-found', 'exists', 'refused', 'denied']);

/** What came of one item, as Bloom's `PlatformFileOutcome` says it. */
const outcome = (item) =>
  item.ok
    ? { uri: item.uri, ok: true, result: item.result }
    : { uri: item.uri, ok: false, error: PLATFORM_ERRORS.has(item.error) ? item.error : 'io' };

/** A folder's own name, from its URI. */
const named = (uri) => decodeURIComponent(uri.replace(/\/+$/, '').split('/').pop());

function createFiles() {
  // A listener's own watch, ended when it unsubscribes.
  const watching = (op) => (listener) => {
    let watch = null;
    let ended = false;
    requestFiles(op).then(
      ({ value }) => {
        if (ended) return unwatchFiles(value);
        watch = value;
        onWatch(value, () => listener());
      },
      (error) => console.error(`OxyOS could not watch (${op}): ${error.message}`),
    );
    return () => {
      ended = true;
      if (watch != null) unwatchFiles(watch);
    };
  };
  const each = async (op, uris) => (await requestFiles(op, { uris: [...uris] })).value.map(outcome);
  return {
    async places() {
      const { home, favourites, volumes } = (await requestFiles('places')).value;
      return [
        { kind: 'home', name: named(home), uri: home },
        ...favourites.map(({ key, uri }) => ({ kind: key, name: named(uri), uri })),
        ...volumes.map(({ name, uri }) => ({ kind: 'volume', name, uri })),
      ];
    },
    watchPlaces: watching('watchPlaces'),
    trash: (uris) => each('trash', uris),
    async trashed() {
      return (await requestFiles('trashed')).value;
    },
    watchTrash: watching('watchTrash'),
    restore: (uris) => each('restore', uris),
    erase: (uris) => each('erase', uris),
    async emptyTrash() {
      await requestFiles('emptyTrash');
    },
    // Bloom's `PlatformFileChooser`: present only in a file chooser the session started. The
    // session ends this process once it has the answer.
    chooser: chooserRequest
      ? Object.freeze({
          request: chooserRequest,
          choose(uris) {
            answerChooser({ chosen: [...uris] });
          },
          cancel() {
            answerChooser({ cancelled: {} });
          },
        })
      : undefined,
  };
}

const files = filesAvailable ? createFiles() : undefined;

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
  ]),
  PopupSurface,
  window,
  menuBar,
  keyboard,
  appearance,
  files,
};
