/**
 * Remote images on OxyOS (images.h), for scenarios/remote-images.json and its scaled and refused
 * variants. The scenario's store serves cloud.oxy.so's images with their caching headers; this
 * shows, with what each one's load events said:
 *
 * - at once: a PNG of two halves larger than its box (reduced at the view's pixels), an SVG
 *   document (Skia's SVG module, rasterised at them), one the server does not have (404), and
 *   Image.getSize and getSizeWithHeaders of the first two once shown (from the memory cache); and
 *   prefetches two more into the disk cache;
 * - "Revalidate": where each is cached (Image.queryCache), then one prefetched whose response says
 *   `no-cache`: asked again with its ETag, answered 304, drawn from the disk cache;
 * - "Slow" and "Unmount": one arriving slowly, unmounted while it loads (its request cancelled);
 * - "Offline", then "Force cache": the machine offline (the scenario's store takes it away when
 *   this says so), one prefetched drawn by `cache: 'force-cache'` from the disk cache, and one
 *   never fetched failing.
 */
import * as React from 'react';
import { Image, Pressable, Text, View } from 'react-native';

const base = 'https://cloud.oxy.so/probe';
const url = (name: string) => `${base}/${name}`;

type Events = {
  onLoad: (event: { nativeEvent: { source: { width: number; height: number } } }) => void;
  onError: (event: { nativeEvent: { error: string } }) => void;
  onProgress: () => void;
};

const at = (left: number, top: number, size: number, height = size) =>
  ({ position: 'absolute', left, top, width: size, height }) as const;

function Button({ label, onPress }: { label: string; onPress: () => void }) {
  return (
    <Pressable
      testID={label}
      accessibilityRole="button"
      onPress={onPress}
      style={{ padding: 6, backgroundColor: '#eef1f5' }}
    >
      <Text>{label}</Text>
    </Pressable>
  );
}

export default function RemoteImageProbe() {
  const [said, setSaid] = React.useState<Record<string, string>>({});
  const [phase, setPhase] = React.useState<string[]>([]);
  const say = (name: string, text: string) => setSaid((all) => ({ ...all, [name]: text }));
  const events = (name: string): Events => ({
    onLoad: ({ nativeEvent }) => say(name, `loaded ${nativeEvent.source.width}x${nativeEvent.source.height}`),
    onError: ({ nativeEvent }) => say(name, `error: ${nativeEvent.error}`),
    // Listened to, so that progress is sent; counted by the scenario, never drawn.
    onProgress: () => {},
  });
  const has = (step: string) => phase.includes(step);
  const go = (step: string) => setPhase((all) => [...all, step]);
  // Each image's size once it is shown: from the memory cache, never asked of the server again.
  const sized = (name: string, size: Promise<{ width: number; height: number }>) =>
    size
      .then(({ width, height }) => say(`getSize ${name}`, `${width}x${height}`))
      .catch((error: Error) => say(`getSize ${name}`, `failed: ${error.message}`));
  const shown = (name: string, then: () => void): Events => {
    const told = events(name);
    return {
      ...told,
      onLoad: (event) => {
        told.onLoad(event);
        then();
      },
    };
  };
  React.useEffect(() => {
    Promise.all([Image.prefetch(url('etag.png')), Image.prefetch(url('offline.png'))])
      .then((done) => say('prefetch', done.every(Boolean) ? 'done' : 'not done'))
      .catch((error: Error) => say('prefetch', `failed: ${error.message}`));
  }, []);
  const revalidate = () => {
    Image.queryCache([url('split.png'), url('etag.png'), url('offline.png'), url('missing.png')]).then((found) =>
      say(
        'cache',
        ['split.png', 'etag.png', 'offline.png', 'missing.png']
          .map((name) => `${name} ${found[url(name)] ?? 'none'}`)
          .join(', '),
      ),
    );
    go('revalidate');
  };
  const names = ['png', 'svg', 'missing', 'getSize png', 'getSize svg', 'prefetch', 'cache', 'etag', 'slow', 'forced', 'later'];
  return (
    <View testID="page" style={{ flex: 1, backgroundColor: '#ffffff' }}>
      <Image
        testID="png"
        source={{ uri: url('split.png') }}
        style={at(16, 16, 96, 48)}
        {...shown('png', () => sized('png', Image.getSize(url('split.png'))))}
      />
      <Image
        testID="svg"
        source={{ uri: url('icon.svg') }}
        style={at(128, 16, 64)}
        {...shown('svg', () => sized('svg', Image.getSizeWithHeaders(url('icon.svg'), { 'X-Probe': 'size' })))}
      />
      <Image testID="missing" source={{ uri: url('missing.png') }} style={at(208, 16, 48)} {...events('missing')} />
      {has('revalidate') ? (
        <Image testID="etag" source={{ uri: url('etag.png') }} style={at(272, 16, 48)} {...events('etag')} />
      ) : null}
      {has('slow') ? (
        <Image testID="slow" source={{ uri: url('slow.png') }} style={at(336, 16, 48)} {...events('slow')} />
      ) : null}
      {has('forced') ? (
        <>
          <Image
            testID="forced"
            source={{ uri: url('offline.png'), cache: 'force-cache' }}
            style={at(400, 16, 48)}
            {...events('forced')}
          />
          <Image testID="later" source={{ uri: url('later.png') }} style={at(400, 80, 48)} {...events('later')} />
        </>
      ) : null}
      <View style={{ position: 'absolute', left: 16, top: 96, flexDirection: 'row', gap: 8 }}>
        <Button label="Revalidate" onPress={revalidate} />
        <Button label="Slow" onPress={() => go('slow')} />
        <Button
          label="Unmount"
          onPress={() => {
            setPhase((all) => all.filter((done) => done !== 'slow'));
            say('slow', 'unmounted');
          }}
        />
        <Button
          label="Offline"
          onPress={() => {
            say('network', 'offline now');
            go('offline');
          }}
        />
        <Button label="Force cache" onPress={() => go('forced')} />
      </View>
      <View style={{ position: 'absolute', left: 16, top: 136 }}>
        {[...names, 'network'].map((name) => (
          <Text key={name}>{`${name}: ${said[name] ?? 'pending'}`}</Text>
        ))}
      </View>
    </View>
  );
}
