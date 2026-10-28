/**
 * React Native's Image on OxyOS (images.h), for scenarios/images.json: a wallpaper frame (a
 * 5120 × 2880 JPEG with a Display P3 profile) covering its box, an app icon contained in a
 * rounded one and the same icon tinted, and two sources OxyOS cannot show (a remote image and a
 * missing file), each with what its load events said; and Image.getSize on the wallpaper.
 */
import * as React from 'react';
import { Image, Text, View } from 'react-native';

const wallpaper = require('../../../assets/wallpapers/solar-gradients/frames/frame-08.jpg');
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
  const names = ['wallpaper', 'icon', 'tinted', 'remote', 'missing'];
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
        <Image testID="remote" source={{ uri: 'https://oxy.so/probe.png' }} style={{ width: 48, height: 48 }} {...events('remote')} />
        <Image testID="missing" source={{ uri: 'file:///nonexistent/oxy.png' }} style={{ width: 48, height: 48 }} {...events('missing')} />
      </View>
      {names.map((name) => (
        <Text key={name}>{`${name}: ${said[name] ?? 'pending'}`}</Text>
      ))}
      <Text>{`getSize: ${size}`}</Text>
    </View>
  );
}
