/**
 * An application that has just signed someone in asks to save the login it used
 * (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 5), through `@oxyos/platform`'s
 * `keychain.savePassword`, for scenarios/autofill-save*.json: the system asks the person in its
 * sheet over the window, and the probe shows only what came of it (saved, not saved, or the
 * error's code and reason), never the password. The login is made up.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';
import { keychain, isPlatformError } from '@oxyos/platform';

/** Made-up: the scenarios check it never appears in what they record. */
const USERNAME = 'bea@example.com';
const PASSWORD = 'fixture-Saved-Passw0rd';

export default function SavePasswordProbe() {
  const [asked, setAsked] = React.useState(0);
  const [result, setResult] = React.useState('none');
  const save = () => {
    setAsked((count) => count + 1);
    setResult('asking');
    keychain
      .savePassword({ username: USERNAME, password: PASSWORD, domain: 'example.com' })
      .then(setResult, (error: unknown) =>
        setResult(isPlatformError(error) ? `${error.code} ${error.reason ?? ''}`.trim() : 'thrown'),
      );
  };
  return (
    <View style={{ padding: 24, gap: 12 }}>
      <Pressable testID="sign-in" accessibilityRole="button" onPress={save}
        style={{ padding: 12, backgroundColor: '#e8eefc', borderRadius: 8 }}>
        <Text>Sign in</Text>
      </Pressable>
      <Text>{`Asked: ${asked}`}</Text>
      <Text>{`Saved: ${result}`}</Text>
      <Text>{`Keychain: ${keychain.available ? 'available' : 'unavailable'}`}</Text>
    </View>
  );
}
