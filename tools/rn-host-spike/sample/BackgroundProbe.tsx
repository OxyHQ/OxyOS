/**
 * An installed application with a login item and background activity
 * (docs/adr/0060-login-items-and-background-activity.mdx), as a chat application keeps its
 * connection: it adds itself to Open at Login and asks to run on in the background once its window
 * closes, through the platform's `application.loginItem` and `application.backgroundActivity`
 * (`@oxyos/platform`),
 * and shows how each stands as the session tells it, the person's changes included.
 */
import * as React from 'react';
import { AppState, Pressable, Text, View } from 'react-native';
import { application, type BackgroundItem } from '@oxyos/platform';
import { useFollowed } from '@oxyos/platform/react';

const { loginItem, backgroundActivity } = application;

function Button({ label, testID, y, onPress }: { label: string; testID: string; y: number; onPress: () => void }) {
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      onPress={onPress}
      style={{ position: 'absolute', left: 20, top: y, width: 230, height: 40, backgroundColor: '#2f6fe4', borderRadius: 8, justifyContent: 'center', alignItems: 'center' }}
    >
      <Text style={{ color: 'white' }}>{label}</Text>
    </Pressable>
  );
}

/** How an item stands, as the session last told it. */
function useStatus(item: BackgroundItem): string | null {
  return useFollowed(item);
}

export default function BackgroundProbe() {
  const login = useStatus(loginItem);
  const background = useStatus(backgroundActivity);
  const [failure, setFailure] = React.useState('-');
  const [state, setState] = React.useState(AppState.currentState);
  React.useEffect(() => {
    const subscription = AppState.addEventListener('change', setState);
    return () => subscription.remove();
  }, []);
  const act = (call: () => Promise<unknown>) => () => {
    setFailure('-');
    call().catch((error) => setFailure(error.reason ? `${error.code} (${error.reason})` : error.code));
  };
  return (
    <View style={{ flex: 1, backgroundColor: 'white' }}>
      <Button label="Add Login Item" testID="login-register" y={20} onPress={act(loginItem.register)} />
      <Button label="Remove Login Item" testID="login-unregister" y={70} onPress={act(loginItem.unregister)} />
      <Button label="Run in Background" testID="background-register" y={120} onPress={act(backgroundActivity.register)} />
      <Button label="Stop Running in Background" testID="background-unregister" y={170} onPress={act(backgroundActivity.unregister)} />
      <Text style={{ position: 'absolute', left: 270, top: 30 }}>{`Login item: ${login ?? '…'}`}</Text>
      <Text style={{ position: 'absolute', left: 270, top: 130 }}>{`Background: ${background ?? '…'}`}</Text>
      <Text style={{ position: 'absolute', left: 270, top: 180 }}>{`Failure: ${failure}`}</Text>
      <Text style={{ position: 'absolute', left: 270, top: 200 }}>{`State: ${state}`}</Text>
    </View>
  );
}
