/**
 * The file chooser for scenarios (docs/adr/0029-chromium-on-oxyos.mdx, decision 8): what the session
 * starts in place of the file manager's picker mode (Stash's, OxyHQ/Stash), through the same
 * adapter, Bloom's `usePlatformFiles().chooser`. It says what it was asked to show and answers with
 * files of the scenario's home (Pictures/a.png and Pictures/b b.png, Documents/), one outside the
 * person's files (which the session refuses, leaving the panel open), or a cancellation.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { usePlatformFiles } from '@oxy.so/bloom/platform-surface';
import { oxyosSurfaces } from './oxyos/platform-surface';

function Button({ label, y, onPress }: { label: string; y: number; onPress: () => void }) {
  return (
    <Pressable
      testID={label.toLowerCase().replace(/ /g, '-')}
      accessibilityRole="button"
      onPress={onPress}
      style={{
        position: 'absolute',
        left: 20,
        top: y,
        width: 200,
        height: 36,
        backgroundColor: '#1d9a5b',
        borderRadius: 8,
        justifyContent: 'center',
        alignItems: 'center',
      }}
    >
      <Text style={{ color: 'white' }}>{label}</Text>
    </Pressable>
  );
}

function Chooser() {
  const files = usePlatformFiles() as any;
  const chooser = files?.chooser;
  const [home, setHome] = React.useState<string | null>(null);
  React.useEffect(() => {
    files?.places().then((places: any[]) => setHome(places.find((place) => place.kind === 'home')?.uri ?? null));
  }, [files]);
  if (!chooser) return <Text>Not a file chooser</Text>;
  const request = chooser.request;
  const at = (relative: string) => `${(home ?? 'file:///nowhere').replace(/\/+$/, '')}/${relative}`;
  return (
    <View style={{ flex: 1, backgroundColor: '#f4f4f5' }}>
      <Button label="Choose a" y={20} onPress={() => chooser.choose([at('Pictures/a.png')])} />
      <Button label="Choose both" y={66} onPress={() => chooser.choose([at('Pictures/a.png'), at('Pictures/b%20b.png')])} />
      <Button label="Choose Pictures" y={112} onPress={() => chooser.choose([`${at('Pictures')}/`])} />
      <Button label="Save here" y={158} onPress={() => chooser.choose([at(`Documents/${encodeURIComponent(request.suggestedName ?? 'Sin titulo')}`)])} />
      <Button label="Choose outside" y={204} onPress={() => chooser.choose(['file:///etc/passwd'])} />
      <Button label="Cancel" y={250} onPress={() => chooser.cancel()} />
      <Text style={{ position: 'absolute', left: 260, top: 24 }}>{`Mode: ${request.mode}${request.multiple ? ' multiple' : ''}${request.directory ? ' folders' : ''}`}</Text>
      <Text style={{ position: 'absolute', left: 260, top: 44 }}>{`Types: ${request.types.join(' ') || 'any'}`}</Text>
      <Text style={{ position: 'absolute', left: 260, top: 64 }}>{`Title: ${request.title ?? '-'}`}</Text>
      <Text style={{ position: 'absolute', left: 260, top: 84 }}>{`Name: ${request.suggestedName ?? '-'}`}</Text>
      <Text style={{ position: 'absolute', left: 260, top: 104 }}>{`For: ${request.application?.name ?? '-'}`}</Text>
    </View>
  );
}

export default function FileChooserFixture() {
  return (
    <BloomProvider platform={oxyosSurfaces}>
      <Chooser />
    </BloomProvider>
  );
}
