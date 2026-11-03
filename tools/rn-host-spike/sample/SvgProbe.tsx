/**
 * SVG on OxyOS, for scenarios/svg.json and svg-scaled.json: an SVG document as an image (React
 * Native's Image, svg-image.h: Skia's SVG module rasterising it at the view's size in pixels) and
 * a malformed one failing, each with what its load events said, and Image.getSize on the first;
 * then react-native-svg's paint servers, clip paths and masks (svg.cpp): a linear gradient, a clip
 * path, a pattern, a mask and a radial gradient, each at a fixed place so the scenarios read
 * their pixels where each colour is solid.
 */
import * as React from 'react';
import { Image, Text, View } from 'react-native';
import Svg, { Circle, ClipPath, Defs, LinearGradient, Mask, Pattern, RadialGradient, Rect, Stop } from 'react-native-svg';

const icon = require('./assets/svg/probe-icon.svg');
const broken = require('./assets/svg/broken.svg');

type Load = {
  onLoad: (event: { nativeEvent: { source: { width: number; height: number } } }) => void;
  onError: (event: { nativeEvent: { error: string } }) => void;
};

const at = (left: number, top: number) => ({ position: 'absolute', left, top }) as const;

export default function SvgProbe() {
  const [said, setSaid] = React.useState<Record<string, string>>({});
  const [size, setSize] = React.useState('pending');
  const events = (name: string): Load => ({
    onLoad: ({ nativeEvent }) =>
      setSaid((all) => ({ ...all, [name]: `loaded ${nativeEvent.source.width}x${nativeEvent.source.height}` })),
    onError: ({ nativeEvent }) => setSaid((all) => ({ ...all, [name]: `error: ${nativeEvent.error}` })),
  });
  React.useEffect(() => {
    Image.getSize(
      Image.resolveAssetSource(icon).uri,
      (width, height) => setSize(`${width}x${height}`),
      (error: Error) => setSize(`failed: ${error.message}`),
    );
  }, []);
  return (
    <View testID="page" style={{ flex: 1, backgroundColor: '#ffffff' }}>
      <Image testID="svg-icon" source={icon} style={{ ...at(16, 16), width: 64, height: 64 }} {...events('svg-icon')} />
      <Image testID="svg-broken" source={broken} style={{ ...at(96, 16), width: 64, height: 64 }} {...events('svg-broken')} />
      <Svg width={120} height={64} style={at(176, 16)}>
        <Defs>
          <LinearGradient id="ramp" x1="0.25" y1="0" x2="0.75" y2="0">
            <Stop offset="0" stopColor="#ff0000" />
            <Stop offset="1" stopColor="#0000ff" />
          </LinearGradient>
        </Defs>
        <Rect width={120} height={64} fill="url(#ramp)" />
      </Svg>
      <Svg width={64} height={64} style={at(312, 16)}>
        <Defs>
          <ClipPath id="disc">
            <Circle cx={32} cy={32} r={24} />
          </ClipPath>
        </Defs>
        <Rect width={64} height={64} fill="#30a050" clipPath="url(#disc)" />
      </Svg>
      <Svg width={120} height={64} style={at(16, 100)}>
        <Defs>
          <Pattern id="checks" patternUnits="userSpaceOnUse" width={16} height={16}>
            <Rect width={8} height={8} fill="#d03030" />
            <Rect x={8} y={8} width={8} height={8} fill="#3050d0" />
          </Pattern>
        </Defs>
        <Rect width={120} height={64} fill="url(#checks)" />
      </Svg>
      <Svg width={64} height={64} style={at(152, 100)}>
        <Defs>
          <Mask id="half">
            <Rect width={32} height={64} fill="#ffffff" />
            <Rect x={32} width={32} height={64} fill="#000000" />
          </Mask>
        </Defs>
        <Rect width={64} height={64} fill="#8030c0" mask="url(#half)" />
      </Svg>
      <Svg width={64} height={64} style={at(232, 100)}>
        <Defs>
          <RadialGradient id="glow" cx="50%" cy="50%" r="50%">
            <Stop offset="0" stopColor="#ffffff" />
            <Stop offset="0.25" stopColor="#ffffff" />
            <Stop offset="1" stopColor="#000000" />
          </RadialGradient>
        </Defs>
        <Circle cx={32} cy={32} r={32} fill="url(#glow)" />
      </Svg>
      <View style={{ ...at(16, 180), gap: 4 }}>
        <Text>{`svg-icon: ${said['svg-icon'] ?? 'pending'}`}</Text>
        <Text>{`svg-broken: ${said['svg-broken'] ?? 'pending'}`}</Text>
        <Text>{`getSize: ${size}`}</Text>
      </View>
    </View>
  );
}
