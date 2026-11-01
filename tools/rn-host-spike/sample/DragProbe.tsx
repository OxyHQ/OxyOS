/**
 * Drag and drop between applications, the source's side (docs/adr/0035-drag-and-drop.mdx): rows of
 * a file of the scenario's home and of text, dragged through the platform's drag (the OxyOS
 * adapter's `drag`, Bloom's usePlatformDrag) once a pressed pointer moves 4 pixels, as a file
 * manager starts its drags; what the drop took is shown ("Result: ..."). A drop target of its own
 * (the adapter's DropTarget, Bloom's DropZone) takes files and text dropped on its right side.
 */
import * as React from 'react';
import { Text, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosSurfaces } from './oxyos/platform-surface';

const surfaces = oxyosSurfaces as any;

function DragRow({ testID, label, data, color }: { testID: string; label: string; data: () => object | null; color: string }) {
  const ref = React.useRef<View>(null);
  const [result, setResult] = React.useState('none yet');
  const pressed = React.useRef<{ x: number; y: number } | null>(null);
  const dragging = React.useRef(false);
  return (
    <View style={{ gap: 4 }}>
      <View
        ref={ref}
        testID={testID}
        onStartShouldSetResponder={() => true}
        onResponderGrant={(event) => {
          pressed.current = { x: event.nativeEvent.pageX, y: event.nativeEvent.pageY };
          dragging.current = false;
        }}
        onResponderMove={(event) => {
          const start = pressed.current;
          if (!start || dragging.current) return;
          const moved = Math.hypot(event.nativeEvent.pageX - start.x, event.nativeEvent.pageY - start.y);
          const offered = data();
          if (moved < 4 || !offered || !surfaces.drag) return;
          dragging.current = true;
          surfaces.drag
            .start({ data: offered, actions: ['copy', 'move'], previewRef: ref })
            .then((action: string | null) => setResult(action ?? 'none'));
        }}
        onResponderRelease={() => {
          pressed.current = null;
        }}
        onResponderTerminate={() => {
          pressed.current = null;
        }}
        style={{ width: 220, height: 44, borderRadius: 10, backgroundColor: color, justifyContent: 'center', paddingLeft: 14 }}
      >
        <Text style={{ color: 'white' }}>{label}</Text>
      </View>
      <Text testID={`${testID}-result`}>{`Result: ${result}`}</Text>
    </View>
  );
}

function LocalZone() {
  const [over, setOver] = React.useState(false);
  const [dropped, setDropped] = React.useState('nothing');
  const DropTarget = surfaces.DropTarget;
  const zone = (
    <View
      style={{
        width: 260,
        height: 160,
        borderRadius: 14,
        borderWidth: 2,
        borderColor: over ? '#2f9e44' : '#adb5bd',
        backgroundColor: over ? '#d3f9d8' : '#f8f9fa',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <Text>{`Here: ${dropped}`}</Text>
    </View>
  );
  if (!DropTarget) return zone;
  return (
    <DropTarget
      testID="local-zone"
      accept={['files', 'text']}
      actions={['move', 'copy']}
      onDragEnter={() => setOver(true)}
      onDragLeave={() => setOver(false)}
      onDrop={({ action, data }: any) =>
        setDropped(`${data.files.map((file: any) => file.name).join(', ') || data.text} (${action})`)
      }
    >
      {zone}
    </DropTarget>
  );
}

function Probe() {
  const [home, setHome] = React.useState<string | null>(null);
  React.useEffect(() => {
    surfaces.files?.places().then((places: any[]) => setHome(places.find((place) => place.kind === 'home')?.uri ?? null));
  }, []);
  const file = home ? `${home.replace(/\/+$/, '')}/informe.txt` : null;
  return (
    <View style={{ flex: 1, flexDirection: 'row', justifyContent: 'space-between', padding: 24, backgroundColor: 'white' }}>
      <View style={{ gap: 18 }}>
        <Text>{surfaces.drag ? 'Drags between applications' : 'No platform drag'}</Text>
        <DragRow testID="file-row" label="informe.txt" color="#1c7ed6" data={() => (file ? { files: [file] } : null)} />
        <DragRow testID="text-row" label="Hola OxyOS" color="#7048e8" data={() => ({ text: 'Hola OxyOS' })} />
      </View>
      <LocalZone />
    </View>
  );
}

export default function DragProbe() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <Probe />
    </BloomProvider>
  );
}
