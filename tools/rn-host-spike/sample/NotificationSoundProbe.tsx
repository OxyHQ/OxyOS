/**
 * The notification sound (docs/adr/0044-system-sounds.mdx), for scenarios/notification-sound*.json:
 * an application whose expo-notifications handler asks for the sound (`shouldPlaySound`), as Allo's
 * and Mention's do. Avisar posts one now; Ráfaga posts three at once, a burst the session sounds
 * once; Sin sonido posts one whose content says `sound: false`. Each shows how many were posted.
 */
import * as React from 'react';
import { View } from 'react-native';
import * as Notifications from 'expo-notifications';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { Muted } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

export default function NotificationSoundProbe() {
  // Its own handler, set while it runs (the probes share one bundle).
  React.useEffect(() => {
    Notifications.setNotificationHandler({
      handleNotification: async () => ({
        shouldShowBanner: true,
        shouldShowList: true,
        shouldPlaySound: true,
        shouldSetBadge: false,
      }),
    });
    return () => Notifications.setNotificationHandler(null);
  }, []);
  const [posted, setPosted] = React.useState(0);
  const [failed, setFailed] = React.useState('none');
  const post = (body: string, sound?: boolean) =>
    Notifications.scheduleNotificationAsync({
      content: { title: 'Ana', body, ...(sound === undefined ? {} : { sound }) },
      trigger: null,
    }).then(
      () => setPosted((count) => count + 1),
      (error: Error) => setFailed(error.message),
    );
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View style={{ padding: 24, gap: 12 }}>
        <View style={{ flexDirection: 'row', gap: 12 }}>
          <Button testID="notify" onPress={() => post('¿Comemos juntos?')}>
            Avisar
          </Button>
          <Button testID="burst" onPress={() => Promise.all(['Uno', 'Dos', 'Tres'].map((body) => post(body)))}>
            Ráfaga
          </Button>
          <Button testID="silent" onPress={() => post('En silencio', false)}>
            Sin sonido
          </Button>
        </View>
        <Muted>{`Posted: ${posted}`}</Muted>
        <Muted>{`Failed: ${failed}`}</Muted>
      </View>
    </BloomProvider>
  );
}
