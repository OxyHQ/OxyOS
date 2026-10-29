/**
 * expo-clipboard on OxyOS (sample/oxyos/expo-adapter.js, docs/adr/0007-clipboard.mdx), for
 * scenarios/clipboard.json: a button copies with setStringAsync, another pastes with
 * getStringAsync, the change listener counts the clipboard's changes and asks hasStringAsync,
 * and a field takes Ctrl+C and Ctrl+V. Each shows what the app received.
 */
import * as React from 'react';
import { View } from 'react-native';
import * as Clipboard from 'expo-clipboard';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { TextField, TextFieldInput } from '@oxy.so/bloom/text-field';
import { Muted } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

export default function ClipboardProbe() {
  const [field, setField] = React.useState('');
  const [pasted, setPasted] = React.useState('nothing yet');
  const [changes, setChanges] = React.useState(0);
  const [has, setHas] = React.useState('unknown');
  React.useEffect(() => {
    const subscription = Clipboard.addClipboardListener(() => {
      setChanges((count) => count + 1);
      Clipboard.hasStringAsync().then((value) => setHas(String(value)));
    });
    return () => Clipboard.removeClipboardListener(subscription);
  }, []);
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View style={{ padding: 24, gap: 12 }}>
        <View style={{ flexDirection: 'row', gap: 12 }}>
          <Button testID="copy" onPress={() => Clipboard.setStringAsync('copiado desde JavaScript')}>
            Copy
          </Button>
          <Button testID="paste" onPress={() => Clipboard.getStringAsync().then((text) => setPasted(JSON.stringify(text)))}>
            Paste
          </Button>
        </View>
        <TextField>
          <TextFieldInput testID="field" label="Field" placeholder="Field" value={field} onChangeText={setField} />
        </TextField>
        <Muted>{`Field: ${JSON.stringify(field)}`}</Muted>
        <Muted>{`Pasted: ${pasted}`}</Muted>
        <Muted>{`Changes: ${changes}, has text: ${has}`}</Muted>
      </View>
    </BloomProvider>
  );
}
