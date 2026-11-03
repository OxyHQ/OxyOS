/**
 * Live regions (scenarios/live-regions.json, docs/adr/0039-keyboard-access.mdx): what a window
 * shows as it changes, said as VoiceOver and TalkBack say ARIA's live regions. A Bloom `Field`
 * whose error appears as the name typed is taken (an `alert`: said at once, interrupting, once,
 * while the field keeps the keyboard), a `status` line counting what was saved (polite), and
 * Bloom toasts: a toast's row is a live region that is no element, said by the text it holds,
 * politely, an important one at once.
 */
import * as React from 'react';
import { View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { Field } from '@oxy.so/bloom/field';
import { Text } from '@oxy.so/bloom/typography';
import { TextField, TextFieldInput } from '@oxy.so/bloom/text-field';
import { toast, ToastOutlet } from '@oxy.so/bloom/toast';
import { oxyosSurfaces } from './oxyos/platform-surface';

export default function LiveRegionProbe() {
  const [name, setName] = React.useState('');
  const [saved, setSaved] = React.useState(0);
  const taken = name.trim().toLowerCase() === 'ada';
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces} locale="es">
        <View style={{ padding: 24, gap: 16 }}>
          <Field label="Nombre de usuario" error={taken ? 'Ese nombre ya está en uso' : null}>
            <TextField>
              <TextFieldInput
                testID="user"
                label="Nombre de usuario"
                placeholder="usuario"
                value={name}
                onChangeText={setName}
              />
            </TextField>
          </Field>
          <Text testID="saved" role="status">
            {saved === 1 ? '1 cambio guardado' : `${saved} cambios guardados`}
          </Text>
          <View style={{ flexDirection: 'row', gap: 12 }}>
            <Button testID="save" onPress={() => setSaved((count) => count + 1)}>
              Guardar
            </Button>
            <Button
              testID="copy"
              onPress={() =>
                toast('Copiado', { description: '3 archivos', duration: Infinity })
              }
            >
              Copiar
            </Button>
            <Button
              testID="delete"
              onPress={() =>
                toast.error('No se ha podido borrar', { important: true, duration: Infinity })
              }
            >
              Borrar
            </Button>
          </View>
        </View>
        <ToastOutlet />
      </BloomProvider>
    </SafeAreaProvider>
  );
}
