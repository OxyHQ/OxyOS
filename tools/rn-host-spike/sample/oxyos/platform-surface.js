/**
 * The OxyOS platform adapter for Bloom (`BloomProvider platform={oxyosSurfaces}`): Bloom's popups
 * become compositor popups, which may extend beyond the application window, and an application's
 * window is Bloom's `window` (docs/adr/0016-windows.mdx): Bloom draws its controls in the
 * platform's place, and they and its drag regions act through the compositor.
 *
 * `PopupSurface` renders the `OxyOSPopupView` host component. Its subtree stays in this React tree;
 * the native host draws it in a popup surface of its own, placed by the compositor from the
 * anchor's frame and Bloom's placement (side, align, offsets). The compositor's dismissal comes
 * back as `onDismiss`.
 */
import * as React from 'react';
import { DeviceEventEmitter, findNodeHandle, TurboModuleRegistry } from 'react-native';
import * as NativeComponentRegistry from 'react-native/Libraries/NativeComponent/NativeComponentRegistry';

const OxyOSPopupView = NativeComponentRegistry.get('OxyOSPopupView', () => ({
  uiViewClassName: 'OxyOSPopupView',
  validAttributes: {
    anchorTag: true,
    side: true,
    align: true,
    sideOffset: true,
    alignOffset: true,
  },
  directEventTypes: {
    topDismiss: { registrationName: 'onDismiss' },
    topKey: { registrationName: 'onKey' },
  },
}));

function PopupSurface({
  anchorRef,
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
  };
}

const window = windowConstants?.available ? createWindow() : undefined;

export const oxyosSurfaces = {
  platform: 'oxyos',
  capabilities: new Set(window ? ['popup-surface', 'window'] : ['popup-surface']),
  PopupSurface,
  window,
};
