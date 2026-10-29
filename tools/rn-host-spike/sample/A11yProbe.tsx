/**
 * What an assistive technology does beyond activating and focusing, for
 * scenarios/accessibility-actions.json (ADR 0003): an adjustable control with increment,
 * decrement and a custom action, whose level a live region says; an announcement; a scroll view
 * it scrolls by a page; a text field it selects in; a dialog it dismisses.
 */
import * as React from 'react';
import { AccessibilityInfo, Modal, Pressable, ScrollView, Text, TextInput, View } from 'react-native';

const row = { height: 40, justifyContent: 'center' as const, paddingHorizontal: 12 };

export default function A11yProbe() {
  const [level, setLevel] = React.useState(5);
  const [dialog, setDialog] = React.useState(false);
  return (
    <View style={{ flex: 1, padding: 16, gap: 10, backgroundColor: '#fff' }}>
      <View
        testID="volume"
        accessible
        accessibilityRole="adjustable"
        accessibilityLabel="Volumen"
        accessibilityActions={[
          { name: 'increment' },
          { name: 'decrement' },
          { name: 'reset', label: 'Reiniciar' },
        ]}
        onAccessibilityAction={({ nativeEvent }) => {
          if (nativeEvent.actionName === 'increment') setLevel((value) => Math.min(10, value + 1));
          if (nativeEvent.actionName === 'decrement') setLevel((value) => Math.max(0, value - 1));
          if (nativeEvent.actionName === 'reset') setLevel(0);
        }}
        style={{ height: 32, backgroundColor: '#e8eefc', justifyContent: 'center', paddingHorizontal: 12 }}
      >
        <View style={{ width: level * 20, height: 8, backgroundColor: '#3060d0' }} />
      </View>
      <Text testID="level" accessibilityLiveRegion="polite">{`Nivel ${level}`}</Text>
      <Pressable
        testID="save"
        accessibilityRole="button"
        onPress={() => AccessibilityInfo.announceForAccessibility('Guardado')}
        style={{ height: 32, justifyContent: 'center', backgroundColor: '#eee' }}
      >
        <Text>Guardar</Text>
      </Pressable>
      <ScrollView testID="list" accessibilityLabel="Lista" style={{ height: 120, flexGrow: 0, backgroundColor: '#f6f6f6' }}>
        {Array.from({ length: 12 }, (_, index) => (
          <View key={index} style={row}>
            <Text>{`Fila ${index + 1}`}</Text>
          </View>
        ))}
      </ScrollView>
      <TextInput
        testID="note"
        accessibilityLabel="Nota"
        defaultValue="hola ñandú"
        style={{ height: 36, borderWidth: 1, borderColor: '#999', paddingHorizontal: 8 }}
      />
      <Pressable
        testID="open"
        accessibilityRole="button"
        onPress={() => setDialog(true)}
        style={{ height: 32, justifyContent: 'center', backgroundColor: '#eee' }}
      >
        <Text>Abrir diálogo</Text>
      </Pressable>
      <Modal transparent visible={dialog} onRequestClose={() => setDialog(false)}>
        <View style={{ flex: 1, alignItems: 'center', justifyContent: 'center', backgroundColor: '#0006' }}>
          <View testID="dialog" accessible accessibilityRole="alert" style={{ padding: 24, backgroundColor: '#fff' }}>
            <Text>Diálogo abierto</Text>
          </View>
        </View>
      </Modal>
    </View>
  );
}
