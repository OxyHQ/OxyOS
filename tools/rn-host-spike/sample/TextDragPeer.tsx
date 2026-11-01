/**
 * The other application of scenarios/text-drag.json (docs/adr/0037-text-editing.mdx): a row of text
 * dragged through the platform's drag once a pressed pointer moves 4 pixels, as DragProbe's, and two
 * drop targets taking text (the OxyOS adapter's DropTarget, Bloom's DropZone): one that copies unless
 * asked otherwise, and one that only moves, as a Trash does.
 */
import * as React from 'react';
import { Text, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';

const surfaces = oxyosSurfaces as any;

function Row() {
  const ref = React.useRef<View>(null);
  const pressed = React.useRef<{ x: number; y: number } | null>(null);
  const [result, setResult] = React.useState('none yet');
  return (
    <View
      ref={ref}
      testID="peer-row"
      onStartShouldSetResponder={() => true}
      onResponderGrant={(event) => {
        pressed.current = { x: event.nativeEvent.pageX, y: event.nativeEvent.pageY };
      }}
      onResponderMove={(event) => {
        const start = pressed.current;
        if (!start || !surfaces.drag) return;
        if (Math.hypot(event.nativeEvent.pageX - start.x, event.nativeEvent.pageY - start.y) < 4) return;
        pressed.current = null;
        surfaces.drag
          .start({ data: { text: 'Hola OxyOS' }, actions: ['copy', 'move'], previewRef: ref })
          .then((action: string | null) => setResult(action ?? 'none'));
      }}
      onResponderRelease={() => {
        pressed.current = null;
      }}
      style={{ height: 40, borderRadius: 10, backgroundColor: '#7048e8', justifyContent: 'center', paddingLeft: 12 }}
    >
      <Text style={{ color: 'white' }}>{`Hola OxyOS: ${result}`}</Text>
    </View>
  );
}

function Zone({ testID, label, actions }: { testID: string; label: string; actions: string[] }) {
  const [over, setOver] = React.useState(false);
  const [drops, setDrops] = React.useState<string[]>([]);
  const DropTarget = surfaces.DropTarget;
  const body = (
    <View
      style={{
        height: 64,
        borderRadius: 10,
        borderWidth: 2,
        borderColor: over ? '#2f9e44' : '#adb5bd',
        justifyContent: 'center',
        paddingLeft: 12,
      }}
    >
      <Text>{`${label}: ${drops.length ? drops.join(' / ') : 'nothing'}`}</Text>
    </View>
  );
  if (!DropTarget) return body;
  return (
    <DropTarget
      testID={testID}
      accept={['text']}
      actions={actions}
      onDragEnter={() => setOver(true)}
      onDragLeave={() => setOver(false)}
      onDrop={({ action, data }: any) => setDrops((before) => [...before, `${data.text} (${action})`])}
    >
      {body}
    </DropTarget>
  );
}

export default function TextDragPeer() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View style={{ flex: 1, padding: 16, gap: 12, backgroundColor: 'white' }}>
        <Row />
        <Zone testID="copy-zone" label="Copied" actions={['copy', 'move']} />
        <Zone testID="move-zone" label="Moved" actions={['move']} />
      </View>
    </BloomProvider>
  );
}
