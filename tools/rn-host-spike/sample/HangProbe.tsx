/**
 * An application that hangs (docs/adr/0067-hangs-and-force-quit.mdx), installed as an application
 * of its own for the hang scenarios (scenarios/app-hang*.json): its button runs its JavaScript
 * thread without a pause for as long as it says (30 s, or 4.5 s), as a long synchronous task does, so
 * it reads no event and answers no Ping meanwhile; the compositor shows the wait cursor over it after
 * 2 s and marks it not responding after 5 s. Counts its presses, so a press queued while it hung is
 * seen to arrive once.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';

function spin(ms: number) {
  const until = Date.now() + ms;
  // A busy loop: nothing else of the JavaScript thread runs, as in a hung application.
  while (Date.now() < until) {
    /* hang */
  }
}

export default function HangProbe() {
  const [presses, setPresses] = React.useState(0);
  const press = (ms: number) => () => {
    setPresses((count) => count + 1);
    // After the press is drawn, so the window shows it was taken before it hangs.
    setTimeout(() => spin(ms), 50);
  };
  return (
    <View testID="hang-probe" style={{ flex: 1, backgroundColor: 'white', padding: 20, gap: 12 }}>
      <Text>{`Presses: ${presses}`}</Text>
      <Pressable accessibilityRole="button" onPress={press(30000)} style={{ padding: 12, backgroundColor: '#ddd' }}>
        <Text>Hang 30 Seconds</Text>
      </Pressable>
      <Pressable accessibilityRole="button" onPress={press(4500)} style={{ padding: 12, backgroundColor: '#ddd' }}>
        <Text>Hang 4.5 Seconds</Text>
      </Pressable>
    </View>
  );
}
