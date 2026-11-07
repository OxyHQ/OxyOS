/**
 * React Native's Image on OxyOS (images.h), for scenarios/images.json: the repository's test JPEG
 * (assets/test-images, scripts/make-test-images.py: 5120 × 2880 with a Display P3 profile) covering
 * its box, an app icon contained in a
 * rounded one and the same icon tinted, and two sources that cannot be shown (a `data:` URI of a
 * PNG cut after its signature, and a missing file), each with what its load events said; and Image.getSize on the wallpaper. Remote
 * images are scenarios/remote-images.json's (RemoteImageProbe).
 */
import * as React from 'react';
import { Image, Text, View } from 'react-native';

const wallpaper = require('../../../assets/test-images/p3-5120x2880.jpg');
const allo = require('../../shell-preview/assets/apps/allo.png');

type Load = {
  onLoad: (event: { nativeEvent: { source: { width: number; height: number } } }) => void;
  onError: (event: { nativeEvent: { error: string } }) => void;
};

export default function ImageProbe() {
  const [said, setSaid] = React.useState<Record<string, string>>({});
  const [size, setSize] = React.useState('pending');
  const events = (name: string): Load => ({
    onLoad: ({ nativeEvent }) =>
      setSaid((all) => ({ ...all, [name]: `loaded ${nativeEvent.source.width}x${nativeEvent.source.height}` })),
    onError: ({ nativeEvent }) => setSaid((all) => ({ ...all, [name]: `error: ${nativeEvent.error}` })),
  });
  React.useEffect(() => {
    Image.getSize(
      Image.resolveAssetSource(wallpaper).uri,
      (width, height) => setSize(`${width}x${height}`),
      (error: Error) => setSize(`failed: ${error.message}`),
    );
  }, []);
  const names = ['wallpaper', 'icon', 'tinted', 'data', 'missing'];
  return (
    <View style={{ flex: 1, padding: 16, gap: 12, backgroundColor: '#fff' }}>
      <View style={{ flexDirection: 'row', gap: 12, alignItems: 'center' }}>
        <Image testID="wallpaper" source={wallpaper} resizeMode="cover" style={{ width: 320, height: 200 }} {...events('wallpaper')} />
        <Image
          testID="icon"
          source={allo}
          resizeMode="contain"
          style={{ width: 96, height: 96, borderRadius: 22, backgroundColor: '#eef1f5' }}
          {...events('icon')}
        />
        <Image testID="tinted" source={allo} resizeMode="contain" tintColor="#294ab4" style={{ width: 48, height: 48 }} {...events('tinted')} />
        <Image testID="data" source={{ uri: 'data:image/png;base64,iVBORw0KGgo=' }} style={{ width: 48, height: 48 }} {...events('data')} />
        <Image testID="missing" source={{ uri: 'file:///nonexistent/oxy.png' }} style={{ width: 48, height: 48 }} {...events('missing')} />
      </View>
      {names.map((name) => (
        <Text key={name}>{`${name}: ${said[name] ?? 'pending'}`}</Text>
      ))}
      <Text>{`getSize: ${size}`}</Text>
    </View>
  );
}
