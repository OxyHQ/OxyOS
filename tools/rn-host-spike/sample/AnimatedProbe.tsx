/**
 * React Native's Animated with its native driver on OxyOS, for scenarios/native-animated.json. The
 * host runs React Native's own C++ implementation (`NativeAnimatedNodesManager`, as Android does
 * with `cxxNativeAnimatedEnabled`): the animations step on the host's frames and set the views'
 * props between React's commits.
 * - "Start" runs three animations of 600 ms, linear, on the native driver: a box fading in
 *   (opacity 0 to 1), a box moving 200 to the right (translateX 0 to 200) and a box turning from
 *   red to blue (backgroundColor, an interpolation of colours). JavaScript is told once of each
 *   end; "Back" runs them back.
 * - A scroll view's `onScroll` through `Animated.event` on the native driver: a bar moves across as
 *   far as the view is scrolled, set from the scroll events themselves, never from JavaScript.
 * - "Start" and "Back" are animated components made at the top of this module, so Animated's native
 *   module is first asked for while the bundle loads, before anything renders, in every scenario
 *   this bundle runs.
 * - "JS" runs a 600 ms linear animation in JavaScript (`useNativeDriver: false`): a box moving 150
 *   to the right, one commit per frame of `requestAnimationFrame`, which the host paces on its
 *   window's frames, as Android's Choreographer does.
 * - "rAF" asks requestAnimationFrame for three callbacks, a, b and c, and cancels b at once; a cancels
 *   c in the same frame and asks for one more, which runs in the next frame, later: "raf a next".
 *   A frame's time is never ahead of performance.now().
 * - Bloom's Switch: a press turns it on, its thumb moving along the track, and calls
 *   onCheckedChange once.
 */
import * as React from 'react';
import { Animated, Easing, Pressable, Text, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Switch } from '@oxy.so/bloom/switch';
import { oxyosSurfaces } from './oxyos/platform-surface';

const DURATION = 600;
const AnimatedPressable = Animated.createAnimatedComponent(Pressable);

export default function AnimatedProbe() {
  const progress = React.useRef(new Animated.Value(0)).current;
  const opacity = React.useRef(new Animated.Value(0)).current;
  const offset = React.useRef(new Animated.Value(0)).current;
  const scrolled = React.useRef(new Animated.Value(0)).current;
  const moved = React.useRef(new Animated.Value(0)).current;
  const onScroll = React.useMemo(
    () => Animated.event([{ nativeEvent: { contentOffset: { y: scrolled } } }], { useNativeDriver: true }),
    [scrolled],
  );
  const [ended, setEnded] = React.useState<string[]>([]);
  const [checked, setChecked] = React.useState(false);
  const [changes, setChanges] = React.useState(0);

  const run = (to: number) => {
    setEnded([]);
    const animate = (value: Animated.Value, toValue: number, name: string) =>
      Animated.timing(value, { toValue, duration: DURATION, easing: Easing.linear, useNativeDriver: true }).start(
        ({ finished }) => setEnded((names) => [...names, `${name}${finished ? '' : ' (stopped)'}`].sort()),
      );
    animate(opacity, to, 'fade');
    animate(offset, to * 200, 'slide');
    animate(progress, to, 'tint');
  };
  const runInJavaScript = () => {
    setEnded([]);
    Animated.timing(moved, { toValue: 150, duration: DURATION, easing: Easing.linear, useNativeDriver: false }).start(
      ({ finished }) => setEnded([`js${finished ? '' : ' (stopped)'}`]),
    );
  };
  const checkFrames = () => {
    const ran: string[] = [];
    let first = 0;
    requestAnimationFrame((time) => {
      first = time;
      ran.push(time <= performance.now() ? 'a' : 'a (ahead of performance.now)');
      cancelAnimationFrame(c);
      requestAnimationFrame((later) => {
        ran.push(later > first ? 'next' : 'next (not later)');
        setEnded([`raf ${ran.join(' ')}`]);
      });
    });
    const b = requestAnimationFrame(() => ran.push('b'));
    const c = requestAnimationFrame(() => ran.push('c'));
    cancelAnimationFrame(b);
  };
  const tint = progress.interpolate({ inputRange: [0, 1], outputRange: ['#d03030', '#3050d0'] });

  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View testID="page" style={{ flex: 1, backgroundColor: '#ffffff' }}>
        <Animated.View
          testID="fade"
          style={{ position: 'absolute', left: 20, top: 20, width: 80, height: 60, backgroundColor: '#d03030', opacity }}
        />
        <Animated.View
          testID="slide"
          style={{
            position: 'absolute',
            left: 20,
            top: 100,
            width: 60,
            height: 40,
            backgroundColor: '#3050d0',
            transform: [{ translateX: offset }],
          }}
        />
        <Animated.View
          testID="jsbox"
          style={{
            position: 'absolute',
            left: 110,
            top: 20,
            width: 30,
            height: 30,
            backgroundColor: '#8030c0',
            transform: [{ translateX: moved }],
          }}
        />
        <Animated.View
          testID="tint"
          style={{ position: 'absolute', left: 300, top: 20, width: 80, height: 60, backgroundColor: tint }}
        />
        <Animated.ScrollView
          testID="scroller"
          onScroll={onScroll}
          scrollEventThrottle={16}
          style={{ position: 'absolute', left: 300, top: 100, width: 80, height: 60, backgroundColor: '#f0f0f0' }}
        >
          <View style={{ height: 260 }} />
        </Animated.ScrollView>
        <Animated.View
          testID="bar"
          style={{
            position: 'absolute',
            left: 200,
            top: 216,
            width: 20,
            height: 10,
            backgroundColor: '#30a050',
            transform: [{ translateX: scrolled }],
          }}
        />
        <AnimatedPressable
          testID="start"
          onPress={() => run(1)}
          style={{ position: 'absolute', left: 20, top: 170, width: 80, height: 36, backgroundColor: '#e0e0e0' }}
        >
          <Text style={{ margin: 8 }}>Start</Text>
        </AnimatedPressable>
        <AnimatedPressable
          testID="back"
          onPress={() => run(0)}
          style={{ position: 'absolute', left: 110, top: 170, width: 80, height: 36, backgroundColor: '#e0e0e0' }}
        >
          <Text style={{ margin: 8 }}>Back</Text>
        </AnimatedPressable>
        <Pressable
          testID="js-start"
          onPress={runInJavaScript}
          style={{ position: 'absolute', left: 200, top: 170, width: 80, height: 36, backgroundColor: '#e0e0e0' }}
        >
          <Text style={{ margin: 8 }}>JS</Text>
        </Pressable>
        <Pressable
          testID="raf"
          onPress={checkFrames}
          style={{ position: 'absolute', left: 110, top: 210, width: 60, height: 24, backgroundColor: '#e0e0e0' }}
        >
          <Text style={{ marginLeft: 8, marginTop: 2 }}>rAF</Text>
        </Pressable>
        <View style={{ position: 'absolute', left: 300, top: 175 }}>
          <Switch
            testID="switch"
            accessibilityLabel="Probe switch"
            checked={checked}
            onCheckedChange={(next) => {
              setChecked(next);
              setChanges((n) => n + 1);
            }}
          />
        </View>
        <Text style={{ position: 'absolute', left: 20, top: 240 }}>{`Ended: ${ended.join(', ') || 'nothing'}`}</Text>
        <Text style={{ position: 'absolute', left: 20, top: 264 }}>
          {`Switch ${checked ? 'on' : 'off'}, changed ${changes}`}
        </Text>
      </View>
    </BloomProvider>
  );
}
