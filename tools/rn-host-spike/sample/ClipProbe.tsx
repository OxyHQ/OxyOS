/**
 * Views clipping their children on OxyOS, for scenarios/clip.json and clip-scaled.json: with
 * `overflow: hidden`, Android's ReactViewGroup clips its children to its padding box
 * (BackgroundStyleApplicator.clipToPaddingBox), inside its borders and along its rounded corners,
 * anti-aliased. A square picture in a round button (a user picture), a square child in a round
 * view beside a round background (the reference its edge must match), a child under a border, a
 * turned child, a rounded clip inside another, a rounded clip that is itself scaled, and a card
 * much wider than its corners beside a rounded background of its size.
 */
import * as React from 'react';
import { Image, Pressable, Text, View } from 'react-native';

const red = require('./assets/red.png');

const round = { position: 'absolute', width: 80, height: 80, borderRadius: 40 } as const;
const fill = { position: 'absolute', left: 0, top: 0, width: 80, height: 80 } as const;
// Wider than its corners: its children are drawn directly but in its corners.
const card = { position: 'absolute', top: 200, width: 120, height: 50, borderRadius: 12 } as const;

export default function ClipProbe() {
  const [pressed, setPressed] = React.useState('nothing');
  return (
    <View testID="page" style={{ flex: 1, backgroundColor: '#ffffff' }}>
      <Pressable
        testID="avatar"
        onPress={() => setPressed('the avatar')}
        style={{ ...round, left: 20, top: 20, overflow: 'hidden' }}
      >
        <Image source={red} resizeMode="stretch" style={fill} />
      </Pressable>
      <View testID="reference" style={{ ...round, left: 120, top: 20, backgroundColor: '#3050d0' }} />
      <View testID="clipped" style={{ ...round, left: 220, top: 20, overflow: 'hidden' }}>
        <View style={{ ...fill, backgroundColor: '#3050d0' }} />
      </View>
      <View
        testID="bordered"
        style={{
          ...round,
          left: 20,
          top: 120,
          borderRadius: 24,
          borderWidth: 8,
          borderColor: '#30a050',
          overflow: 'hidden',
        }}
      >
        <View style={{ position: 'absolute', left: -28, top: -28, right: -28, bottom: -28, backgroundColor: '#d08000' }} />
      </View>
      <View testID="turned" style={{ ...round, left: 120, top: 120, overflow: 'hidden' }}>
        <View style={{ ...fill, backgroundColor: '#8030c0', transform: [{ rotate: '45deg' }, { scale: 1.2 }] }} />
      </View>
      <View testID="nested" style={{ ...round, left: 220, top: 120, overflow: 'hidden' }}>
        <View style={{ ...fill, borderRadius: 10, overflow: 'hidden' }}>
          <Image source={red} resizeMode="stretch" style={fill} />
        </View>
      </View>
      <View
        testID="scaled"
        style={{
          position: 'absolute',
          left: 330,
          top: 30,
          width: 40,
          height: 40,
          borderRadius: 20,
          overflow: 'hidden',
          transform: [{ scale: 1.5 }],
        }}
      >
        <View style={{ position: 'absolute', left: 0, top: 0, width: 40, height: 40, backgroundColor: '#3050d0' }} />
      </View>
      <View testID="card" style={{ ...card, left: 120, overflow: 'hidden' }}>
        <View style={{ position: 'absolute', left: -10, top: -10, right: -10, bottom: -10, backgroundColor: '#3050d0' }} />
      </View>
      <View testID="card-reference" style={{ ...card, left: 248, backgroundColor: '#3050d0' }} />
      <Text style={{ position: 'absolute', left: 20, bottom: 20 }}>{`Pressed: ${pressed}`}</Text>
    </View>
  );
}
