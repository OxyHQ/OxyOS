/**
 * React Native's Modal and its `animationType` on OxyOS, for scenarios/modal.json: a modal that
 * slides, one that fades and one that does not animate, each opened by a button on a grey page and
 * closed by its own button or by Escape (onRequestClose).
 */
import * as React from 'react';
import { Modal, Pressable, Text, View } from 'react-native';

type Kind = 'slide' | 'fade' | 'none';

const button = { position: 'absolute', left: 20, width: 120, height: 40, backgroundColor: '#3050d0' } as const;

export default function ModalProbe() {
  const [open, setOpen] = React.useState<Kind | null>(null);
  const close = () => setOpen(null);
  return (
    <View testID="page" style={{ flex: 1, backgroundColor: '#e0e0e0' }}>
      {(['slide', 'fade', 'none'] as const).map((kind, index) => (
        <Pressable key={kind} testID={`open-${kind}`} onPress={() => setOpen(kind)} style={{ ...button, top: 20 + index * 60 }}>
          <Text style={{ color: '#ffffff' }}>{`Open ${kind}`}</Text>
        </Pressable>
      ))}
      {(['slide', 'fade', 'none'] as const).map((kind) => (
        <Modal key={kind} testID={`${kind}-modal`} visible={open === kind} animationType={kind} onRequestClose={close}>
          <View style={{ flex: 1 }}>
            <Pressable testID={`close-${kind}`} onPress={close} style={{ ...button, top: 20 }}>
              <Text style={{ color: '#ffffff' }}>Close</Text>
            </Pressable>
            <Text style={{ position: 'absolute', left: 20, top: 100 }}>{`The ${kind} modal`}</Text>
          </View>
        </Modal>
      ))}
    </View>
  );
}
