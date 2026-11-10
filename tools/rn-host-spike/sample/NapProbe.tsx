/**
 * App Nap (docs/adr/0066-quality-of-service.mdx, decision 5), for scenarios/app-nap.json: an
 * application whose requestAnimationFrame loop never ends (counting frames it never draws, so its
 * window settles), showing React Native's AppState as the session presents it and every state it
 * went through. Hidden, its host stops the loop and says `background`; shown behind another
 * application's window, `inactive`; its window the active one, `active`.
 */
import * as React from 'react';
import { AppState, Text, View } from 'react-native';

export default function NapProbe() {
  const [states, setStates] = React.useState<string[]>([AppState.currentState ?? 'unknown']);
  const frames = React.useRef(0);
  React.useEffect(() => {
    const subscription = AppState.addEventListener('change', (state) =>
      setStates((seen) => [...seen, state]),
    );
    return () => subscription.remove();
  }, []);
  React.useEffect(() => {
    let frame = 0;
    const tick = () => {
      frames.current += 1;
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, []);
  return (
    <View testID="nap" style={{ flex: 1, padding: 24, gap: 12, backgroundColor: 'white' }}>
      <Text>{`AppState: ${states[states.length - 1]}`}</Text>
      <Text>{`States: ${states.join(' ')}`}</Text>
    </View>
  );
}
