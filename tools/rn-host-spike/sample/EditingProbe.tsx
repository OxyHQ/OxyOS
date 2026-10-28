/**
 * Two Bloom fields for scenarios/text-editing.json: a single-line one and a narrow multiline one
 * whose text wraps. Each shows its value and selection as the application receives them, so the
 * scenario checks where every key leaves the caret, not only that something changed.
 */
import * as React from 'react';
import { View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { TextField, TextFieldInput } from '@oxy.so/bloom/text-field';
import { Muted } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

type Selection = { start: number; end: number };

function useField() {
  const [value, setValue] = React.useState('');
  const [selection, setSelection] = React.useState<Selection>({ start: 0, end: 0 });
  return {
    value,
    shown: `${JSON.stringify(value)} [${selection.start},${selection.end}]`,
    props: {
      value,
      onChangeText: setValue,
      onSelectionChange: (event: { nativeEvent: { selection: Selection } }) =>
        setSelection(event.nativeEvent.selection),
    },
  };
}

export default function EditingProbe() {
  const line = useField();
  const notes = useField();
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View style={{ padding: 24, gap: 12 }}>
        <TextField>
          <TextFieldInput testID="line" label="Line" placeholder="Line" {...line.props} />
        </TextField>
        <View style={{ width: 260 }}>
          <TextField>
            <TextFieldInput
              testID="notes"
              label="Notes"
              placeholder="Notes"
              multiline
              style={{ height: 120 }}
              {...notes.props}
            />
          </TextField>
        </View>
        <Muted>{`Line: ${line.shown}`}</Muted>
        <Muted>{`Notes: ${notes.shown}`}</Muted>
      </View>
    </BloomProvider>
  );
}
