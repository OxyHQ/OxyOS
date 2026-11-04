/**
 * The Edit menu acting on the focused text field, as a Mac's first responder takes it
 * (scenarios/edit-menu.json, docs/adr/0021-menu-bar.mdx, decision 3): Bloom's `Menubar` with a
 * Mac's Edit menu, each standard item an `action` (Undo, Redo, Cut, Copy, Paste, Paste and Match
 * Style, Delete, Select All), Copy the application's own too (it copies the page's selected item
 * when no field has focus), and Find… the application's command. The fields are a Bloom field, a
 * plain TextInput and a secure field; the page says each field's value and selection and what the
 * application's own commands did, and none of the menu's editing is the page's. `EditMenuPeer` is
 * the same page as a second application, its names and test ids prefixed with "Peer", for the
 * clipboard's round trip between two applications' fields.
 */
import * as React from 'react';
import { Keyboard, Pressable, TextInput, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { TextField, TextFieldInput } from '@oxy.so/bloom/text-field';
import { Muted } from '@oxy.so/bloom/typography';
import {
  Menubar,
  MenubarContent,
  MenubarItem,
  MenubarMenu,
  MenubarSeparator,
  MenubarTrigger,
} from '@oxy.so/bloom/menubar';
import { oxyosSurfaces } from './oxyos/platform-surface';

type Selection = { start: number; end: number };

function useField(initial: string) {
  const [value, setValue] = React.useState(initial);
  const [selection, setSelection] = React.useState<Selection>({ start: 0, end: 0 });
  return {
    shown: `${JSON.stringify(value)} [${selection.start},${selection.end}]`,
    props: {
      value,
      onChangeText: setValue,
      onSelectionChange: (event: { nativeEvent: { selection: Selection } }) =>
        setSelection(event.nativeEvent.selection),
    },
  };
}

const button = { paddingHorizontal: 10, paddingVertical: 6, borderRadius: 6, backgroundColor: '#e8eaee' };

export function EditMenuPeer() {
  return <EditMenuProbe peer />;
}

export default function EditMenuProbe({ peer = false }: { peer?: boolean }) {
  const id = (name: string) => (peer ? `peer-${name}` : name);
  const named = (name: string) => (peer ? `Peer ${name.toLowerCase()}` : name);
  const line = useField('');
  const plain = useField('');
  const secret = useField('clave');
  const [itemSelected, setItemSelected] = React.useState(false);
  const [copies, setCopies] = React.useState(0);
  const [finds, setFinds] = React.useState(0);
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View testID={id('edit-probe')} style={{ flex: 1, padding: 24, gap: 10, backgroundColor: '#ffffff' }}>
        <Menubar>
          <MenubarMenu value="edit">
            <MenubarTrigger>Edit</MenubarTrigger>
            <MenubarContent>
              <MenubarItem action="undo">Undo</MenubarItem>
              <MenubarItem action="redo">Redo</MenubarItem>
              <MenubarSeparator />
              <MenubarItem action="cut">Cut</MenubarItem>
              <MenubarItem action="copy" disabled={!itemSelected} onPress={() => setCopies((count) => count + 1)}>
                Copy
              </MenubarItem>
              <MenubarItem action="paste">Paste</MenubarItem>
              <MenubarItem action="pasteAndMatchStyle">Paste and Match Style</MenubarItem>
              <MenubarItem action="delete">Delete</MenubarItem>
              <MenubarItem action="selectAll">Select All</MenubarItem>
              <MenubarSeparator />
              <MenubarItem shortcut="Mod+F" onPress={() => setFinds((count) => count + 1)}>
                Find…
              </MenubarItem>
            </MenubarContent>
          </MenubarMenu>
        </Menubar>
        <TextField>
          <TextFieldInput testID={id('line')} label={named('Line')} placeholder={named('Line')} {...line.props} />
        </TextField>
        <TextInput
          testID={id('plain')}
          accessibilityLabel={named('Plain')}
          style={{ height: 36, borderWidth: 1, borderColor: '#8a8f98', borderRadius: 6, paddingHorizontal: 10, fontSize: 15, color: '#111' }}
          {...plain.props}
        />
        <TextField>
          <TextFieldInput testID={id('secret')} label={named('Secret')} placeholder={named('Secret')} secureTextEntry {...secret.props} />
        </TextField>
        <View style={{ flexDirection: 'row', gap: 8 }}>
          <Pressable testID={id('select-item')} accessibilityRole="button" accessibilityLabel={named('Select item')} style={button} onPress={() => setItemSelected((selected) => !selected)}>
            <Muted>{itemSelected ? 'Item selected' : 'Select item'}</Muted>
          </Pressable>
          <Pressable testID={id('done')} accessibilityRole="button" accessibilityLabel={named('Done')} style={button} onPress={() => Keyboard.dismiss()}>
            <Muted>Done</Muted>
          </Pressable>
        </View>
        <Muted>{`Line: ${line.shown}`}</Muted>
        <Muted>{`Plain: ${plain.shown}`}</Muted>
        <Muted>{`Secret: ${secret.shown}`}</Muted>
        <Muted>{`copies: ${copies}, finds: ${finds}`}</Muted>
      </View>
    </BloomProvider>
  );
}
