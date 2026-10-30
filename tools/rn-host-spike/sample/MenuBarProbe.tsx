/**
 * An application's menus in the session's menu bar (docs/adr/0021-menu-bar.mdx), for
 * scenarios/menu-bar.json: File and Edit with their shortcuts and a check item, given to the
 * platform's `menuBar` (sample/oxyos/platform-surface.js) and run when the shell's bar says one was
 * chosen. The page says what ran and the check's state.
 *
 * Bloom's `Menubar` describing itself through the `menu-bar` capability is Bloom's step of ADR 0021
 * and not released yet: until it is, this probe calls the adapter as Bloom will, with the
 * `PlatformMenu` description the ADR gives.
 */
import * as React from 'react';
import { View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Text } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

const menus = (wrap: boolean) => [
  {
    id: 'file',
    label: 'File',
    items: [
      { kind: 'item', id: 'file.new', label: 'New', enabled: true, shortcut: 'Mod+N' },
      { kind: 'item', id: 'file.open', label: 'Open…', enabled: true, shortcut: 'Mod+O' },
      { kind: 'separator' },
      {
        kind: 'submenu',
        id: 'file.recent',
        label: 'Open Recent',
        enabled: true,
        items: [{ kind: 'item', id: 'file.recent.notes', label: 'notes.txt', enabled: true }],
      },
    ],
  },
  {
    id: 'edit',
    label: 'Edit',
    items: [
      { kind: 'item', id: 'edit.undo', label: 'Undo', enabled: false, shortcut: 'Mod+Z' },
      { kind: 'separator' },
      { kind: 'item', id: 'edit.wrap', label: 'Wrap Lines', enabled: true, checked: wrap },
    ],
  },
];

export default function MenuBarProbe() {
  const [news, setNews] = React.useState(0);
  const [wrap, setWrap] = React.useState(false);
  const bar = oxyosSurfaces.menuBar;
  React.useEffect(
    () =>
      bar?.onActivate((id: string) => {
        if (id === 'file.new') setNews((count) => count + 1);
        if (id === 'edit.wrap') setWrap((on) => !on);
      }),
    [bar],
  );
  // Whenever they change, as Bloom's Menubar does.
  React.useEffect(() => {
    bar?.setMenus(menus(wrap));
  }, [bar, wrap]);
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View testID="menu-probe" style={{ flex: 1, padding: 24, gap: 8, backgroundColor: '#ffffff' }}>
        <Text testID="menu-bar">{bar ? 'menu bar: the platform’s' : 'menu bar: none'}</Text>
        <Text testID="news">{`new documents: ${news}`}</Text>
        <Text testID="wrap">{`wrap lines: ${wrap ? 'on' : 'off'}`}</Text>
      </View>
    </BloomProvider>
  );
}
