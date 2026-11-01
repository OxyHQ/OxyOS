/**
 * Drag and drop between applications, the target's side (docs/adr/0035-drag-and-drop.mdx): one
 * drop target filling the window (the OxyOS adapter's DropTarget, Bloom's DropZone) that takes
 * files and text, copying unless Command asks a move. It shows the action a drop would take while
 * a drag is over it, and each drop: what arrived, with its action, and how many drops it had.
 */
import * as React from 'react';
import { Text, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';

const surfaces = oxyosSurfaces as any;

function Probe() {
  const [over, setOver] = React.useState<string | null>(null);
  const [drops, setDrops] = React.useState<string[]>([]);
  const DropTarget = surfaces.DropTarget;
  const body = (
    <View
      style={{
        flex: 1,
        backgroundColor: over ? '#d3f9d8' : drops.length > 0 ? '#fff3bf' : '#e7f5ff',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 6,
      }}
    >
      <Text>{over ? `Over: ${over}` : 'Drop here'}</Text>
      <Text>{`Drops: ${drops.length}`}</Text>
      {drops.map((drop, index) => (
        <Text key={index}>{drop}</Text>
      ))}
    </View>
  );
  if (!DropTarget) return body;
  return (
    <DropTarget
      testID="drop-zone"
      accept={['files', 'text']}
      actions={['copy', 'move']}
      style={{ flex: 1 }}
      onDragEnter={({ action }: any) => setOver(action ?? 'nothing')}
      onDragOver={({ action }: any) => setOver(action ?? 'nothing')}
      onDragLeave={() => setOver(null)}
      onDrop={({ action, data }: any) => {
        const files = data.files.map((file: any) => `${file.name} at ${file.uri}`);
        setDrops((before) => [...before, `${files.join(', ') || `text ${data.text}`} (${action})`]);
      }}
    >
      {body}
    </DropTarget>
  );
}

export default function DropProbe() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <Probe />
    </BloomProvider>
  );
}
