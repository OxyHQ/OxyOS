/**
 * Transforms on OxyOS, for scenarios/transform.json: a box moved by its `transform` is drawn,
 * pressed and measured where the transform puts it, as on Android, and the view moved with it
 * too. Scale, rotation and skew are not drawn yet (react-native-android-adaptation).
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';

export default function TransformProbe() {
  const [pressed, setPressed] = React.useState('nothing pressed');
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
      <Text style={{ position: 'absolute', left: 20, bottom: 20 }}>{`Pressed: ${pressed}`}</Text>
    </View>
  );
}
