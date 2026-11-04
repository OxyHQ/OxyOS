/**
 * One colour edge to edge, for the genie (docs/adr/0011-transitions.mdx, step 7) and the scale
 * effect: scenarios/genie*.json minimize it into the dock and look at the output's pixels at chosen
 * points of its funnel, as Salaja recorded a magenta window minimizing. The whole window is one
 * press target, so a press anywhere on it while it goes is told (it never should be).
 */
import * as React from 'react';
import { Pressable, Text } from 'react-native';

export const GENIE_COLOUR = '#c8209a';

export default function GenieProbe() {
  const [presses, setPresses] = React.useState(0);
  return (
    <Pressable
      testID="genie"
      onPress={() => setPresses((count) => count + 1)}
      style={{ flex: 1, backgroundColor: GENIE_COLOUR, padding: 12 }}
    >
      <Text style={{ color: '#ffffff', fontSize: 13 }}>{`Presses ${presses}`}</Text>
    </Pressable>
  );
}
