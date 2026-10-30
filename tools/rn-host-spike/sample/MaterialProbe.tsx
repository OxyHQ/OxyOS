/**
 * expo-blur's blur with a target and React Native's MaskedView, for scenarios/material.json, as
 * Android draws them (and as the lock screen's clock is made of them):
 * - a blur view over a target half black, half white, across their edge: the target under it
 *   blurred (a Gaussian of radius 10: intensity 40 over a reduction factor of 4), its tint over
 *   that, and nothing outside it; the same view with the method "none" is its tint alone;
 * - a red view masked by an opaque circle and a half transparent square: red in the circle, half
 *   red in the square, and nothing elsewhere;
 * - the lock screen's clock (apps/shell's LockClock) at a fixed moment, its digits the material
 *   over a blue target, and a press that moves it on a minute, its last digit turning.
 */
import * as React from 'react';
import { Pressable, View } from 'react-native';
import { BlurTargetView, BlurView } from 'expo-blur';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { oxyosShell } from './oxyos/shell-surface';
import { oxyosSurfaces } from './oxyos/platform-surface';
import { LockClock } from '../../../apps/shell/src/components/LockClock';

function Target({ top, children }: { top: number; children?: React.ReactNode }) {
  return (
    <View style={{ position: 'absolute', left: 0, top, width: 200, height: 100, flexDirection: 'row' }}>
      <View style={{ width: 100, height: 100, backgroundColor: '#000000' }} />
      <View style={{ width: 100, height: 100, backgroundColor: '#ffffff' }} />
      {children}
    </View>
  );
}

export default function MaterialProbe() {
  const target = React.useRef<React.ComponentRef<typeof View>>(null);
  const plain = React.useRef<React.ComponentRef<typeof View>>(null);
  const clockTarget = React.useRef<React.ComponentRef<typeof View>>(null);
  const [now, setNow] = React.useState(() => new Date(2026, 9, 29, 9, 41));
  const { Masked } = oxyosShell;
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View testID="page" style={{ flex: 1, backgroundColor: '#ffffff' }}>
        <BlurTargetView ref={target} style={{ position: 'absolute', left: 0, top: 0, width: 200, height: 100 }}>
          <Target top={0} />
        </BlurTargetView>
        <BlurView
          blurTarget={target}
          blurMethod="dimezisBlurView"
          intensity={40}
          blurReductionFactor={4}
          tint="systemUltraThinMaterial"
          style={{ position: 'absolute', left: 40, top: 20, width: 120, height: 60 }}
        />
        <BlurTargetView ref={plain} style={{ position: 'absolute', left: 0, top: 150, width: 200, height: 100 }}>
          <Target top={0} />
        </BlurTargetView>
        <BlurView
          blurTarget={plain}
          blurMethod="none"
          intensity={40}
          blurReductionFactor={4}
          tint="systemUltraThinMaterial"
          style={{ position: 'absolute', left: 40, top: 170, width: 120, height: 60 }}
        />
        <Masked
          style={{ position: 'absolute', left: 220, top: 20, width: 160, height: 100 }}
          mask={
            <View style={{ flex: 1 }}>
              <View
                style={{ position: 'absolute', left: 0, top: 20, width: 60, height: 60, borderRadius: 30, backgroundColor: '#000000' }}
              />
              <View
                style={{ position: 'absolute', left: 100, top: 20, width: 60, height: 60, backgroundColor: 'rgba(0,0,0,0.5)' }}
              />
            </View>
          }
        >
          <View style={{ flex: 1, backgroundColor: '#d03030' }} />
        </Masked>
        {/* The lock screen's clock over a target of its own, at a fixed moment a press moves on a
            minute: its last digit turns. */}
        <BlurTargetView ref={clockTarget} style={{ position: 'absolute', left: 0, top: 260, right: 0, height: 200 }}>
          <View style={{ flex: 1, backgroundColor: '#3050d0' }} />
        </BlurTargetView>
        <View style={{ position: 'absolute', left: 0, top: 280, right: 0 }}>
          <LockClock now={now} Masked={Masked} blurTarget={clockTarget} />
        </View>
        <Pressable
          testID="next-minute"
          accessibilityRole="button"
          accessibilityLabel="Next minute"
          onPress={() => setNow((then) => new Date(then.getTime() + 60_000))}
          style={{ position: 'absolute', left: 300, top: 440, width: 90, height: 30, backgroundColor: '#30a050' }}
        />
      </View>
    </BloomProvider>
  );
}
