/**
 * A desktop's presses in the Bloom host (input.h, popup.h, textinput.h), for
 * scenarios/pointer-presses.json: every mouse button's press and release as W3C pointer events
 * with its button, the buttons held and the modifiers; a secondary press that presses nothing and
 * opens Bloom's context menu in a compositor popup at the press; a double press; and a selection
 * JavaScript sets in a text input (`setSelection`), which the next key replaces.
 */
import * as React from 'react';
import { Pressable, TextInput, View, type PointerEvent } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from '@oxy.so/bloom/context-menu';
import { Muted } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

/** A pointer event as the probe shows it: what, which button, which held, which modifiers. */
function described(kind: string, event: PointerEvent) {
  const { button, buttons, shiftKey, ctrlKey, altKey, metaKey } = event.nativeEvent;
  const modifiers = [shiftKey && 'shift', ctrlKey && 'ctrl', altKey && 'alt', metaKey && 'meta'].filter(Boolean);
  return `${kind} button ${button} held ${buttons}${modifiers.length ? ` ${modifiers.join('+')}` : ''}`;
}

export default function PressProbe() {
  const [pointer, setPointer] = React.useState('none');
  const [downs, setDowns] = React.useState(0);
  const [presses, setPresses] = React.useState(0);
  const [chosen, setChosen] = React.useState('nothing');
  const [text, setText] = React.useState('hello world');
  const field = React.useRef<TextInput>(null);
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View style={{ padding: 16, gap: 10 }}>
        <ContextMenu>
          <ContextMenuTrigger label="Pad" testID="pad-menu">
            <Pressable
              testID="pad"
              onPress={() => setPresses((count) => count + 1)}
              onPointerDown={(event) => {
                setDowns((count) => count + 1);
                setPointer(described('down', event));
              }}
              onPointerUp={(event) => setPointer(described('up', event))}
              style={{ width: 300, height: 120, backgroundColor: '#dfe7f5', borderRadius: 12 }}
            />
          </ContextMenuTrigger>
          <ContextMenuContent>
            <ContextMenuItem onPress={() => setChosen('Rename')}>Rename</ContextMenuItem>
            <ContextMenuItem onPress={() => setChosen('Duplicate')}>Duplicate</ContextMenuItem>
          </ContextMenuContent>
        </ContextMenu>
        <Muted>{`Pointer: ${pointer}`}</Muted>
        <Muted>{`Downs: ${downs}, presses: ${presses}, chosen: ${chosen}`}</Muted>
        <View style={{ flexDirection: 'row', gap: 12, alignItems: 'center' }}>
          <TextInput
            ref={field}
            testID="field"
            accessibilityLabel="Field"
            value={text}
            onChangeText={setText}
            style={{ width: 200, height: 36, borderWidth: 1, borderColor: '#8a94a6', borderRadius: 8, paddingHorizontal: 8 }}
          />
          <Button testID="select" onPress={() => field.current?.setSelection(6, 11)}>
            Select world
          </Button>
        </View>
        <Muted>{`Text: ${text}`}</Muted>
      </View>
    </BloomProvider>
  );
}
