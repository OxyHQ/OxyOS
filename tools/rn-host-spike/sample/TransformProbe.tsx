/**
 * Transforms on OxyOS, for scenarios/transform.json: views moved, scaled and turned by their
 * `transform` are drawn, pressed and measured where the transform puts them, as on Android, with
 * the views inside them transformed too. The box that turns keeps its bounding box and has no
 * child to move, so only its own drawing, its one rounded corner, tells the turn.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';

export default function TransformProbe() {
  const [pressed, setPressed] = React.useState('nothing pressed');
  const [turns, setTurns] = React.useState(0);
  return (
    <View testID="page" style={{ flex: 1, backgroundColor: '#ffffff' }}>
      <Pressable
        testID="moved"
        onPress={() => setPressed('the moved box')}
        style={{
          position: 'absolute',
          left: 20,
          top: 20,
          width: 80,
          height: 60,
          backgroundColor: '#d03030',
          transform: [{ translateX: 120 }, { translateY: 40 }],
        }}
      >
        <View testID="inner" style={{ margin: 20, width: 40, height: 20, backgroundColor: '#3050d0' }} />
      </Pressable>
      <Pressable
        testID="scaled"
        onPress={() => setPressed('the scaled box')}
        style={{
          position: 'absolute',
          left: 260,
          top: 40,
          width: 40,
          height: 40,
          backgroundColor: '#30a050',
          transform: [{ scale: 2 }],
        }}
      >
        <View style={{ position: 'absolute', left: 0, top: 0, width: 10, height: 10, backgroundColor: '#3050d0' }} />
      </Pressable>
      <Pressable
        testID="rotated"
        onPress={() => setPressed('the rotated box')}
        style={{
          position: 'absolute',
          left: 60,
          top: 160,
          width: 60,
          height: 60,
          backgroundColor: '#d08000',
          transform: [{ rotate: '45deg' }],
        }}
      />
      <Pressable
        testID="turning"
        onPress={() => setTurns((n) => n + 1)}
        style={{
          position: 'absolute',
          left: 200,
          top: 170,
          width: 60,
          height: 60,
          backgroundColor: '#8030c0',
          borderTopLeftRadius: 30,
          transform: [{ rotate: `${turns * 90}deg` }],
        }}
      />
      <Text style={{ position: 'absolute', left: 20, bottom: 20 }}>{`Pressed: ${pressed}, turned ${turns}`}</Text>
    </View>
  );
}
