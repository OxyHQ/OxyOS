/**
 * A Bloom panel shown and hidden (`panelLayout`'s `visible`), for scenarios/panel-shown.json: the
 * detail panel starts hidden; "Show details" shows it, "Hide details" hides it. Its fade and its
 * track's size are Reanimated's (AppShellPanelLayout): what React commits for the panel and what
 * Reanimated then sets on it must both reach the window, with no later commit of React's to carry
 * them.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { AppShell } from '@oxy.so/bloom/app-shell';
import { oxyosSurfaces } from './oxyos/platform-surface';

function Toggle({ shown, onPress }: { shown: boolean; onPress: () => void }) {
  return (
    <Pressable
      testID="toggle"
      accessibilityRole="button"
      onPress={onPress}
      style={{ margin: 16, marginTop: 120, height: 44, borderRadius: 10, backgroundColor: '#dfe7f5', justifyContent: 'center' }}
    >
      <Text style={{ textAlign: 'center', fontSize: 15 }}>{shown ? 'Hide details' : 'Show details'}</Text>
    </Pressable>
  );
}

export default function PanelProbe() {
  const [shown, setShown] = React.useState(false);
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <AppShell
        testID="probe"
        panelLayout={{
          mode: 'split',
          gap: 8,
          panels: [
            { id: 'list', width: 260, content: <Toggle shown={shown} onPress={() => setShown((s) => !s)} /> },
            {
              id: 'detail',
              visible: shown,
              content: (
                <View testID="detail" style={{ padding: 16 }}>
                  <Text style={{ fontSize: 15 }}>The detail panel</Text>
                </View>
              ),
            },
          ],
        }}
      />
    </BloomProvider>
  );
}
