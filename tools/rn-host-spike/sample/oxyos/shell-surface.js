/**
 * The OxyOS platform for the shell (`<Shell platform={oxyosShell} />`, apps/shell/src/platform.tsx):
 * each region the shell wraps in `Surface` (the top bar, the dock, the open overview or launcher)
 * renders the `OxyOSShellSurface` host component. Its subtree stays in the shell's React tree; the
 * Bloom host, run as the shell (`--shell`), draws it in a panel or an overlay of its own, where the
 * shell's layout put it (docs/adr/0009-session-shell.mdx). The overview's cards show each window
 * live through `WindowView` (docs/adr/0022-window-views.mdx). A `backdrop` region is an overlay's,
 * beneath it, which the compositor fades and never scales (docs/adr/0011-transitions.mdx).
 */
import * as React from 'react';
import { DeviceEventEmitter, StyleSheet, View } from 'react-native';
import * as NativeComponentRegistry from 'react-native/Libraries/NativeComponent/NativeComponentRegistry';
import { oxyosSurfaces } from './platform-surface';

const OxyOSShellSurface = NativeComponentRegistry.get('OxyOSShellSurface', () => ({
  uiViewClassName: 'OxyOSShellSurface',
  validAttributes: {
    kind: true,
    edge: true,
    exclusive: true,
    keyboard: true,
    focusTarget: true,
    focusRequest: true,
    excludeFromCapture: true,
  },
}));

// A region asking for the keyboard (`keyboard`) is given it by the host, citing the latest input;
// once it had it, `onKeyboardLost` hears that it went elsewhere (OxyOSShellKeyboardLost).
// A region left out of screenshots (`excludeFromCapture`: the screenshot interface, its thumbnail)
// is never in one (docs/adr/0024-system-shortcuts.mdx, decision 10).
// Once it has the keyboard, the host puts the keyboard's focus on `focusTarget` ("first", its first
// control, or a control's testID), again for each new `focusRequest` (docs/adr/0039-keyboard-access.mdx).
function Surface({
  kind,
  edge,
  exclusive,
  keyboard,
  focusTarget,
  focusRequest,
  excludeFromCapture,
  onKeyboardLost,
  style,
  children,
}) {
  const lost = React.useRef(onKeyboardLost);
  lost.current = onKeyboardLost;
  React.useEffect(() => {
    if (!keyboard) return undefined;
    const subscription = DeviceEventEmitter.addListener('OxyOSShellKeyboardLost', () => lost.current?.());
    return () => subscription.remove();
  }, [keyboard]);
  return (
    // Never flattened away: the host finds the region by its view.
    <OxyOSShellSurface
      kind={kind}
      edge={edge ?? 'top'}
      exclusive={!!exclusive}
      keyboard={!!keyboard}
      focusTarget={focusTarget ?? ''}
      focusRequest={focusRequest ?? 0}
      excludeFromCapture={!!excludeFromCapture}
      style={style}
      collapsable={false}
    >
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

// The keys the shell's surfaces receive, offered before the focused field (the host's
// OxyOSKeyboard, as an application's window has it): Launchpad's arrows and Return.
export const oxyosShell = { Surface, Masked, WindowView, keyboard: oxyosSurfaces.keyboard };
