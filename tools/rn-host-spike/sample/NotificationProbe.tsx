/**
 * expo-notifications on OxyOS (sample/oxyos/expo-adapter.js, docs/adr/0026-notifications.mdx), for
 * scenarios/notifications.json: Avisar posts a notification now with scheduleNotificationAsync, in a
 * category whose action is a reply, as a chat application posts one; the response listener counts
 * the responses it hears and shows the last (its action, the reply typed, the data posted with it).
 * Withdraw dismisses every one this application presented.
 */
import * as React from 'react';
import { View } from 'react-native';
import * as Notifications from 'expo-notifications';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { Muted } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

const short = (action: string) =>
  action === Notifications.DEFAULT_ACTION_IDENTIFIER ? 'default' : action;

export default function NotificationProbe() {
  const [posted, setPosted] = React.useState('nothing yet');
  const [responses, setResponses] = React.useState(0);
  const [last, setLast] = React.useState('none');
  React.useEffect(() => {
    Notifications.setNotificationCategoryAsync('chat', [
      {
        identifier: 'reply',
        buttonTitle: 'Responder',
        textInput: { submitButtonTitle: 'Enviar', placeholder: 'Mensaje' },
      },
    ]);
    const subscription = Notifications.addNotificationResponseReceivedListener((response) => {
      setResponses((count) => count + 1);
      const data = response.notification.request.content.data as { chat?: number };
      setLast(`${short(response.actionIdentifier)} ${response.userText ?? '-'} chat ${data?.chat ?? '?'}`);
    });
    return () => subscription.remove();
  }, []);
  const notify = () =>
    Notifications.scheduleNotificationAsync({
      content: { title: 'Ana', body: '¿Comemos juntos?', data: { chat: 7 }, categoryIdentifier: 'chat' },
      trigger: null,
    }).then(
      () => setPosted('posted'),
      (error: Error) => setPosted(error.message),
    );
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View style={{ padding: 24, gap: 12 }}>
        <View style={{ flexDirection: 'row', gap: 12 }}>
          <Button testID="notify" onPress={notify}>
            Avisar
          </Button>
          <Button testID="withdraw" onPress={() => Notifications.dismissAllNotificationsAsync()}>
            Withdraw
          </Button>
        </View>
        <Muted>{`Posted: ${posted}`}</Muted>
        <Muted>{`Responses: ${responses}`}</Muted>
        <Muted>{`Last: ${last}`}</Muted>
      </View>
    </BloomProvider>
  );
}
