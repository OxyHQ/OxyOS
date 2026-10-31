/**
 * A touchpad's glides and a pan's velocity, for scenarios/touchpad-hold.json and
 * gesture-velocity.json (and their -scaled twins). On the left, a list the touchpad flicks: it says
 * how its glide ended and what its scroll events' `timestamp`s were, as React Native's Android
 * ScrollEvent stamps each when it is dispatched (SystemClock.uptimeMillis(), performance.now()'s
 * clock): the glide's scrolls rising, the momentum's begin and end stamped. On the right, a pan
 * (react-native-gesture-handler) says the velocity it ended with; its second pan's beginning holds
 * JavaScript's thread for 300 ms, as a slow frame does, so its motions reach the host together
 * afterwards: measured by when the hand made them, as Android's VelocityTracker measures them by
 * their MotionEvents' eventTime, its velocity is the first's.
 */
import * as React from 'react';
import { ScrollView, Text, View, type NativeScrollEvent, type NativeSyntheticEvent } from 'react-native';
import { Gesture, GestureDetector, GestureHandlerRootView } from 'react-native-gesture-handler';

const BUSY_MS = 300;
const ROWS = Array.from({ length: 60 }, (_, i) => `Row ${i + 1}`);

type Scroll = NativeSyntheticEvent<NativeScrollEvent>;

export default function GlideProbe() {
  const stamps = React.useRef({ momentum: false, glide: [] as number[], begin: 0, end: 0 });
  const [glide, setGlide] = React.useState('No glide yet');
  const glides = React.useRef(0);
  const pans = React.useRef(0);
  const [pan, setPan] = React.useState('No pan yet');
  const stamped = (t: number) => t > 0 && Math.abs(t - performance.now()) < 1000;
  const onScroll = (event: Scroll) => {
    if (stamps.current.momentum) stamps.current.glide.push(event.nativeEvent.timestamp);
  };
  const onMomentumScrollBegin = (event: Scroll) => {
    stamps.current = { momentum: true, glide: [], begin: event.nativeEvent.timestamp, end: 0 };
  };
  const onMomentumScrollEnd = (event: Scroll) => {
    const s = stamps.current;
    s.momentum = false;
    s.end = event.nativeEvent.timestamp;
    const rising = s.glide.length > 1 && s.glide.every((t, i) => i === 0 || t > s.glide[i - 1]);
    const ordered = s.begin <= (s.glide[0] ?? s.end) && s.end >= (s.glide[s.glide.length - 1] ?? s.begin);
    glides.current += 1;
    setGlide(
      `Glide ${glides.current}: scrolls stamped rising ${rising ? 'yes' : 'no'}, momentum stamped ${
        stamped(s.begin) && stamped(s.end) && ordered ? 'yes' : 'no'
      }, ended at ${Math.round(event.nativeEvent.contentOffset.y)}`,
    );
  };
  const panGesture = Gesture.Pan()
    .runOnJS(true)
    .onBegin(() => {
      pans.current += 1;
      if (pans.current % 2 === 0) {
        // Every second pan: the thread is busy as its motions come, as a slow render is.
        const until = performance.now() + BUSY_MS;
        while (performance.now() < until) {
          // busy
        }
      }
    })
    .onEnd((event) => setPan(`Pan ${pans.current} ended at ${Math.round(event.velocityX / 10) * 10} px/s`));
  return (
    <GestureHandlerRootView style={{ flex: 1, flexDirection: 'row', backgroundColor: '#ffffff' }}>
      <View style={{ width: 300 }}>
        <ScrollView
          testID="list"
          style={{ height: 300 }}
          onScroll={onScroll}
          scrollEventThrottle={1}
          onMomentumScrollBegin={onMomentumScrollBegin}
          onMomentumScrollEnd={onMomentumScrollEnd}
        >
          {ROWS.map((row) => (
            <Text key={row} style={{ height: 40, paddingLeft: 12 }}>
              {row}
            </Text>
          ))}
        </ScrollView>
        <Text style={{ fontSize: 12 }}>{glide}</Text>
      </View>
      <GestureDetector gesture={panGesture}>
        <View testID="pan" collapsable={false} style={{ width: 280, height: 300, backgroundColor: '#e8f0f8' }}>
          <Text>{pan}</Text>
        </View>
      </GestureDetector>
    </GestureHandlerRootView>
  );
}
