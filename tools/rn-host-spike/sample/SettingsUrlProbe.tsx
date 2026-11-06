/**
 * An application opening the platform's Settings at a page (docs/adr/0019-settings.mdx), for
 * scenarios/settings-url.json: React Native's Linking opens `oxyos-settings://about`, which the
 * session opens in Settings for the person's press, as a Mac's application opens
 * `x-apple.systempreferences:`; any other URL is still refused, saying why.
 */
import * as React from 'react';
import { Linking, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { Muted } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

export default function SettingsUrlProbe() {
  const [said, setSaid] = React.useState('nothing yet');
  const open = (url: string) => {
    Linking.openURL(url).then(
      () => setSaid(`opened ${url}`),
      (error: Error) => setSaid(`refused: ${error.message}`),
    );
  };
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View style={{ padding: 16, gap: 12, alignItems: 'flex-start' }}>
        <Button testID="open-settings" onPress={() => open('oxyos-settings://about')}>
          Open Settings
        </Button>
        <Button testID="open-web" onPress={() => open('https://example.com')}>
          Open the web
        </Button>
        <Muted testID="said">{said}</Muted>
      </View>
    </BloomProvider>
  );
}
