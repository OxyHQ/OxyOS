/**
 * Bloom's bottom sheet on OxyOS, for scenarios/sheet.json: its pan to close runs on
 * react-native-gesture-handler (gesture-handler.h, crates/oxyos-gestures). One sheet is Bloom's
 * default, its pan always recognizing alongside the content's native gesture; the other is the
 * scrolling kind (`manualActivation`), whose pan decides from its touch events and activates from
 * a worklet. Each shows how often it was dismissed.
 */
import * as React from 'react';
import { View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { BottomSheet, type BottomSheetRef } from '@oxy.so/bloom/bottom-sheet';
import { Button } from '@oxy.so/bloom/button';
import { Muted, Text } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

function Sheet({ name, manual }: { name: string; manual: boolean }) {
  const sheet = React.useRef<BottomSheetRef>(null);
  const [open, setOpen] = React.useState(false);
  const [dismissed, setDismissed] = React.useState(0);
  return (
    <View testID={`${name}-sheet`} collapsable={false} style={{ gap: 8 }}>
      <Button
        testID={`open-${name}`}
        onPress={() => {
          setOpen(true);
          sheet.current?.present();
        }}
      >
        {`Open ${name}`}
      </Button>
      <Muted>{`${name}: ${open ? 'open' : 'closed'}, dismissed ${dismissed}`}</Muted>
      <BottomSheet
        ref={sheet}
        manualActivation={manual}
        onDismiss={() => {
          setOpen(false);
          setDismissed((count) => count + 1);
        }}
      >
        <View testID={`${name}-content`} collapsable={false} style={{ padding: 24, height: 220 }}>
          <Text>{`The ${name} sheet`}</Text>
        </View>
      </BottomSheet>
    </View>
  );
}

export default function SheetProbe() {
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces}>
        <View style={{ padding: 24, gap: 16 }}>
          <Sheet name="default" manual={false} />
          <Sheet name="scrolling" manual />
        </View>
      </BloomProvider>
    </SafeAreaProvider>
  );
}
