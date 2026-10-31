/**
 * Expo's pickers on OxyOS (docs/adr/0029-chromium-on-oxyos.mdx, decision 8): expo-file-system's
 * `File.pickFileAsync` and `Directory.pickDirectoryAsync` and expo-document-picker's
 * `getDocumentAsync`, the same calls an application makes on Android and iOS, each asking the
 * system's file chooser; what each answered is said as text, a document's copy read back.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';
import { Directory, File, Paths } from 'expo-file-system';
import * as DocumentPicker from 'expo-document-picker';

const named = (uri: string) => decodeURIComponent(uri.replace(/\/+$/, '').split('/').pop() ?? '');

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
        width: 160,
        height: 36,
        backgroundColor: '#2f6fe4',
        borderRadius: 8,
        justifyContent: 'center',
        alignItems: 'center',
      }}
    >
      <Text style={{ color: 'white' }}>{label}</Text>
    </Pressable>
  );
}

export default function ExpoPickerProbe() {
  const [said, setSaid] = React.useState<Record<string, string>>({});
  const say = (what: string, text: string) => setSaid((before) => ({ ...before, [what]: text }));
  const attempt = (what: string, run: () => Promise<string>) => {
    say(what, 'asking');
    return run().then(
      (text) => say(what, text),
      (error) => say(what, `threw ${error?.code ?? error?.message}`),
    );
  };
  return (
    <View style={{ flex: 1, backgroundColor: 'white' }}>
      <Button
        label="Pick file"
        y={540}
        onPress={() =>
          attempt('Pick file', async () => {
            const picked = await File.pickFileAsync({ mimeTypes: ['image/png'] });
            if (picked.canceled) return 'cancelled';
            const file = picked.result as File;
            return `${file.name} "${file.textSync()}"`;
          })
        }
      />
      <Button
        label="Pick files"
        y={490}
        onPress={() =>
          attempt('Pick files', async () => {
            const picked = await File.pickFileAsync({ multipleFiles: true });
            if (picked.canceled) return 'cancelled';
            return (picked.result as File[]).map((file) => file.name).join(', ');
          })
        }
      />
      <Button
        label="Pick folder"
        y={440}
        onPress={() =>
          attempt('Pick folder', async () => {
            const folder = await Directory.pickDirectoryAsync();
            return `${named(folder.uri)} (${folder.list().length} items)`;
          })
        }
      />
      <Button
        label="Document"
        y={390}
        onPress={() =>
          attempt('Document', async () => {
            const picked = await DocumentPicker.getDocumentAsync({ type: 'image/png' });
            if (picked.canceled) return 'cancelled';
            const asset = picked.assets[0];
            const copied = asset.uri.startsWith(Paths.cache.uri) ? 'a copy in the cache' : 'not copied';
            return `${asset.name} ${asset.size} bytes, ${copied}: ${new File(asset.uri).textSync()}`;
          })
        }
      />
      {['Pick file', 'Pick files', 'Pick folder', 'Document'].map((what, index) => (
        <Text key={what} style={{ position: 'absolute', left: 200, top: 400 + index * 40 }}>
          {`${what}: ${said[what] ?? '-'}`}
        </Text>
      ))}
    </View>
  );
}
