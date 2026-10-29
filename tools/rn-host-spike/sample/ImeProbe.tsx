/**
 * Three Bloom fields for scenarios/text-input.json (ADR 0008): a plain one, an email one and a
 * password. Each shows its value and selection as the application receives them, so the scenario
 * checks what an input method's transactions leave in the field; the password shows only how many
 * characters it has.
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

export default function ImeProbe() {
  const line = useField();
  const email = useField();
  const [password, setPassword] = React.useState('');
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View style={{ padding: 24, gap: 12 }}>
        <TextField>
          <TextFieldInput testID="line" label="Line" placeholder="Line" {...line.props} />
        </TextField>
        <TextField>
          <TextFieldInput
            testID="email"
            label="Email"
            placeholder="Email"
            keyboardType="email-address"
            autoCapitalize="none"
            {...email.props}
          />
        </TextField>
        <TextField>
          <TextFieldInput
            testID="password"
            label="Password"
            placeholder="Password"
            secureTextEntry
            value={password}
            onChangeText={setPassword}
          />
        </TextField>
        <Muted>{`Line: ${line.shown}`}</Muted>
        <Muted>{`Email: ${email.shown}`}</Muted>
        <Muted>{`Password: ${[...password].length} characters`}</Muted>
      </View>
    </BloomProvider>
  );
}
