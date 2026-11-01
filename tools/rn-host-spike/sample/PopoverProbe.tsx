/**
 * A popover that is not a menu, from the keyboard (scenarios/keyboard-popover.json,
 * docs/adr/0039-keyboard-access.mdx): a button opens Bloom's Popover holding a field, a switch and
 * a button, as a Mac's popover with controls. Tab goes round its controls and stays in it, Space
 * presses them, Escape closes it and gives the keyboard back to the button that opened it.
 */
import * as React from 'react';
import { View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { Popover, PopoverContent, PopoverTrigger } from '@oxy.so/bloom/popover';
import { Switch } from '@oxy.so/bloom/switch';
import { TextField, TextFieldInput } from '@oxy.so/bloom/text-field';
import { Muted } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

export default function PopoverProbe() {
  const [name, setName] = React.useState('');
  const [notify, setNotify] = React.useState(false);
  const [saved, setSaved] = React.useState(0);
  return (
    <SafeAreaProvider>
    <BloomProvider platform={oxyosSurfaces} locale="es">
      <View style={{ padding: 24, gap: 12, alignItems: 'flex-start' }}>
        <Button testID="before">Antes</Button>
        <Popover>
          <PopoverTrigger asChild>
            <Button testID="options">Opciones</Button>
          </PopoverTrigger>
          <PopoverContent label="Opciones del perfil" testID="options-popover">
            <TextField>
              <TextFieldInput testID="name" label="Nombre" placeholder="Nombre" value={name} onChangeText={setName} />
            </TextField>
            <Switch testID="notify" accessibilityLabel="Avisarme" value={notify} onValueChange={setNotify} />
            <Button testID="save" onPress={() => setSaved((count) => count + 1)}>
              Guardar
            </Button>
          </PopoverContent>
        </Popover>
        <Button testID="after">Después</Button>
        <Muted>{`Guardado ${saved}, aviso ${notify ? 'sí' : 'no'}`}</Muted>
      </View>
    </BloomProvider>
    </SafeAreaProvider>
  );
}
