/**
 * Text fields' context menu (scenarios/text-field-menu.json, docs/adr/0037-text-editing.mdx): a
 * Bloom field, a plain React Native TextInput that knows nothing of Bloom, and a secure field. The
 * menu is the platform's for every TextInput (sample/oxyos/text-menu.js); the probe draws none of
 * it. Each field shows its value and selection as the application receives them.
 */
import * as React from 'react';
import { TextInput, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { TextField, TextFieldInput } from '@oxy.so/bloom/text-field';
import { Muted } from '@oxy.so/bloom/typography';
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

export default function TextMenuProbe() {
  const line = useField('Hola mundo café');
  const plain = useField('texto plano');
  const secret = useField('clave');
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View style={{ padding: 24, gap: 12 }}>
        <TextField>
          <TextFieldInput testID="line" label="Line" placeholder="Line" {...line.props} />
        </TextField>
        <TextInput
          testID="plain"
          accessibilityLabel="Plain"
          style={{ height: 36, borderWidth: 1, borderColor: '#8a8f98', borderRadius: 6, paddingHorizontal: 10, fontSize: 15, color: '#111' }}
          {...plain.props}
        />
        <TextField>
          <TextFieldInput testID="secret" label="Secret" placeholder="Secret" secureTextEntry {...secret.props} />
        </TextField>
        <Muted>{`Line: ${line.shown}`}</Muted>
        <Muted>{`Plain: ${plain.shown}`}</Muted>
        <Muted>{`Secret: ${secret.shown}`}</Muted>
      </View>
    </BloomProvider>
  );
}
