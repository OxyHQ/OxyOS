/**
 * What React Native's AccessibilityInfo tells an application of the session, for
 * scenarios/accessibility-info.json: whether the screen reader runs (docs/adr/0014-screen-reader.mdx,
 * decision 8) and Reduce Transparency (docs/adr/0019-settings.mdx), as each changes while it runs.
 * - "Screen reader at start" and "Service at start" are what `isScreenReaderEnabled()` and
 *   `isAccessibilityServiceEnabled()` answered as it mounted.
 * - "Screen reader changes" and "Service changes" list every `screenReaderChanged` and
 *   `accessibilityServiceChanged` event, in order, so a duplicate or a missing one shows; after each,
 *   "Screen reader asked" is what `isScreenReaderEnabled()` answers then.
 * - The same for reduce transparency, which "Toggle" (at 420, 16, 140 × 40) asks the session to turn the other way
 *   (`OxyOSSettings`, which the scenario grants this probe as the session grants Settings).
 */
import * as React from 'react';
import { AccessibilityInfo, Pressable, Text, TurboModuleRegistry, View } from 'react-native';

const settings = TurboModuleRegistry.get<{ setPreference(key: string, value: string): number }>('OxyOSSettings');

/** An answer once it came: `-` before. */
function useAnswer(): [string, (value: boolean) => void] {
  const [answer, setAnswer] = React.useState('-');
  return [answer, (value: boolean) => setAnswer(String(value))];
}

/** Every event of `name`, in order, and what `ask` answers after each. */
function useChanges(name: 'screenReaderChanged' | 'accessibilityServiceChanged' | 'reduceTransparencyChanged',
                    ask?: () => Promise<boolean>): [boolean[], string] {
  const [changes, setChanges] = React.useState<boolean[]>([]);
  const [asked, setAsked] = useAnswer();
  React.useEffect(() => {
    const subscription = AccessibilityInfo.addEventListener(name, (value: boolean) => {
      setChanges((all) => [...all, value]);
      ask?.().then(setAsked);
    });
    return () => subscription.remove();
  }, []);
  return [changes, asked];
}

export default function AccessibilityInfoProbe() {
  const [readerAtStart, setReaderAtStart] = useAnswer();
  const [serviceAtStart, setServiceAtStart] = useAnswer();
  const [transparencyAtStart, setTransparencyAtStart] = useAnswer();
  const [readerChanges, readerAsked] = useChanges('screenReaderChanged', AccessibilityInfo.isScreenReaderEnabled);
  const [serviceChanges] = useChanges('accessibilityServiceChanged');
  const [transparencyChanges, transparencyAsked] = useChanges(
    'reduceTransparencyChanged',
    AccessibilityInfo.isReduceTransparencyEnabled,
  );
  React.useEffect(() => {
    AccessibilityInfo.isScreenReaderEnabled().then(setReaderAtStart);
    AccessibilityInfo.isAccessibilityServiceEnabled().then(setServiceAtStart);
    AccessibilityInfo.isReduceTransparencyEnabled().then(setTransparencyAtStart);
  }, []);
  const reduced = transparencyChanges.length > 0 ? transparencyChanges[transparencyChanges.length - 1]
    : transparencyAtStart === 'true';

  return (
    <View style={{ flex: 1, padding: 16, gap: 4, backgroundColor: '#fff' }}>
      <Text>{`Screen reader at start: ${readerAtStart}`}</Text>
      <Text>{`Screen reader changes: ${JSON.stringify(readerChanges)}`}</Text>
      <Text>{`Screen reader asked: ${readerAsked}`}</Text>
      <Text>{`Service at start: ${serviceAtStart}`}</Text>
      <Text>{`Service changes: ${JSON.stringify(serviceChanges)}`}</Text>
      <Text>{`Reduce transparency at start: ${transparencyAtStart}`}</Text>
      <Text>{`Reduce transparency changes: ${JSON.stringify(transparencyChanges)}`}</Text>
      <Text>{`Reduce transparency asked: ${transparencyAsked}`}</Text>
      <Pressable
        testID="toggle"
        accessibilityRole="button"
        accessibilityLabel="Toggle"
        onPress={() => settings?.setPreference('reduce_transparency', reduced ? 'false' : 'true')}
        style={{ position: 'absolute', left: 420, top: 16, width: 140, height: 40, backgroundColor: '#2f6fe4', borderRadius: 8, justifyContent: 'center', alignItems: 'center' }}
      >
        <Text style={{ color: 'white' }}>Toggle</Text>
      </Pressable>
    </View>
  );
}
