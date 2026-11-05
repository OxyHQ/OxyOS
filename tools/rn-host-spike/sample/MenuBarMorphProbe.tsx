/**
 * An application's menus of different sizes in the session's menu bar, for
 * scenarios/menu-bar-morph*.json: the bar's one popup changing from one menu into the next as
 * the pointer crosses the titles (Bloom's Menubar viewport, docs/adr/0021-menu-bar.mdx). File has
 * four rows at the menu's 266px floor; Edit two, shorter; View one row wider than the floor, so
 * the popup moves, grows and shrinks in both directions between them. The page counts what ran.
 */
import * as React from 'react';
import { View } from 'react-native';
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

export default function MenuBarMorphProbe() {
  const [news, setNews] = React.useState(0);
  const [wrap, setWrap] = React.useState(false);
  const [both, setBoth] = React.useState(false);
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
              <MenubarCheckboxItem checked={wrap} onCheckedChange={setWrap}>
                Wrap Lines
              </MenubarCheckboxItem>
            </MenubarContent>
          </MenubarMenu>
          <MenubarMenu value="view">
            <MenubarTrigger>View</MenubarTrigger>
            <MenubarContent>
              <MenubarCheckboxItem shortcut="Mod+Alt+Shift+S" checked={both} onCheckedChange={setBoth}>
                Show the Sidebar and the Inspector Together
              </MenubarCheckboxItem>
            </MenubarContent>
          </MenubarMenu>
        </Menubar>
        <Text testID="news">{`new documents: ${news}`}</Text>
        <Text testID="wrap">{`wrap lines: ${wrap ? 'on' : 'off'}`}</Text>
        <Text testID="both">{`sidebar and inspector: ${both ? 'on' : 'off'}`}</Text>
      </View>
    </BloomProvider>
  );
}
