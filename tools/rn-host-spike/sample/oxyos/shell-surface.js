/**
 * The OxyOS platform for the shell (`<Shell platform={oxyosShell} />`, apps/shell/src/platform.tsx):
 * each region the shell wraps in `Surface` (the top bar, the dock, the open overview or launcher)
 * renders the `OxyOSShellSurface` host component. Its subtree stays in the shell's React tree; the
 * Bloom host, run as the shell (`--shell`), draws it in a panel or an overlay of its own, where the
 * shell's layout put it (docs/adr/0009-session-shell.mdx).
 */
import * as React from 'react';
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

export const oxyosShell = { Surface };
