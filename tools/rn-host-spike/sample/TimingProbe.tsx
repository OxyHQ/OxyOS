/**
 * When input happened, as React receives it (input.h's eventTime), checked by
 * scenarios/input-timing.json: two presses count as a double press by when the person made them,
 * however long the application's JavaScript was busy in between. Each press's first is answered
 * by 600 ms of JavaScript holding the thread, as a slow render would; the second arrives behind
 * it. The pad tells two presses apart by their W3C pointer events' `timeStamp` and their touches'
 * `timestamp` (the device's time, on performance.now()'s clock), as a double press must, and
 * also says how far apart JavaScript got to them.
 */
import * as React from 'react';
import { Text, View, type GestureResponderEvent, type PointerEvent } from 'react-native';

const DOUBLE_PRESS_MS = 500;
const BUSY_MS = 600;

type Pair = { pointer: number[]; touch: number[]; handled: number[] };

export default function TimingProbe() {
  const pairs = React.useRef<Pair[]>([]);
  const [said, setSaid] = React.useState<string[]>([]);
  const pair = () => {
    const last = pairs.current[pairs.current.length - 1];
    if (last && last.pointer.length < 2) return last;
    const next: Pair = { pointer: [], touch: [], handled: [] };
    pairs.current.push(next);
    return next;
  };
  const show = () =>
    setSaid(
      pairs.current
        .filter((p) => p.pointer.length === 2 && p.touch.length === 2)
        .map((p, index) => {
          const gap = Math.round(p.pointer[1] - p.pointer[0]);
          const touchGap = Math.round(p.touch[1] - p.touch[0]);
          const handled = p.handled[1] - p.handled[0];
          return `Pair ${index + 1}: ${gap} ms apart (touch ${touchGap} ms), double press: ${
            gap < DOUBLE_PRESS_MS ? 'yes' : 'no'
          }, handled ${handled >= BUSY_MS ? `at least ${BUSY_MS}` : 'under ' + BUSY_MS} ms apart`;
        }),
    );
  const pressed = (event: PointerEvent) => {
    const current = pair();
    current.pointer.push(event.nativeEvent.timeStamp);
    current.handled.push(performance.now());
    if (current.pointer.length === 1) {
      // The first press of a pair is answered slowly: the thread is busy for BUSY_MS.
      const until = performance.now() + BUSY_MS;
      while (performance.now() < until) {
        // busy, as a slow render is
      }
    }
  };
  const touched = (event: GestureResponderEvent) => {
    const current = pairs.current[pairs.current.length - 1];
    current?.touch.push(event.nativeEvent.timestamp);
    show();
  };
  return (
    <View style={{ padding: 16, gap: 12, backgroundColor: '#ffffff', flex: 1 }}>
      <View
        testID="pad"
        onPointerDown={pressed}
        onStartShouldSetResponder={() => true}
        onResponderGrant={touched}
        style={{ width: 240, height: 100, backgroundColor: '#dfe7f5', borderRadius: 12 }}
      />
      <Text style={{ fontSize: 13 }}>{said.length ? said.join('\n') : 'No pair yet'}</Text>
    </View>
  );
}
