/**
 * An application with a document that may have unsaved changes, asked to quit before the session
 * logs out, restarts or shuts down (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx, decision
 * 5), through `@oxyos/platform`'s `application.onQuitRequest`, as a Mac's application answers
 * `applicationShouldTerminate:`: while its changes are unsaved it cancels the whole end (the
 * session then says it did), once saved it quits. It shows how many times it was asked and why,
 * for scenarios/session-end*.json.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';
import { application, type QuitReason } from '@oxyos/platform';

export default function QuitProbe() {
  const [unsaved, setUnsaved] = React.useState(true);
  const [asked, setAsked] = React.useState(0);
  const [reason, setReason] = React.useState<QuitReason | 'none'>('none');
  // What the handler reads when the session asks: the latest, not what it was registered with.
  const dirty = React.useRef(unsaved);
  dirty.current = unsaved;
  React.useEffect(
    () =>
      application.onQuitRequest((why) => {
        setAsked((count) => count + 1);
        setReason(why);
        return dirty.current ? 'cancel' : 'quit';
      }),
    [],
  );
  return (
    <View testID="quit-probe" style={{ flex: 1, padding: 24, gap: 12, backgroundColor: 'white' }}>
      <Text testID="quit-probe-document">{`Unsaved changes: ${unsaved ? 'yes' : 'no'}`}</Text>
      <Pressable
        testID="quit-probe-save"
        accessibilityRole="button"
        accessibilityLabel="Save"
        onPress={() => setUnsaved(false)}
        style={{ width: 160, height: 40, borderRadius: 8, backgroundColor: '#2f6fe4', justifyContent: 'center', alignItems: 'center' }}
      >
        <Text style={{ color: 'white' }}>Save</Text>
      </Pressable>
      <Text testID="quit-probe-asked">{`Quit requests: ${asked}`}</Text>
      <Text testID="quit-probe-reason">{`Last reason: ${reason}`}</Text>
      <Text>{`Asked by the session: ${application.quitRequests ? 'yes' : 'no'}`}</Text>
    </View>
  );
}
