/**
 * A store for scenarios (docs/adr/0030-installing-applications.mdx, decisions 9 and 16): what
 * Atlas asks the session through `OxyOSSystem` with its `store` and `updates` grants, each answer
 * said as text. Atlas itself is OxyHQ/Atlas's; this is the platform's test of the path beneath it.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';
import { available, store, updates } from './oxyos/system';

const ID = 'so.oxy.notes';

/** An answer as one line: its kind, and its reason when it has one. */
function said(answer: any): string {
  if (!answer || typeof answer !== 'object') return String(answer);
  const kind = Object.keys(answer)[0];
  const value = answer[kind];
  return value && typeof value === 'object' && 'reason' in value ? `${kind} ${value.reason}` : kind;
}

function Button({ label, x, y, onPress }: { label: string; x: number; y: number; onPress: () => void }) {
  return (
    <Pressable
      testID={label.toLowerCase()}
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      style={{ position: 'absolute', left: x, top: y, width: 140, height: 40, backgroundColor: '#2f6fe4', borderRadius: 8, justifyContent: 'center', alignItems: 'center' }}
    >
      <Text style={{ color: 'white' }}>{label}</Text>
    </Pressable>
  );
}

export default function StoreProbe() {
  const [answer, setAnswer] = React.useState({ count: 0, text: 'none yet' });
  const [progress, setProgress] = React.useState(0);
  const [state, setState] = React.useState(() => updates.current());
  React.useEffect(() => updates.subscribe(setState), []);
  // Numbered, so each answer is drawn even when it says what the one before said.
  const show = (asked: Promise<any>) => asked.then((a) => setAnswer((last) => ({ count: last.count + 1, text: said(a) })));
  return (
    <View style={{ flex: 1, backgroundColor: 'white' }}>
      <Text style={{ position: 'absolute', left: 20, top: 120 }}>{`Store: ${available ? 'available' : 'unavailable'}`}</Text>
      <Text style={{ position: 'absolute', left: 20, top: 140 }}>{`Answer ${answer.count}: ${answer.text}`}</Text>
      <Text style={{ position: 'absolute', left: 20, top: 160 }}>{`Progress: ${progress}`}</Text>
      <Text style={{ position: 'absolute', left: 20, top: 180 }}>{`Updates: ${state ? state.updates.length : 'unknown'}`}</Text>
      <Button label="Install" x={20} y={20} onPress={() => show(store.install(ID, () => setProgress((n) => n + 1)))} />
      <Button label="Uninstall" x={180} y={20} onPress={() => show(store.uninstall(ID))} />
      <Button label="Open" x={340} y={20} onPress={() => show(store.open(ID))} />
      <Button label="Check" x={20} y={70} onPress={() => show(updates.check())} />
      <Button label="UpdateAll" x={180} y={70} onPress={() => show(updates.updateAll())} />
      <Button label="Status" x={340} y={70} onPress={() => show(store.status([ID]))} />
    </View>
  );
}
