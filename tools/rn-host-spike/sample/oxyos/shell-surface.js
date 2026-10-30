/**
 * The OxyOS platform for the shell (`<Shell platform={oxyosShell} />`, apps/shell/src/platform.tsx):
 * each region the shell wraps in `Surface` (the top bar, the dock, the open overview or launcher)
 * renders the `OxyOSShellSurface` host component. Its subtree stays in the shell's React tree; the
 * Bloom host, run as the shell (`--shell`), draws it in a panel or an overlay of its own, where the
 * shell's layout put it (docs/adr/0009-session-shell.mdx). The overview's cards show each window
 * live through `WindowView` (docs/adr/0022-window-views.mdx).
 */
import * as React from 'react';
import { StyleSheet, View } from 'react-native';
import * as NativeComponentRegistry from 'react-native/Libraries/NativeComponent/NativeComponentRegistry';

const OxyOSShellSurface = NativeComponentRegistry.get('OxyOSShellSurface', () => ({
  uiViewClassName: 'OxyOSShellSurface',
  validAttributes: {
    kind: true,
    edge: true,
    exclusive: true,
  },
}));

function Surface({ kind, edge, exclusive, style, children }) {
  return (
    // Never flattened away: the host finds the region by its view.
    <OxyOSShellSurface kind={kind} edge={edge ?? 'top'} exclusive={!!exclusive} style={style} collapsable={false}>
      {children}
    </OxyOSShellSurface>
  );
}

// React Native's MaskedView (@react-native-masked-view/masked-view), as that package renders it:
// its RNCMaskedView with the mask in its first child, filling it, then the children. The Bloom host
// draws it as Android does (masked-view.h): the children kept where the mask is opaque.
const RNCMaskedView = NativeComponentRegistry.get('RNCMaskedView', () => ({
  uiViewClassName: 'RNCMaskedView',
  validAttributes: {},
}));

function Masked({ mask, style, children }) {
  return (
    <RNCMaskedView style={style} pointerEvents="box-none">
      {/* Never flattened away: the mask is the first child, whatever it holds. */}
      <View pointerEvents="none" style={StyleSheet.absoluteFill} collapsable={false}>
        {mask}
      </View>
      {children}
    </RNCMaskedView>
  );
}

// A window shown live (docs/adr/0022-window-views.mdx): the OxyOSWindowView host component, where
// the compositor draws the window numbered `window` itself, over the shell surface the view is in,
// fitted to the view in its proportions and clipped as it is. The shell never holds its pixels.
const OxyOSWindowView = NativeComponentRegistry.get('OxyOSWindowView', () => ({
  uiViewClassName: 'OxyOSWindowView',
  validAttributes: {
    window: true,
  },
}));

function WindowView({ windowId, style, accessibilityLabel }) {
  return (
    // Never flattened away: the host finds the window's place by its view.
    <OxyOSWindowView
      window={Number(windowId)}
      style={style}
      accessible={!!accessibilityLabel}
      accessibilityLabel={accessibilityLabel}
      collapsable={false}
    />
  );
}

export const oxyosShell = { Surface, Masked, WindowView };
