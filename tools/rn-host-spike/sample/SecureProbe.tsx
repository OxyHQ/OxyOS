/**
 * A Bloom password field, for scenarios/secure-field.json: what is typed into a secure field
 * reaches the application (the count below), but is never drawn, reported or published to an
 * assistive technology.
 */
import * as React from 'react';
import { View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { TextField, TextFieldInput } from '@oxy.so/bloom/text-field';
import { Muted } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

export default function SecureProbe() {
  const [password, setPassword] = React.useState('');
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View style={{ padding: 24, gap: 12 }}>
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
        <Muted>{`Typed ${[...password].length} characters`}</Muted>
      </View>
    </BloomProvider>
  );
}
