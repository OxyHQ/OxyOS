/**
 * What the dock's drags need of the platform (docs/adr/0035-drag-and-drop.mdx, 6 November 2026),
 * the source's side: rows dragged through the platform's drag once a pressed pointer moves 4
 * pixels, as DragProbe's. "Inbox" is an application, as a launcher or a dock drags one (its id as
 * `application/x-oxyos-application`), and vanishes in a poof dropped on nothing; "notas.md" and
 * "foto.png" are files of the scenario's home, whose types the drag says. What each drag took is
 * shown ("Result: ..."), and why one dropped nothing ("Cancelled: ...", the adapter's onCancel).
 */
import * as React from 'react';
import { Text, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { ACCEPTS_FIRST_MOUSE } from '@oxy.so/bloom/window';
import { oxyosSurfaces } from './oxyos/platform-surface';

const surfaces = oxyosSurfaces as any;

function DragRow({
  label,
  data,
  actions,
  vanish = false,
  color,
}: {
  label: string;
  data: () => object | null;
  actions: string[];
  vanish?: boolean;
  color: string;
}) {
  const ref = React.useRef<View>(null);
  const [result, setResult] = React.useState('none yet');
  const [cancelled, setCancelled] = React.useState('nothing');
  const pressed = React.useRef<{ x: number; y: number } | null>(null);
  const dragging = React.useRef(false);
  return (
    <View style={{ gap: 4 }}>
      <View
        ref={ref}
        {...ACCEPTS_FIRST_MOUSE}
        accessible
        accessibilityLabel={label}
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
            .start({
              data: offered,
              actions,
              previewRef: ref,
              vanish,
              onCancel: (reason: string) => setCancelled(reason),
            })
            .then((action: string | null) => setResult(action ?? 'none'));
        }}
        onResponderRelease={() => {
          pressed.current = null;
        }}
        onResponderTerminate={() => {
          pressed.current = null;
        }}
        style={{ width: 200, height: 40, borderRadius: 10, backgroundColor: color, justifyContent: 'center', paddingLeft: 14 }}
      >
        <Text style={{ color: 'white' }}>{label}</Text>
      </View>
      <Text>{`${label} result: ${result}, cancelled: ${cancelled}`}</Text>
    </View>
  );
}

function Probe() {
  const [home, setHome] = React.useState<string | null>(null);
  React.useEffect(() => {
    surfaces.files?.places().then((places: any[]) => setHome(places.find((place) => place.kind === 'home')?.uri ?? null));
  }, []);
  const file = (name: string) => (home ? `${home.replace(/\/+$/, '')}/${name}` : null);
  const files = (name: string) => () => {
    const uri = file(name);
    return uri ? { files: [uri] } : null;
  };
  return (
    <View style={{ flex: 1, padding: 20, gap: 14, backgroundColor: 'white' }}>
      <DragRow
        label="Inbox"
        color="#1c7ed6"
        actions={['move']}
        vanish
        data={() => ({ other: { 'application/x-oxyos-application': 'so.oxy.inbox' } })}
      />
      <DragRow label="notas.md" color="#2b8a3e" actions={['copy']} data={files('notas.md')} />
      <DragRow label="foto.png" color="#c2255c" actions={['copy']} data={files('foto.png')} />
    </View>
  );
}

export default function VanishProbe() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <Probe />
    </BloomProvider>
  );
}
