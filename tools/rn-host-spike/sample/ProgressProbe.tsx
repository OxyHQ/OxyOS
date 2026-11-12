/**
 * React Native's ActivityIndicator on OxyOS (tools/rn-host-spike/progress-bar.h; issue #559), for
 * scenarios/progress.json: Android's `AndroidProgressBar`, drawn by the host.
 * - "Guardar": a Bloom `Button` that works when pressed (`loading`: its label hidden, its spinner
 *   over it, busy) until its request to the scenario's server (regress.py's ProbeServer, `/save`)
 *   is answered, which the server does once a step gives the cue `save-done`; then it says what
 *   came of it. The work ends when the scenario says, never on a clock the steps race.
 * - The status line `status`, a polite live region: what the screen reader says of the work.
 * - `sync`: an ActivityIndicator that spins all along, labelled "Sincronizando", so that every step
 *   settles while a spinner animates.
 */
import * as React from 'react';
import { ActivityIndicator, Text, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { oxyosSurfaces } from './oxyos/platform-surface';

// The scenario's server (scenarios/progress.json's `http`).
const SAVE = 'http://127.0.0.1:18656/save';

export default function ProgressProbe() {
  const [loading, setLoading] = React.useState(false);
  const [status, setStatus] = React.useState('Sin guardar');
  const save = () => {
    setLoading(true);
    setStatus('Guardando…');
    fetch(SAVE)
      .then((response) => setStatus(response.ok ? 'Guardado' : 'No guardado'))
      .catch(() => setStatus('No guardado'))
      .finally(() => setLoading(false));
  };
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <View style={{ flex: 1, padding: 24, gap: 16, backgroundColor: '#ffffff' }}>
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 16 }}>
          <Button testID="save" loading={loading} onPress={save}>
            Guardar
          </Button>
          <Text testID="status" accessibilityLiveRegion="polite" style={{ fontSize: 15 }}>
            {status}
          </Text>
        </View>
        <ActivityIndicator testID="sync" accessibilityLabel="Sincronizando" size="large" color="#d03030" />
      </View>
    </BloomProvider>
  );
}
