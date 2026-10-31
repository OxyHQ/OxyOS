/**
 * Frames fast and slow, for the host's frame diagnostics (frame-diagnostics.h), checked by
 * scenarios/frame-diagnostics.json: a press on "fast" counts at once; one on "slow" first holds
 * JavaScript for SLOW_MS, as a slow render would; one on "animate" changes what is drawn for
 * ANIMATED_FRAMES frames of requestAnimationFrame.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';

const SLOW_MS = 120;
const ANIMATED_FRAMES = 90;

function Button({ id, label, onPress }: { id: string; label: string; onPress: () => void }) {
  return (
    <Pressable
      testID={id}
      onPress={onPress}
      style={{ width: 200, height: 60, borderRadius: 10, backgroundColor: '#dfe7f5', justifyContent: 'center' }}
    >
      <Text style={{ textAlign: 'center', fontSize: 15 }}>{label}</Text>
    </Pressable>
  );
}

export default function FrameProbe() {
  const [count, setCount] = React.useState(0);
  const [frame, setFrame] = React.useState(0);
  const slow = () => {
    const until = performance.now() + SLOW_MS;
    while (performance.now() < until) {
      // busy, as a slow render is
    }
    setCount((c) => c + 1);
  };
  const animate = () => {
    let left = ANIMATED_FRAMES;
    const tick = () => {
      setFrame((f) => f + 1);
      if (--left > 0) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  };
  return (
    <View style={{ padding: 16, gap: 12, backgroundColor: '#ffffff', flex: 1 }}>
      <Button id="fast" label="Fast" onPress={() => setCount((c) => c + 1)} />
      <Button id="slow" label="Slow" onPress={slow} />
      <Button id="animate" label="Animate" onPress={animate} />
      <Text style={{ fontSize: 13 }}>{`Count ${count}, frame ${frame}`}</Text>
      <View style={{ width: 20 + (frame % 200), height: 6, backgroundColor: '#3b6fd8' }} />
    </View>
  );
}
