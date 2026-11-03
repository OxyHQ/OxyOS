/**
 * expo-image's view on OxyOS (expo-image.h, images.h), for scenarios/expo-image.json and its scaled
 * variant. The scenario's store serves cloud.oxy.so's images; this shows, with what each view's
 * events said:
 *
 * - one image of two halves (160 x 80) by each contentFit (cover, contain, fill, scale-down), a
 *   small one (32 x 32) by none and scale-down, one contained at the top left (contentPosition)
 *   and one tinted;
 * - an array of sources of 32, 64 and 72 pixels in a 64-point view: the one nearest the view's
 *   pixels (getBestSource), 64 at 1 and 72 at 1.25;
 * - "Late": a BlurHash placeholder shown until a late image arrives, then cross-dissolving into it
 *   over 400 ms, linearly;
 * - data: URIs (a PNG, an SVG document) and a `require()`d asset;
 * - "Animate": an animated GIF playing its frames with their timing as many times as it loops, and
 *   the same one still (autoplay false);
 * - the cache policies: `none`, `disk` and `memory-disk`, each onLoad saying where its image came
 *   from; "Remount" mounts them again, "Clear memory" and "Clear disk" empty the caches,
 *   "Prefetch" fetches one into the caches before it is shown, "Cache path" asks where one is kept;
 * - an ImageRef from useImage (loadAsync), shown by a view.
 */
import { Image, useImage } from 'expo-image';
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';

const base = 'https://cloud.oxy.so/probe';
const url = (name: string) => `${base}/${name}`;
const allo = require('../../shell-preview/assets/apps/allo.png');

// A 2 x 2 PNG of #20c0a0, and an SVG document filled with #d06010.
const dataPng =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR42mNQOLAAiBggFAAoDgYB3qEAQgAAAABJRU5ErkJggg==';
const dataSvg = `data:image/svg+xml,${encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#d06010"/></svg>',
)}`;

type Load = { cacheType: string; source: { url: string; width: number; height: number; isAnimated: boolean } };

const at = (left: number, top: number, size: number) =>
  ({ position: 'absolute', left, top, width: size, height: size, backgroundColor: '#e0e0e0' }) as const;

function Button({ label, onPress }: { label: string; onPress: () => void }) {
  return (
    <Pressable testID={label} accessibilityRole="button" onPress={onPress} style={{ padding: 4, backgroundColor: '#eef1f5' }}>
      <Text>{label}</Text>
    </Pressable>
  );
}

function RefImage({ say }: { say: (name: string, text: string) => void }) {
  const image = useImage(url('ref.png'), { onError: (error) => say('ref', `error: ${error.message}`) });
  return image ? (
    <Image
      testID="ref"
      source={image}
      style={at(560, 216, 48)}
      onLoad={({ cacheType }: Load) => say('ref', `${image.width}x${image.height} ${cacheType}`)}
    />
  ) : null;
}

export default function ExpoImageProbe() {
  const [said, setSaid] = React.useState<Record<string, string>>({});
  const [mounted, setMounted] = React.useState(0);
  const [prefetched, setPrefetched] = React.useState(false);
  const [animated, setAnimated] = React.useState(false);
  const [late, setLate] = React.useState(false);
  const say = (name: string, text: string) => setSaid((all) => ({ ...all, [name]: text }));
  const events = (name: string) => ({
    testID: name,
    onLoad: ({ cacheType, source }: Load) =>
      say(name, `load ${cacheType} ${source.width}x${source.height}${source.isAnimated ? ' animated' : ''}`),
    onError: ({ error }: { error: string }) => say(name, `error: ${error}`),
  });
  const split = { uri: url('split.png') };
  const small = { uri: url('small.png') };
  const fits = [
    ['cover', split, 'cover'],
    ['contain', split, 'contain'],
    ['fill', split, 'fill'],
    ['scaledown', split, 'scale-down'],
    ['none', small, 'none'],
    ['scaledownsmall', small, 'scale-down'],
  ] as const;
  const names = [
    ...fits.map(([name]) => name),
    'position',
    'tinted',
    'sources',
    'fade',
    'datapng',
    'datasvg',
    'asset',
    'gif',
    'gifstill',
    'none-policy',
    'disk-policy',
    'memory-policy',
    'prefetched',
    'prefetch',
    'cleared',
    'path',
    'ref',
  ];
  return (
    <View testID="page" style={{ flex: 1, backgroundColor: '#ffffff' }}>
      {fits.map(([name, source, fit], index) => (
        <Image key={name} source={source} contentFit={fit} cachePolicy="memory-disk" style={at(16 + index * 80, 16, 64)} {...events(name)} />
      ))}
      <Image
        source={split}
        contentFit="contain"
        contentPosition="top left"
        cachePolicy="memory-disk"
        style={at(496, 16, 64)}
        {...events('position')}
      />
      <Image source={split} contentFit="contain" tintColor="#c02060" cachePolicy="memory-disk" style={at(576, 16, 64)} {...events('tinted')} />
      <Image
        source={[
          { uri: url('s32.png'), width: 32, height: 32 },
          { uri: url('s64.png'), width: 64, height: 64 },
          { uri: url('s72.png'), width: 72, height: 72 },
        ]}
        style={at(16, 104, 64)}
        onLoad={({ source }: Load) => say('sources', `${source.url.slice(base.length + 1)} ${source.width}x${source.height}`)}
        testID="sources"
      />
      {late ? (
        <Image
          source={url('fade.png')}
          placeholder={{ blurhash: '008ii8' }}
          transition={{ duration: 400, timing: 'linear' }}
          cachePolicy="none"
          style={at(96, 104, 96)}
          {...events('fade')}
        />
      ) : null}
      <Image source={dataPng} style={at(208, 104, 48)} {...events('datapng')} />
      <Image source={dataSvg} style={at(272, 104, 48)} {...events('datasvg')} />
      <Image source={allo} contentFit="contain" style={at(336, 104, 48)} alt="Allo" {...events('asset')} />
      {animated ? (
        <>
          <Image source={url('frames.gif')} style={at(400, 104, 48)} {...events('gif')} />
          <Image source={url('frames.gif')} autoplay={false} style={at(464, 104, 48)} {...events('gifstill')} />
        </>
      ) : null}
      <View key={mounted} style={{ position: 'absolute', left: 0, top: 0 }}>
        <Image source={url('c-none.png')} cachePolicy="none" style={at(16, 216, 48)} {...events('none-policy')} />
        <Image source={url('c-disk.png')} cachePolicy="disk" style={at(80, 216, 48)} {...events('disk-policy')} />
        <Image source={url('c-memory.png')} cachePolicy="memory-disk" style={at(144, 216, 48)} {...events('memory-policy')} />
        {prefetched ? (
          <Image source={url('c-prefetch.png')} cachePolicy="memory-disk" style={at(208, 216, 48)} {...events('prefetched')} />
        ) : null}
      </View>
      <RefImage say={say} />
      <View style={{ position: 'absolute', left: 16, top: 280, flexDirection: 'row', gap: 6 }}>
        <Button label="Remount" onPress={() => setMounted((n) => n + 1)} />
        <Button label="Late" onPress={() => setLate(true)} />
        <Button label="Animate" onPress={() => setAnimated(true)} />
        <Button
          label="Clear memory"
          onPress={() => Image.clearMemoryCache().then((done) => say('cleared', `memory ${done}`))}
        />
        <Button label="Clear disk" onPress={() => Image.clearDiskCache().then((done) => say('cleared', `disk ${done}`))} />
        <Button
          label="Prefetch"
          onPress={() =>
            Image.prefetch(url('c-prefetch.png'), 'memory-disk').then((done) => {
              say('prefetch', `${done}`);
              setPrefetched(true);
            })
          }
        />
        <Button
          label="Cache path"
          onPress={() =>
            Promise.all([Image.getCachePathAsync(url('c-disk.png')), Image.getCachePathAsync(url('c-none.png'))]).then(
              ([disk, none]) => say('path', `disk ${disk ? 'kept' : 'none'}, none ${none ? 'kept' : 'none'}`),
            )
          }
        />
      </View>
      {[names.slice(0, 13), names.slice(13)].map((column, index) => (
        <View key={index} style={{ position: 'absolute', left: 16 + index * 330, top: 320 }}>
          {column.map((name) => (
            <Text key={name}>{`${name}: ${said[name] ?? 'pending'}`}</Text>
          ))}
        </View>
      ))}
    </View>
  );
}
