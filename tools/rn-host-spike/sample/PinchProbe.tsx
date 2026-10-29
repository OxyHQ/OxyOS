/**
 * A touchpad pinch on OxyOS, for scenarios/pinch.json: it reaches the application as Android 14's
 * touchpad stack gives it, two fingers around the pointer (input.h). On the left, a view's own
 * touch handlers show how many fingers are down and how far apart they are, relative to where they
 * started; on the right, react-native-gesture-handler's pinch and rotation, recognized together,
 * show their scale and rotation.
 */
import * as React from 'react';
import { type GestureResponderEvent, Text, View } from 'react-native';
import { Gesture, GestureDetector, GestureHandlerRootView } from 'react-native-gesture-handler';

const area = { position: 'absolute', top: 20, width: 260, height: 300 } as const;

function spread(event: GestureResponderEvent) {
  const [a, b] = event.nativeEvent.touches;
  return a && b ? Math.hypot(b.pageX - a.pageX, b.pageY - a.pageY) : 0;
}

export default function PinchProbe() {
  const [fingers, setFingers] = React.useState({ most: 0, now: 0, spread: 1 });
  const start = React.useRef(0);
  const [gesture, setGesture] = React.useState({ scale: 1, rotation: 0, pinch: 'idle', rotate: 'idle' });
  const touches = (event: GestureResponderEvent) => {
    const now = event.nativeEvent.touches.length;
    if (now === 2 && start.current === 0) start.current = spread(event);
    const ratio = now === 2 && start.current > 0 ? spread(event) / start.current : undefined;
    setFingers((old) => ({ most: Math.max(old.most, now), now, spread: ratio ?? old.spread }));
    if (now === 0) start.current = 0;
  };
  const pinch = Gesture.Pinch()
    .runOnJS(true)
    .onStart(() => setGesture((g) => ({ ...g, pinch: 'active' })))
    .onUpdate((e) => setGesture((g) => ({ ...g, scale: e.scale })))
    .onEnd(() => setGesture((g) => ({ ...g, pinch: 'ended' })));
  const rotation = Gesture.Rotation()
    .runOnJS(true)
    .onStart(() => setGesture((g) => ({ ...g, rotate: 'active' })))
    .onUpdate((e) => setGesture((g) => ({ ...g, rotation: e.rotation })))
    .onEnd(() => setGesture((g) => ({ ...g, rotate: 'ended' })));
  return (
    <GestureHandlerRootView style={{ flex: 1, backgroundColor: '#ffffff' }}>
      <View
        testID="touch-area"
        style={{ ...area, left: 20, backgroundColor: '#e8e8f8' }}
        onTouchStart={touches}
        onTouchMove={touches}
        onTouchEnd={touches}
        onTouchCancel={touches}
      >
        <Text>{`fingers ${fingers.now}, most ${fingers.most}, spread ${fingers.spread.toFixed(2)}`}</Text>
      </View>
      <GestureDetector gesture={Gesture.Simultaneous(pinch, rotation)}>
        <View testID="gesture-area" collapsable={false} style={{ ...area, left: 320, backgroundColor: '#e8f8e8' }}>
          <Text>{`pinch ${gesture.pinch}, scale ${gesture.scale.toFixed(2)}`}</Text>
          <Text>{`rotation ${gesture.rotate}, ${((gesture.rotation * 180) / Math.PI).toFixed(0)} degrees`}</Text>
        </View>
      </GestureDetector>
    </GestureHandlerRootView>
  );
}
