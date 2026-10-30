/**
 * An installed application asking for notifications (docs/adr/0030-installing-applications.mdx,
 * decision 8): expo-notifications' own permission request, which the session asks in the system's
 * sheet and the install service keeps, then a post, which the host refuses without the grant.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';
import * as Notifications from 'expo-notifications';

function Button({ label, y, onPress }: { label: string; y: number; onPress: () => void }) {
  return (
    <Pressable
      testID={label.toLowerCase()}
      accessibilityRole="button"
      onPress={onPress}
      style={{ position: 'absolute', left: 20, top: y, width: 160, height: 40, backgroundColor: '#2f6fe4', borderRadius: 8, justifyContent: 'center', alignItems: 'center' }}
    >
      <Text style={{ color: 'white' }}>{label}</Text>
    </Pressable>
  );
}

export default function PermissionProbe() {
  const [status, setStatus] = React.useState('unknown');
  const [posted, setPosted] = React.useState('-');
  const [asked, setAsked] = React.useState(0);
  React.useEffect(() => {
    Notifications.getPermissionsAsync().then((p) => setStatus(p.status));
  }, []);
  return (
    <View style={{ flex: 1, backgroundColor: 'white' }}>
      <Button
        label="Ask"
        y={20}
        onPress={() => {
          setAsked((n) => n + 1);
          Notifications.requestPermissionsAsync().then((p) => setStatus(p.status));
        }}
      />
      <Button
        label="Post"
        y={70}
        onPress={() => {
          setAsked((n) => n + 1);
          Notifications.scheduleNotificationAsync({ content: { title: 'Probe', body: 'Hello' }, trigger: null })
            .then(() => setPosted('posted'))
            .catch(() => setPosted('refused'));
        }}
      />
      <Text style={{ position: 'absolute', left: 20, top: 130 }}>{`Permission: ${status}`}</Text>
      <Text style={{ position: 'absolute', left: 20, top: 150 }}>{`Post: ${posted}`}</Text>
      <Text style={{ position: 'absolute', left: 20, top: 170 }}>{`Asked: ${asked}`}</Text>
    </View>
  );
}
