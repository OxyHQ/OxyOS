/**
 * Reanimated with the session's Reduce Motion (docs/adr/0019-settings.mdx), for
 * scenarios/reanimated-reduce-motion.json: what an application built on Reanimated sees as the
 * preference changes while it runs.
 * - "Reduced motion: on/off" is Reanimated's `useReducedMotion()`, rendered again when the session
 *   changes the preference.
 * - "Animate" moves the red box 200 to the right, or back, with `withTiming` over 3 s, its
 *   `reduceMotion` `ReduceMotion.System` (Reanimated's default): with Reduce Motion on it jumps to its
 *   end at once, as on Android. Its end is said with how long it took: "at once" (under a second) or
 *   "after its duration" (at least 2.7 s).
 * - "Toggle" asks the session to turn Reduce Motion the other way (`OxyOSSettings`, which the
 *   scenario grants this probe as the session grants Settings): the session keeps it and tells every
 *   application, this one among them.
 */
import * as React from 'react';
import { Pressable, Text, TurboModuleRegistry, View } from 'react-native';
import Animated, {
  ReduceMotion,
  runOnJS,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withTiming,
} from 'react-native-reanimated';

const DURATION = 3000;
const settings = TurboModuleRegistry.get<{ setPreference(key: string, value: string): number }>('OxyOSSettings');

function Button({ label, x, onPress }: { label: string; x: number; onPress: () => void }) {
  return (
    <Pressable
      testID={label.toLowerCase()}
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      style={{ position: 'absolute', left: x, top: 20, width: 140, height: 40, backgroundColor: '#2f6fe4', borderRadius: 8, justifyContent: 'center', alignItems: 'center' }}
    >
      <Text style={{ color: 'white' }}>{label}</Text>
    </Pressable>
  );
}

export default function ReducedMotionProbe() {
  const reduced = useReducedMotion();
  const offset = useSharedValue(0);
  const [runs, setRuns] = React.useState(0);
  const [ended, setEnded] = React.useState('-');
  const style = useAnimatedStyle(() => ({ transform: [{ translateX: offset.value }] }));

  const end = (finished: boolean, startedAt: number) => {
    const took = Date.now() - startedAt;
    const how = took < 1000 ? 'at once' : took >= DURATION * 0.9 ? 'after its duration' : `after ${took} ms`;
    setEnded(`${finished ? 'ended' : 'stopped'} ${how}`);
  };
  const animate = () => {
    const startedAt = Date.now();
    setRuns((n) => n + 1);
    setEnded('running');
    offset.value = withTiming(
      offset.value < 100 ? 200 : 0,
      { duration: DURATION, reduceMotion: ReduceMotion.System },
      (finished) => {
        runOnJS(end)(Boolean(finished), startedAt);
      },
    );
  };

  return (
    <View style={{ flex: 1, backgroundColor: 'white' }}>
      <Button label="Animate" x={20} onPress={animate} />
      <Button label="Toggle" x={180} onPress={() => settings?.setPreference('reduce_motion', reduced ? 'false' : 'true')} />
      <Text style={{ position: 'absolute', left: 20, top: 80 }}>{`Reduced motion: ${reduced ? 'on' : 'off'}`}</Text>
      <Text style={{ position: 'absolute', left: 20, top: 100 }}>{`Run ${runs}: ${ended}`}</Text>
      <Animated.View
        testID="box"
        style={[{ position: 'absolute', left: 20, top: 140, width: 40, height: 40, backgroundColor: '#d03030' }, style]}
      />
    </View>
  );
}
