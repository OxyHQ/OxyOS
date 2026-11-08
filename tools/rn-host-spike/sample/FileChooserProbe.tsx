/**
 * An application asking the person for files (docs/adr/0029-chromium-on-oxyos.mdx, decision 8):
 * each button asks the session through the platform's `chooser`, citing its press, for one
 * image, several, a folder or a file to save, or asks and withdraws; the answer is said as text.
 * The panel is the system's (the file manager in its picker mode, a sheet of this window), never
 * this application's. A text field below the sheet shows what the sheet's keyboard does to the
 * window's field (scenarios/text-inactive-sheet.json).
 */
import * as React from 'react';
import { Pressable, Text, TextInput, View } from 'react-native';
import { chooser, isPlatformError } from '@oxyos/platform';

/** A chosen URI's last name, decoded; a folder's ends with `/`. */
function named(uri: string): string {
  const folder = uri.endsWith('/');
  const name = decodeURIComponent(uri.replace(/\/+$/, '').split('/').pop() ?? '');
  return folder ? `${name}/` : name;
}

/** The panel's outcome as one line: what was chosen, cancelled (by the person, or withdrawn), or
    the session's refusal. */
function said(outcome: { chosen: string[] | null } | { error: unknown }): string {
  if ('chosen' in outcome) return outcome.chosen ? `chosen ${outcome.chosen.map(named).join(', ')}` : 'cancelled';
  const { error } = outcome;
  if (isPlatformError(error, 'cancelled')) return 'cancelled';
  if (isPlatformError(error)) return `refused ${error.reason ?? error.code}`;
  return String(error);
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
  const [note, setNote] = React.useState('');
  const ask = (panel: (signal: AbortSignal) => Promise<string[] | string | null>, withdrawAfter?: number) => {
    setAsked((n) => n + 1);
    const controller = new AbortController();
    panel(controller.signal).then(
      (chosen) => setAnswer(said({ chosen: typeof chosen === 'string' ? [chosen] : chosen })),
      (error) => setAnswer(said({ error })),
    );
    if (withdrawAfter != null) setTimeout(() => controller.abort(), withdrawAfter);
  };
  return (
    <View style={{ flex: 1, backgroundColor: 'white' }}>
      <Button label="Open" y={540} onPress={() => ask((signal) => chooser.open({ types: ['image/png', '.png'], title: 'Choose an image', signal }))} />
      <Button label="Open several" y={490} onPress={() => ask((signal) => chooser.open({ multiple: true, types: ['.png'], signal }))} />
      <Button label="Folder" y={440} onPress={() => ask((signal) => chooser.open({ directory: true, signal }))} />
      <Button label="Save" y={390} onPress={() => ask((signal) => chooser.save({ name: 'Informe.txt', types: ['text/plain'], signal }))} />
      <Button label="Withdraw" y={340} onPress={() => ask((signal) => chooser.open({ signal }), 600)} />
      <Text style={{ position: 'absolute', left: 200, top: 545 }}>{`Asked: ${asked}`}</Text>
      <Text style={{ position: 'absolute', left: 200, top: 565 }}>{`Answer: ${answer}`}</Text>
      <TextInput
        testID="note"
        value={note}
        onChangeText={setNote}
        style={{ position: 'absolute', left: 560, bottom: 12, width: 200, height: 36, borderWidth: 1, borderColor: '#8a94a6', borderRadius: 8, paddingHorizontal: 8 }}
      />
    </View>
  );
}
