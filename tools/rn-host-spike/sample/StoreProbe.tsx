/**
 * A store for scenarios (docs/adr/0030-installing-applications.mdx, decisions 9 and 16): what
 * Atlas asks the session through `OxyOSSystem` with its `store` and `updates` grants, each answer
 * said as text. Atlas itself is OxyHQ/Atlas's; this is the platform's test of the path beneath it.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';
import { available, store, updates } from './oxyos/system';

const ID = 'so.oxy.inbox';

/** An answer as one line: its kind, and its reason or what it came to when it has one. */
function said(answer: any): string {
  if (!answer || typeof answer !== 'object') return String(answer);
  const kind = Object.keys(answer)[0];
  const value = answer[kind];
  if (kind === 'status' && Array.isArray(value)) {
    const app = value[0] ?? {};
    return `status ${app.installed ? `v${app.versionCode}` : 'none'}${app.update ? ` update ${app.update.toVersionCode}` : ''}`;
  }
  if (kind === 'updated' && Array.isArray(value)) return `updated ${value.map((u: any) => u.outcome).join(',') || 'none'}`;
  if (kind === 'installed' && value) return `installed v${value.versionCode}`;
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

function Line({ y, children }: { y: number; children: string }) {
  return <Text style={{ position: 'absolute', left: 20, top: y }}>{children}</Text>;
}

export default function StoreProbe() {
  const [answer, setAnswer] = React.useState({ count: 0, text: 'none yet' });
  const [progress, setProgress] = React.useState(0);
  const [state, setState] = React.useState(() => updates.current());
  React.useEffect(() => updates.subscribe(setState), []);
  const [asked, setAsked] = React.useState(0);
  // Each action's last outcome on a line of its own: steady whenever it came.
  const [outcomes, setOutcomes] = React.useState<Record<string, string>>({});
  // What was asked, at once (a sheet may be waiting), then its answer when it comes.
  const show = (pending: Promise<any>) => {
    setAsked((n) => n + 1);
    pending.then((a) => setAnswer((last) => ({ count: last.count + 1, text: said(a) })));
  };
  const keep = (action: string, pending: Promise<any>) => {
    show(pending);
    pending.then((a) => setOutcomes((all) => ({ ...all, [action]: said(a) })));
  };
  const listed = state
    ? `${state.updates.length}${state.updates.map((u: any) => ` ${u.id} ${u.toVersionCode} ${u.status}`).join('')}`
    : 'unknown';
  return (
    <View style={{ flex: 1, backgroundColor: 'white' }}>
      <Button label="Install" x={20} y={20} onPress={() => keep('install', store.install(ID, () => setProgress((n) => n + 1)))} />
      <Button label="Uninstall" x={180} y={20} onPress={() => keep('uninstall', store.uninstall(ID))} />
      <Button label="Open" x={340} y={20} onPress={() => keep('open', store.open(ID))} />
      <Button label="Check" x={20} y={70} onPress={() => keep('check', updates.check())} />
      <Button label="UpdateAll" x={180} y={70} onPress={() => keep('updateAll', updates.updateAll())} />
      <Button label="Status" x={340} y={70} onPress={() => keep('status', store.status([ID]))} />
      {/* Two checks at once: the install service reads the store once for both. */}
      <Button label="CheckTwice" x={20} y={120} onPress={() => { keep('check', updates.check()); keep('check2', updates.check()); }} />
      <Button label="Settings" x={180} y={120} onPress={() => keep('openSettings', store.open('so.oxy.settings'))} />
      <Button label="Probe" x={340} y={120} onPress={() => keep('openProbe', store.open('so.oxy.permissionprobe'))} />
      <Line y={170}>{`Store: ${available ? 'available' : 'unavailable'}`}</Line>
      <Line y={190}>{`Answer ${answer.count}: ${answer.text}`}</Line>
      <Line y={210}>{`Progress: ${progress}`}</Line>
      <Line y={230}>{`Updates: ${listed}`}</Line>
      <Line y={250}>{`Asked: ${asked}`}</Line>
      <Line y={270}>{`Install: ${outcomes.install ?? '-'} · Uninstall: ${outcomes.uninstall ?? '-'} · Open: ${outcomes.open ?? '-'}`}</Line>
      <Line y={290}>{`Check: ${outcomes.check ?? '-'} · UpdateAll: ${outcomes.updateAll ?? '-'} · Status: ${outcomes.status ?? '-'}`}</Line>
    </View>
  );
}
