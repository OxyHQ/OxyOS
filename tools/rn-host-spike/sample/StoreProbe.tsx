/**
 * A store for scenarios (docs/adr/0030-installing-applications.mdx, decisions 9 and 16): what
 * Atlas asks the session through `@oxyos/platform`'s `store` and `settings` with its `store` and `updates` grants, each answer
 * said as text. Atlas itself is OxyHQ/Atlas's; this is the platform's test of the path beneath it.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';
import { settings, store, type PlatformError } from '@oxyos/platform';
import { useFollowed } from '@oxyos/platform/react';

const available = store.available;

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

/** A rejection as the line its answer was: refused or failed, with the session's reason. */
function saidError(error: PlatformError): string {
  const kind = error.refused ? 'refused' : 'failed';
  return error.reason ? `${kind} ${error.reason}` : kind;
}

/** A request's outcome as one line, whether it resolved or rejected. */
const outcome = (pending: Promise<unknown>): Promise<string> => pending.then(said, saidError);

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
  const state = useFollowed(settings.updates) as { updates: any[] } | null;
  const [asked, setAsked] = React.useState(0);
  // Each action's last outcome on a line of its own: steady whenever it came.
  const [outcomes, setOutcomes] = React.useState<Record<string, string>>({});
  // What was asked, at once (a sheet may be waiting), then its answer when it comes.
  const show = (pending: Promise<string>) => {
    setAsked((n) => n + 1);
    pending.then((text) => setAnswer((last) => ({ count: last.count + 1, text })));
  };
  const keep = (action: string, request: Promise<unknown>) => {
    const pending = outcome(request);
    show(pending);
    pending.then((text) => setOutcomes((all) => ({ ...all, [action]: text })));
  };
  // Asked before the person did anything: it cites no input, and the session refuses it.
  React.useEffect(() => {
    outcome(store.install({ id: ID })).then((text) => setOutcomes((all) => ({ ...all, mount: text })));
  }, []);
  const listed = state
    ? `${state.updates.length}${state.updates.map((u: any) => ` ${u.id} ${u.toVersionCode} ${u.status}`).join('')}`
    : 'unknown';
  return (
    <View style={{ flex: 1, backgroundColor: 'white' }}>
      <Button label="Install" x={20} y={20} onPress={() => keep('install', store.install({ id: ID }, { onProgress: () => setProgress((n) => n + 1) }))} />
      <Button label="Uninstall" x={180} y={20} onPress={() => keep('uninstall', store.uninstall(ID))} />
      <Button label="Open" x={340} y={20} onPress={() => keep('open', store.open(ID))} />
      <Button label="Check" x={20} y={70} onPress={() => keep('check', settings.checkUpdates())} />
      <Button label="UpdateAll" x={180} y={70} onPress={() => keep('updateAll', settings.updateAll())} />
      <Button label="Status" x={340} y={70} onPress={() => keep('status', store.status({ ids: [ID] }))} />
      {/* Two checks at once: the install service reads the store once for both. */}
      <Button label="CheckTwice" x={20} y={120} onPress={() => { keep('check', settings.checkUpdates()); keep('check2', settings.checkUpdates()); }} />
      <Button label="Settings" x={180} y={120} onPress={() => keep('openSettings', store.open('so.oxy.settings'))} />
      <Button label="Probe" x={340} y={120} onPress={() => keep('openProbe', store.open('so.oxy.permissionprobe'))} />
      <Line y={170}>{`Store: ${available ? 'available' : 'unavailable'}`}</Line>
      <Line y={190}>{`Answer ${answer.count}: ${answer.text}`}</Line>
      <Line y={210}>{`Progress: ${progress}`}</Line>
      <Line y={230}>{`Updates: ${listed}`}</Line>
      <Line y={250}>{`Asked: ${asked}`}</Line>
      <Line y={270}>{`Install: ${outcomes.install ?? '-'} · Uninstall: ${outcomes.uninstall ?? '-'} · Open: ${outcomes.open ?? '-'}`}</Line>
      <Line y={310}>{`Unasked: ${outcomes.mount ?? '-'}`}</Line>
      <Line y={290}>{`Check: ${outcomes.check ?? '-'} · UpdateAll: ${outcomes.updateAll ?? '-'} · Status: ${outcomes.status ?? '-'}`}</Line>
    </View>
  );
}
