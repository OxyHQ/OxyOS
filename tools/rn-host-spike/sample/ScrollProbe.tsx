/**
 * Scrolling on OxyOS beyond the pointer's (docs/react-native-android-adaptation.mdx, Scrolling),
 * for scenarios/scroll-keys.json, scroll-reveal.json, scrollbars.json and scroll-commands.json (and
 * their -scaled twins):
 * - `list`: a field, an adjustable stepper, rows that are buttons and a horizontal row of chips
 *   that are buttons too (`chips`, a scroll view in a scroll view), more than it shows: what Tab
 *   brings into view and what the keys scroll with the focus in it.
 * - `main`: the largest, lines that are no controls: the window's main scroll view, which the keys
 *   scroll with the focus in no scroll view; JavaScript's scrollTo (`jump`), scrollToEnd (`tail`)
 *   and flashScrollIndicators (`flash`) move it.
 * - `persistent` (Android's persistentScrollbar) and `hidden` (showsVerticalScrollIndicator false).
 * - `strip`: a horizontal scroll view, the main one across.
 * - `outside`: a button in no scroll view; `sheet` opens a sheet (a Modal) whose view is inside
 *   `main` in the tree, its buttons below what `main` shows: Tab in it must scroll nothing behind it.
 * Every row is white, so the scrollbars' pixels are theirs over white (or the dark window's black
 * in the dark appearance); `status` says what JavaScript last heard.
 */
import * as React from 'react';
import { Modal, Pressable, ScrollView, Text, TextInput, View } from 'react-native';

const ROW = 40;

function Line({ label }: { label: string }) {
  return (
    <View style={{ height: ROW, justifyContent: 'center', paddingLeft: 8 }}>
      <Text style={{ fontSize: 13, color: '#808080' }}>{label}</Text>
    </View>
  );
}

function Button({ id, label, onPress, width }: { id: string; label: string; onPress: () => void; width?: number }) {
  return (
    <Pressable
      testID={id}
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      style={{ height: ROW, width, justifyContent: 'center', paddingLeft: 8 }}
    >
      <Text style={{ fontSize: 13, color: '#808080' }}>{label}</Text>
    </Pressable>
  );
}

export default function ScrollProbe() {
  const [status, setStatus] = React.useState('Nothing yet');
  const [steps, setSteps] = React.useState(0);
  const [sheet, setSheet] = React.useState(false);
  const main = React.useRef<ScrollView>(null);
  const press = (what: string) => () => setStatus(`Pressed ${what}`);
  const lines = (count: number) => Array.from({ length: count }, (_, i) => <Line key={i} label={`Line ${i + 1}`} />);
  return (
    <View testID="page" style={{ flex: 1, backgroundColor: 'transparent' }}>
      <ScrollView testID="list" style={{ position: 'absolute', left: 0, top: 0, width: 200, height: 280 }}>
        <TextInput
          testID="field"
          accessibilityLabel="Field"
          defaultValue="Some text"
          style={{ height: ROW, paddingLeft: 8, fontSize: 13 }}
        />
        <View
          testID="stepper"
          focusable
          accessible
          accessibilityRole="adjustable"
          accessibilityLabel="Stepper"
          accessibilityValue={{ text: String(steps) }}
          accessibilityActions={[{ name: 'increment' }, { name: 'decrement' }]}
          onAccessibilityAction={(event) =>
            setSteps((value) => value + (event.nativeEvent.actionName === 'increment' ? 1 : -1))
          }
          style={{ height: ROW, justifyContent: 'center', paddingLeft: 8 }}
        >
          <Text style={{ fontSize: 13, color: '#808080' }}>{`Stepper ${steps}`}</Text>
        </View>
        {Array.from({ length: 12 }, (_, i) => (
          <Button key={i} id={`row-${i}`} label={`Row ${i}`} onPress={press(`row-${i}`)} />
        ))}
        <ScrollView testID="chips" horizontal style={{ height: ROW }}>
          {Array.from({ length: 6 }, (_, i) => (
            <Button key={i} id={`chip-${i}`} label={`Chip ${i}`} width={80} onPress={press(`chip-${i}`)} />
          ))}
        </ScrollView>
        {Array.from({ length: 2 }, (_, i) => (
          <Button key={i} id={`row-${12 + i}`} label={`Row ${12 + i}`} onPress={press(`row-${12 + i}`)} />
        ))}
      </ScrollView>
      <ScrollView ref={main} testID="main" style={{ position: 'absolute', left: 220, top: 0, width: 240, height: 280 }}>
        {lines(30)}
        <Modal transparent animationType="none" visible={sheet} onRequestClose={() => setSheet(false)}>
          <View
            testID="sheet-panel"
            style={{ position: 'absolute', left: 220, top: 300, flexDirection: 'row', gap: 10, backgroundColor: '#f0f0f0' }}
          >
            <Button id="sheet-a" label="Sheet A" width={100} onPress={press('sheet-a')} />
            <Button id="sheet-b" label="Sheet B" width={100} onPress={press('sheet-b')} />
          </View>
        </Modal>
      </ScrollView>
      <ScrollView
        testID="persistent"
        persistentScrollbar
        style={{ position: 'absolute', left: 480, top: 0, width: 140, height: 130 }}
      >
        {lines(10)}
      </ScrollView>
      <ScrollView
        testID="hidden"
        showsVerticalScrollIndicator={false}
        style={{ position: 'absolute', left: 480, top: 150, width: 140, height: 130 }}
      >
        {lines(10)}
      </ScrollView>
      <ScrollView testID="strip" horizontal style={{ position: 'absolute', left: 0, top: 290, width: 460, height: 50 }}>
        {Array.from({ length: 20 }, (_, i) => (
          <View key={i} style={{ width: 60, height: 50, justifyContent: 'center', paddingLeft: 8 }}>
            <Text style={{ fontSize: 13, color: '#808080' }}>{`Cell ${i}`}</Text>
          </View>
        ))}
      </ScrollView>
      <View style={{ position: 'absolute', left: 0, top: 350, flexDirection: 'row', gap: 10 }}>
        <Button
          id="jump"
          label="Jump"
          width={100}
          onPress={() => {
            setStatus('Pressed jump');
            main.current?.scrollTo({ y: 400, animated: true });
          }}
        />
        <Button
          id="tail"
          label="Tail"
          width={100}
          onPress={() => {
            setStatus('Pressed tail');
            main.current?.scrollToEnd({ animated: false });
          }}
        />
        <Button
          id="flash"
          label="Flash"
          width={100}
          onPress={() => {
            setStatus('Pressed flash');
            main.current?.flashScrollIndicators();
          }}
        />
        <Button
          id="sheet"
          label="Sheet"
          width={100}
          onPress={() => {
            setStatus('Pressed sheet');
            setSheet(true);
          }}
        />
        <Button id="outside" label="Outside" width={100} onPress={press('outside')} />
      </View>
      <Text testID="status" style={{ position: 'absolute', left: 480, top: 300, fontSize: 13, color: '#808080' }}>
        {status}
      </Text>
    </View>
  );
}
