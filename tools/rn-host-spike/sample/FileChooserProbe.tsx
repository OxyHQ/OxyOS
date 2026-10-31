/**
 * An application asking the person for files (docs/adr/0029-chromium-on-oxyos.mdx, decision 8):
 * each button asks the session through the host's `OxyOSFileChooser`, citing its press, for one
 * image, several, a folder or a file to save, or asks and withdraws; the answer is said as text.
 * The panel is the system's (the file manager in its picker mode, a sheet of this window), never
 * this application's.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';
import { chooseFiles } from './oxyos/file-chooser';

/** A chosen URI's last name, decoded; a folder's ends with `/`. */
function named(uri: string): string {
  const folder = uri.endsWith('/');
  const name = decodeURIComponent(uri.replace(/\/+$/, '').split('/').pop() ?? '');
  return folder ? `${name}/` : name;
}

/** The session's answer as one line. */
function said(answer: any): string {
  if (answer?.chosen) return `chosen ${answer.chosen.map(named).join(', ')}`;
  if (answer?.cancelled) return 'cancelled';
  if (answer?.refused) return `refused ${answer.refused.reason}`;
  return JSON.stringify(answer);
}

function Button({ label, y, onPress }: { label: string; y: number; onPress: () => void }) {
  return (
    <Pressable
      testID={label.toLowerCase().replace(/ /g, '-')}
      accessibilityRole="button"
      onPress={onPress}
      style={{
        position: 'absolute',
        left: 20,
        top: y,
        width: 160,
        height: 36,
        backgroundColor: '#2f6fe4',
        borderRadius: 8,
        justifyContent: 'center',
        alignItems: 'center',
      }}
    >
      <Text style={{ color: 'white' }}>{label}</Text>
    </Pressable>
  );
}

export default function FileChooserProbe() {
  const [asked, setAsked] = React.useState(0);
  const [answer, setAnswer] = React.useState('none yet');
  const ask = (request: object, withdrawAfter?: number) => {
    setAsked((n) => n + 1);
    const controller = new AbortController();
    chooseFiles(request, controller.signal).then((a) => setAnswer(said(a)));
    if (withdrawAfter != null) setTimeout(() => controller.abort(), withdrawAfter);
  };
  return (
    <View style={{ flex: 1, backgroundColor: 'white' }}>
      <Button label="Open" y={540} onPress={() => ask({ open: { types: ['image/png', '.png'], title: 'Choose an image' } })} />
      <Button label="Open several" y={490} onPress={() => ask({ open: { multiple: true, types: ['.png'] } })} />
      <Button label="Folder" y={440} onPress={() => ask({ open: { directory: true } })} />
      <Button label="Save" y={390} onPress={() => ask({ save: { name: 'Informe.txt', types: ['text/plain'] } })} />
      <Button label="Withdraw" y={340} onPress={() => ask({ open: {} }, 600)} />
      <Text style={{ position: 'absolute', left: 200, top: 545 }}>{`Asked: ${asked}`}</Text>
      <Text style={{ position: 'absolute', left: 200, top: 565 }}>{`Answer: ${answer}`}</Text>
    </View>
  );
}
