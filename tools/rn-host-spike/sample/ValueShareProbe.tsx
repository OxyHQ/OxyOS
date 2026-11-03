/**
 * A value said as a share of its range (scenarios/value-share.json, docs/adr/0039-keyboard-access.mdx):
 * an element whose `accessibilityValue` gives a number between a minimum and a maximum and no words
 * is said as TalkBack and VoiceOver say a progress bar or a slider, its share of the range ("25 %"),
 * never the raw number; words it gives win. A slider stepped by the arrows says its new share once.
 * Bloom's `Meter` is a meter (a level indicator), said with its share as Bloom formats it.
 */
import * as React from 'react';
import { View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Meter } from '@oxy.so/bloom/stat-bar';
import { oxyosSurfaces } from './oxyos/platform-surface';

const BAR = { height: 8, width: 240, borderRadius: 4, backgroundColor: '#dde3ea' } as const;

export default function ValueShareProbe() {
  // A volume from 10 to 30 in steps of 5: a slider with a range that does not start at 0.
  const [level, setLevel] = React.useState(15);
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces} locale="es">
        <View style={{ padding: 24, gap: 16 }}>
          <View
            testID="download"
            accessible
            accessibilityRole="progressbar"
            accessibilityLabel="Descarga"
            accessibilityValue={{ min: 0, max: 200, now: 50 }}
            style={BAR}
          />
          <View
            testID="level"
            accessible
            focusable
            accessibilityRole="adjustable"
            accessibilityLabel="Nivel"
            accessibilityValue={{ min: 10, max: 30, now: level }}
            accessibilityActions={[{ name: 'increment' }, { name: 'decrement' }]}
            onAccessibilityAction={({ nativeEvent }) => {
              const step = nativeEvent.actionName === 'increment' ? 5 : -5;
              setLevel((current) => Math.min(30, Math.max(10, current + step)));
            }}
            style={BAR}
          />
          <View
            testID="worded"
            accessible
            accessibilityRole="progressbar"
            accessibilityLabel="Copia"
            accessibilityValue={{ min: 0, max: 10, now: 3, text: '3 de 10 archivos' }}
            style={BAR}
          />
          <Meter testID="battery" value={0.64} accessibilityLabel="Batería" width={240} />
          <Meter testID="steps" role="progressbar" value={2} max={8} accessibilityLabel="Pasos" width={240} />
        </View>
      </BloomProvider>
    </SafeAreaProvider>
  );
}
