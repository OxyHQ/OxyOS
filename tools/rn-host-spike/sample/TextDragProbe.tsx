/**
 * Dragging selected text, as on a Mac (docs/adr/0037-text-editing.mdx), for scenarios/text-drag.json:
 * Bloom fields whose selected text drags out through the platform's drag and which take text
 * dropped on them: a single-line field, a multiline one, and a single-line one inside a drop target
 * (the OxyOS adapter's DropTarget, Bloom's DropZone) that takes text too, so a drop on the field is
 * the field's. Each field shows its value and selection as the application receives them, and the
 * drop target what was dropped on it.
 */
import * as React from 'react';
import { View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { TextField, TextFieldInput } from '@oxy.so/bloom/text-field';
import { Muted } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

const surfaces = oxyosSurfaces as any;

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

function Zone({ children }: { children: React.ReactNode }) {
  const [dropped, setDropped] = React.useState('nothing');
  const DropTarget = surfaces.DropTarget;
  const body = (
    <View style={{ padding: 12, gap: 6, borderRadius: 12, borderWidth: 2, borderColor: '#adb5bd' }}>
      {children}
      <Muted>{`Zone: ${dropped}`}</Muted>
    </View>
  );
  if (!DropTarget) return body;
  return (
    <DropTarget
      testID="zone"
      accept={['text']}
      actions={['copy']}
      onDrop={({ action, data }: any) => setDropped(`${data.text} (${action})`)}
    >
      {body}
    </DropTarget>
  );
}

function Probe() {
  const line = useField('uno dos tres');
  const notes = useField('');
  const inner = useField('');
  return (
    <View style={{ padding: 24, gap: 10, backgroundColor: 'white', flex: 1 }}>
      <View style={{ width: 300 }}>
        <TextField>
          <TextFieldInput testID="line" label="Line" placeholder="Line" {...line.props} />
        </TextField>
      </View>
      <View style={{ width: 300 }}>
        <TextField>
          <TextFieldInput
            testID="notes"
            label="Notes"
            placeholder="Notes"
            multiline
            style={{ height: 64 }}
            {...notes.props}
          />
        </TextField>
      </View>
      <View style={{ width: 300 }}>
        <Zone>
          <TextField>
            <TextFieldInput testID="inner" label="Inner" placeholder="Inner" maxLength={12} {...inner.props} />
          </TextField>
        </Zone>
      </View>
      <Muted>{`Line: ${line.shown}`}</Muted>
      <Muted>{`Notes: ${notes.shown}`}</Muted>
      <Muted>{`Inner: ${inner.shown}`}</Muted>
    </View>
  );
}

export default function TextDragProbe() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <Probe />
    </BloomProvider>
  );
}
