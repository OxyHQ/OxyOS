/**
 * A sign-in form and a new account's fields, for scenarios/autofill*.json
 * (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 6): Bloom's text fields, each saying
 * what it holds as an application would on a phone (`textContentType`, `autoComplete`), nothing
 * more. The platform offers its suggestions at the focused field and fills the person's pick; the
 * application sees only the text filled, as if typed. The password's characters are counted,
 * never shown.
 */
import * as React from 'react';
import { View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { TextField, TextFieldInput } from '@oxy.so/bloom/text-field';
import { Muted } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

export default function LoginFormProbe() {
  const [username, setUsername] = React.useState('');
  const [password, setPassword] = React.useState('');
  const [chosen, setChosen] = React.useState('');
  const [code, setCode] = React.useState('');
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View style={{ padding: 24, gap: 12 }}>
        <TextField>
          <TextFieldInput
            testID="username"
            label="Email"
            placeholder="Email"
            textContentType="username"
            autoComplete="username"
            autoCapitalize="none"
            value={username}
            onChangeText={setUsername}
          />
        </TextField>
        <TextField>
          <TextFieldInput
            testID="password"
            label="Password"
            placeholder="Password"
            secureTextEntry
            textContentType="password"
            value={password}
            onChangeText={setPassword}
          />
        </TextField>
        <Muted>{`Password: ${[...password].length} characters`}</Muted>
        <TextField>
          <TextFieldInput
            testID="new-password"
            label="New password"
            placeholder="New password"
            secureTextEntry
            autoComplete="new-password"
            value={chosen}
            onChangeText={setChosen}
          />
        </TextField>
        <Muted>{`New password: ${[...chosen].length} characters`}</Muted>
        <TextField>
          <TextFieldInput
            testID="code"
            label="Code"
            placeholder="Code"
            textContentType="oneTimeCode"
            keyboardType="number-pad"
            value={code}
            onChangeText={setCode}
          />
        </TextField>
      </View>
    </BloomProvider>
  );
}
