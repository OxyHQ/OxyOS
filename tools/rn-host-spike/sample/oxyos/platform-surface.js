/**
 * The OxyOS platform adapter for Bloom (`BloomProvider platform={oxyosSurfaces}`): Bloom's popups
 * become compositor popups, which may extend beyond the application window.
 *
 * `PopupSurface` renders the `OxyPopupView` host component. Its subtree stays in this React tree;
 * the native host draws it in a popup surface of its own, placed by the compositor from the
 * anchor's frame and Bloom's placement (side, align, offsets). The compositor's dismissal comes
 * back as `onDismiss`.
 */
import * as React from 'react';
import { findNodeHandle } from 'react-native';
import * as NativeComponentRegistry from 'react-native/Libraries/NativeComponent/NativeComponentRegistry';

const OxyPopupView = NativeComponentRegistry.get('OxyPopupView', () => ({
  uiViewClassName: 'OxyPopupView',
  validAttributes: {
    anchorTag: true,
    side: true,
    align: true,
    sideOffset: true,
    alignOffset: true,
  },
  directEventTypes: {
    topDismiss: { registrationName: 'onDismiss' },
  },
}));

function PopupSurface({ anchorRef, side, align, sideOffset, alignOffset, role, label, onDismiss, testID, children }) {
  // The anchor is a mounted view; its tag is known once the trigger has committed.
  const [anchorTag, setAnchorTag] = React.useState(null);
  React.useLayoutEffect(() => {
    setAnchorTag(anchorRef.current ? findNodeHandle(anchorRef.current) : null);
  }, [anchorRef]);
  if (anchorTag == null) return null;
  return (
    <OxyPopupView
      anchorTag={anchorTag}
      side={side}
      align={align}
      sideOffset={sideOffset}
      alignOffset={alignOffset}
      accessibilityRole={role}
      accessibilityLabel={label}
      onDismiss={(event) => onDismiss(event.nativeEvent.reason)}
      testID={testID}
      collapsable={false}
      // Out of the window's layout: the panel sizes itself, the compositor places it.
      style={{ position: 'absolute', left: 0, top: 0 }}
    >
      {children}
    </OxyPopupView>
  );
}

export const oxyosSurfaces = {
  platform: 'oxyos',
  capabilities: new Set(['popup-surface']),
  PopupSurface,
};
