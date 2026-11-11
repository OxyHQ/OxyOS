/**
 * JavaScript held for 500 ms while something moves (docs/adr/0067-the-ui-thread.mdx), for
 * scenarios/stall.json and perf-stall.json: what moves on the UI thread goes on being drawn, a frame
 * each refresh, however long JavaScript's thread is busy, as a slow render holds it.
 * - "Native": a 3 s linear Animated.timing of the box `native` on Animated's native driver; 1 s in,
 *   JavaScript is held 500 ms.
 * - "Worklet": the same with Reanimated's withTiming (the box `worklet`), a worklet on the UI runtime.
 * - The list `list`, which a touchpad's flick glides: its first glide runs free, and 30 ms into each
 *   glide after it (onMomentumScrollBegin) JavaScript is held 500 ms. Each says how far it glided, so
 *   the held one's distance can be told against the free one's.
 */
import * as React from 'react';
import { Animated, Easing, Pressable, ScrollView, Text, View, type NativeScrollEvent, type NativeSyntheticEvent } from 'react-native';
import Reanimated, { Easing as ReanimatedEasing, runOnJS, useAnimatedStyle, useSharedValue, withTiming } from 'react-native-reanimated';

const DURATION = 3000;
const HELD_AFTER_MS = 1000;
const HELD_MS = 500;
const GLIDE_HELD_AFTER_MS = 30;
const ROWS = Array.from({ length: 60 }, (_, i) => `Row ${i + 1}`);

// JavaScript's thread busy, as a slow render keeps it.
function hold() {
  const until = performance.now() + HELD_MS;
  while (performance.now() < until) {
    // busy
  }
}

function Button({ id, label, x, onPress }: { id: string; label: string; x: number; onPress: () => void }) {
  return (
    <Pressable
      testID={id}
      onPress={onPress}
      style={{ position: 'absolute', left: x, top: 20, width: 120, height: 40, borderRadius: 8, backgroundColor: '#dfe7f5', justifyContent: 'center' }}
    >
      <Text style={{ textAlign: 'center', fontSize: 15 }}>{label}</Text>
    </Pressable>
  );
}

type Scroll = NativeSyntheticEvent<NativeScrollEvent>;

export default function StallProbe() {
  const [status, setStatus] = React.useState('Ready');
  const [glide, setGlide] = React.useState('No glide yet');
  const native = React.useRef(new Animated.Value(0)).current;
  const worklet = useSharedValue(0);
  const workletStyle = useAnimatedStyle(() => ({ transform: [{ translateX: worklet.value }] }));
  const glides = React.useRef(0);
  const glideFrom = React.useRef(0);

  const runNative = () => {
    native.setValue(0);
    setStatus('Native running');
    Animated.timing(native, { toValue: 200, duration: DURATION, easing: Easing.linear, useNativeDriver: true }).start(
      ({ finished }) => setStatus(`Native ${finished ? 'ended' : 'stopped'}`),
    );
    setTimeout(hold, HELD_AFTER_MS);
  };
  const workletEnded = (finished: boolean) => setStatus(`Worklet ${finished ? 'ended' : 'stopped'}`);
  const runWorklet = () => {
    setStatus('Worklet running');
    worklet.value = 0;
    worklet.value = withTiming(200, { duration: DURATION, easing: ReanimatedEasing.linear }, (finished) => {
      runOnJS(workletEnded)(finished === true);
    });
    setTimeout(hold, HELD_AFTER_MS);
  };
  const onMomentumScrollBegin = (event: Scroll) => {
    glideFrom.current = event.nativeEvent.contentOffset.y;
    if (glides.current > 0) setTimeout(hold, GLIDE_HELD_AFTER_MS);
  };
  const onMomentumScrollEnd = (event: Scroll) => {
    glides.current += 1;
    const distance = Math.round(event.nativeEvent.contentOffset.y - glideFrom.current);
    setGlide(`Glide ${glides.current}${glides.current > 1 ? ' (JavaScript held)' : ''}: ${distance} px`);
  };

  return (
    <View style={{ flex: 1, backgroundColor: '#ffffff' }}>
      <Button id="run-native" label="Native" x={20} onPress={runNative} />
      <Button id="run-worklet" label="Worklet" x={160} onPress={runWorklet} />
      <Animated.View
        testID="native"
        style={{ position: 'absolute', left: 20, top: 80, width: 40, height: 30, backgroundColor: '#d03030', transform: [{ translateX: native }] }}
      />
      <Reanimated.View
        testID="worklet"
        style={[{ position: 'absolute', left: 20, top: 120, width: 40, height: 30, backgroundColor: '#3050d0' }, workletStyle]}
      />
      <Text style={{ position: 'absolute', left: 300, top: 24, fontSize: 13 }}>{status}</Text>
      <Text style={{ position: 'absolute', left: 300, top: 44, fontSize: 13 }}>{glide}</Text>
      <ScrollView
        testID="list"
        style={{ position: 'absolute', left: 20, top: 170, width: 440, height: 230, backgroundColor: '#f4f6fa' }}
        onMomentumScrollBegin={onMomentumScrollBegin}
        onMomentumScrollEnd={onMomentumScrollEnd}
      >
        {ROWS.map((row) => (
          <View key={row} style={{ height: 40, justifyContent: 'center', paddingLeft: 12, borderBottomWidth: 1, borderColor: '#e1e5ee' }}>
            <Text style={{ fontSize: 14 }}>{row}</Text>
          </View>
        ))}
      </ScrollView>
    </View>
  );
}
