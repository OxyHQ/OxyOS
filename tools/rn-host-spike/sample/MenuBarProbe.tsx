/**
 * An application's menus in the session's menu bar (docs/adr/0021-menu-bar.mdx), for
 * scenarios/menu-bar.json: Bloom's `Menubar` with File and Edit, their shortcuts and a check item.
 * With the adapter's `menu-bar` (sample/oxyos/platform-surface.js) Bloom draws nothing in the
 * window and describes the menus to the platform, which shows them in the shell's bar; a command
 * chosen there, or its shortcut typed in the window, runs here. The page says what ran and the
 * check's state, and has a field the shortcuts must reach past.
 */
import * as React from 'react';
import { TextInput, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Text } from '@oxy.so/bloom/typography';
import {
  Menubar,
  MenubarCheckboxItem,
  MenubarContent,
  MenubarItem,
  MenubarMenu,
  MenubarSeparator,
  MenubarSub,
  MenubarSubContent,
  MenubarSubTrigger,
  MenubarTrigger,
} from '@oxy.so/bloom/menubar';
import { oxyosSurfaces } from './oxyos/platform-surface';

export default function MenuBarProbe() {
  const [news, setNews] = React.useState(0);
  const [wrap, setWrap] = React.useState(false);
  const [note, setNote] = React.useState('');
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View testID="menu-probe" style={{ flex: 1, padding: 24, gap: 8, backgroundColor: '#ffffff' }}>
        <Menubar>
          <MenubarMenu value="file">
            <MenubarTrigger>File</MenubarTrigger>
            <MenubarContent>
              <MenubarItem shortcut="Mod+N" onPress={() => setNews((count) => count + 1)}>
                New
              </MenubarItem>
              <MenubarItem shortcut="Mod+O" onPress={() => {}}>
                Open…
              </MenubarItem>
              <MenubarSeparator />
              <MenubarSub>
                <MenubarSubTrigger>Open Recent</MenubarSubTrigger>
                <MenubarSubContent>
                  <MenubarItem onPress={() => {}}>notes.txt</MenubarItem>
                </MenubarSubContent>
              </MenubarSub>
            </MenubarContent>
          </MenubarMenu>
          <MenubarMenu value="edit">
            <MenubarTrigger>Edit</MenubarTrigger>
            <MenubarContent>
              <MenubarItem shortcut="Mod+Z" disabled onPress={() => {}}>
                Undo
              </MenubarItem>
              <MenubarSeparator />
              <MenubarCheckboxItem checked={wrap} onCheckedChange={setWrap}>
                Wrap Lines
              </MenubarCheckboxItem>
            </MenubarContent>
          </MenubarMenu>
        </Menubar>
        <Text testID="news">{`new documents: ${news}`}</Text>
        <Text testID="wrap">{`wrap lines: ${wrap ? 'on' : 'off'}`}</Text>
        <TextInput
          testID="note"
          value={note}
          onChangeText={setNote}
          placeholder="note"
          style={{ height: 36, borderWidth: 1, borderColor: '#cccccc', paddingHorizontal: 8 }}
        />
      </View>
    </BloomProvider>
  );
}
