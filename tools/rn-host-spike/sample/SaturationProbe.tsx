/**
 * The saturation benchmark's applications (perf.py saturation, docs/performance.mdx;
 * docs/adr/0066-quality-of-service.mdx).
 * - PointerTrackProbe draws a dot where the pointer is, moved on every pointermove: each motion of
 *   the benchmark's pointer interaction has a frame that answers it, as a drawing application's or
 *   a tooltip following the pointer has.
 *
 * And two that keep the machine busy while hidden, for load S4: the session starts them as it
 * starts any application and minimizes their windows, and the input measured goes to another
 * application beside them.
 * - LoadAnimationProbe animates without end in JavaScript: a bar moved and a counter changed on
 *   every frame of requestAnimationFrame, a commit a frame, as a spinner or a chart drawn in
 *   JavaScript is (what App Nap stops for a window nobody sees).
 * - LoadScriptProbe keeps its JavaScript running: 40 ms of computing, then a timer of 0 ms and
 *   again, as a page's script stuck in a loop, with a frame every second to say how far it got.
 */
import * as React from 'react';
import { Text, View } from 'react-native';

const BAR_TRAVEL = 400;
const DOT = 16;

export function PointerTrackProbe() {
  const [at, setAt] = React.useState({ x: -DOT, y: -DOT, moves: 0 });
  return (
    <View
      testID="track"
      onPointerMove={(event) =>
        setAt((was) => ({ x: event.nativeEvent.offsetX, y: event.nativeEvent.offsetY, moves: was.moves + 1 }))
      }
      style={{ flex: 1, backgroundColor: '#ffffff' }}
    >
      <Text style={{ position: 'absolute', left: 12, top: 8, fontSize: 13 }}>{`Moves: ${at.moves}`}</Text>
      <View
        pointerEvents="none"
        style={{
          position: 'absolute',
          left: at.x - DOT / 2,
          top: at.y - DOT / 2,
          width: DOT,
          height: DOT,
          borderRadius: DOT / 2,
          backgroundColor: '#3b6fd8',
        }}
      />
    </View>
  );
}
const SPIN_MS = 40;

export function LoadAnimationProbe() {
  const [frame, setFrame] = React.useState(0);
  React.useEffect(() => {
    let handle = requestAnimationFrame(function next() {
      setFrame((count) => count + 1);
      handle = requestAnimationFrame(next);
    });
    return () => cancelAnimationFrame(handle);
  }, []);
  return (
    <View style={{ flex: 1, padding: 24, backgroundColor: '#f4f6fb' }}>
      <Text testID="frames" style={{ fontSize: 15 }}>{`Frames: ${frame}`}</Text>
      <View
        style={{
          marginTop: 16,
          width: 80,
          height: 24,
          borderRadius: 6,
          backgroundColor: '#3b6fd8',
          transform: [{ translateX: frame % BAR_TRAVEL }],
        }}
      />
    </View>
  );
}

export function LoadScriptProbe() {
  const [rounds, setRounds] = React.useState(0);
  React.useEffect(() => {
    let stopped = false;
    let done = 0;
    let shown = Date.now();
    let timer: ReturnType<typeof setTimeout>;
    const spin = () => {
      const end = Date.now() + SPIN_MS;
      let x = 1;
      while (Date.now() < end) x = (x * 1103515245 + 12345) % 2147483648;
      done += x > -1 ? 1 : 0;
      if (Date.now() - shown >= 1000) {
        shown = Date.now();
        setRounds(done);
      }
      if (!stopped) timer = setTimeout(spin, 0);
    };
    timer = setTimeout(spin, 0);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, []);
  return (
    <View style={{ flex: 1, padding: 24, backgroundColor: '#f4f6fb' }}>
      <Text testID="rounds" style={{ fontSize: 15 }}>{`Rounds: ${rounds}`}</Text>
    </View>
  );
}
