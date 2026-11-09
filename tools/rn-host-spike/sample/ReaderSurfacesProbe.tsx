/**
 * The screen reader across surfaces (scenarios/reader-surfaces.json, docs/adr/0014-screen-reader.mdx,
 * decision 5 as amended): a window with a popover in a popup of its own (Bloom's Popover, as a Mac's)
 * and a button asking the platform's file chooser, whose window is this window's sheet, beside the
 * session's shell. The reader's cursor goes through the window, then what is its own in front of it
 * (the popover, the sheet), then the shell's menu bar and dock, and back.
 */
import * as React from 'react';
import { View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { Popover, PopoverContent, PopoverTrigger } from '@oxy.so/bloom/popover';
import { Switch } from '@oxy.so/bloom/switch';
import { Muted } from '@oxy.so/bloom/typography';
import { chooser } from '@oxyos/platform';
import { oxyosSurfaces } from './oxyos/platform-surface';

export default function ReaderSurfacesProbe() {
  const [notify, setNotify] = React.useState(false);
  const [attached, setAttached] = React.useState('Nada adjunto');
  const attach = () =>
    chooser.open({ types: ['image/png'], title: 'Elige una imagen' }).then(
      (chosen: string[] | null) => setAttached(chosen ? `Adjuntos ${chosen.length}` : 'Nada adjunto'),
      () => setAttached('Nada adjunto'),
    );
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces} locale="es">
        <View style={{ padding: 24, gap: 12, alignItems: 'flex-start' }}>
          <Button testID="attach" onPress={attach}>
            Adjuntar
          </Button>
          <Popover>
            <PopoverTrigger asChild>
              <Button testID="options">Opciones</Button>
            </PopoverTrigger>
            <PopoverContent label="Opciones de envío" testID="options-popover">
              <Switch testID="notify" accessibilityLabel="Avisarme" checked={notify} onCheckedChange={setNotify} />
              <Button testID="send">Enviar</Button>
            </PopoverContent>
          </Popover>
          <Muted>{attached}</Muted>
        </View>
      </BloomProvider>
    </SafeAreaProvider>
  );
}
