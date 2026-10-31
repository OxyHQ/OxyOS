/**
 * When inputs were made, for scenarios/input-times.json: React Native's events carry the time the
 * device reported each input (the compositor's `time_us`), as Android's carry
 * MotionEvent.getEventTime(), not the time they reached JavaScript. The pad tells a double press
 * as a desktop application does, by how far apart its presses were made: within Android's
 * double-tap timeout (ViewConfiguration's 300 ms). Each press-in keeps JavaScript busy for 400 ms,
 * a slow frame, so a second press reaches it that much later than it was made.
 */
import * as React from 'react';
import { Pressable, Text, View, type GestureResponderEvent, type PointerEvent } from 'react-native';

const DOUBLE_TAP_TIMEOUT_MS = 300;
const SLOW_FRAME_MS = 400;

function apart(times: number[]) {
  return times.length < 2 ? 'none' : `${(times[times.length - 1] - times[times.length - 2]).toFixed(1)} ms`;
}

export default function InputTimeProbe() {
  const [presses, setPresses] = React.useState<number[]>([]);
  const [downs, setDowns] = React.useState<number[]>([]);
  const [received, setReceived] = React.useState<number[]>([]);
  const [clock, setClock] = React.useState('unknown');
  const onPressIn = (event: GestureResponderEvent) => {
    const { timestamp } = event.nativeEvent;
    const now = performance.now();
    // An event's time is on performance.now()'s clock, and before it: when the input was made.
    setClock(timestamp > 0 && timestamp <= now && now - timestamp < 10_000 ? 'yes' : `no (${timestamp} at ${now})`);
    setReceived((all) => [...all, now]);
    const until = now + SLOW_FRAME_MS;
    while (performance.now() < until) {
      // A slow frame: the next input waits for it.
    }
  };
  const onPress = (event: GestureResponderEvent) => setPresses((all) => [...all, event.nativeEvent.timestamp]);
  const onPointerDown = (event: PointerEvent) => setDowns((all) => [...all, event.nativeEvent.timeStamp]);
  const between = presses.length >= 2 ? presses[presses.length - 1] - presses[presses.length - 2] : Infinity;
  const kind = presses.length < 2 ? 'single' : between <= DOUBLE_TAP_TIMEOUT_MS ? 'double' : 'two singles';
  return (
    <View style={{ flex: 1, padding: 16, gap: 10, backgroundColor: '#ffffff' }}>
      <Pressable
        testID="pad"
        onPressIn={onPressIn}
        onPress={onPress}
        onPointerDown={onPointerDown}
        style={{ width: 300, height: 120, backgroundColor: '#dfe7f5', borderRadius: 12 }}
      />
      <Text>{`Presses: ${presses.length}, made ${apart(presses)} apart: ${kind}`}</Text>
      <Text>{`Pointer downs made ${apart(downs)} apart`}</Text>
      <Text>{`Second press reached JavaScript behind the slow frame: ${
        received.length < 2 ? 'not yet' : received[received.length - 1] - received[received.length - 2] >= SLOW_FRAME_MS ? 'yes' : 'no'
      }`}</Text>
      <Text>{`Times on performance.now's clock: ${clock}`}</Text>
    </View>
  );
}
