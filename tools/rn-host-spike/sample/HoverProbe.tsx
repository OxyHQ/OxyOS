/**
 * A mouse moving over views without a button held (input.h's hover), checked by
 * scenarios/pointer-hover.json: the W3C pointer events React receives, each once. Where the views
 * under the pointer change, pointerout to the view left and pointerleave to the views left,
 * innermost first; pointerover to the view entered and pointerenter to the views entered,
 * outermost first; as it moves, pointermove to the view under it. Each only where a view listens
 * for it, or under a view that captures it (React Native's pointer events processor makes them
 * from the host's pointermove and pointerleave, as for iOS's Fabric).
 *
 * Three zones in a row: `plain` listens for nothing; `card` listens for enter, leave, over and
 * out, and the view inside it (`card-child`) for moves; `capture` captures enter (so the view
 * inside it, `capture-child`, which listens for nothing, gets it too) and listens for leave.
 */
import * as React from 'react';
import { Text, View } from 'react-native';

const ZONE = { width: 160, height: 120, borderRadius: 8, padding: 12 } as const;

export default function HoverProbe() {
  const [card, setCard] = React.useState({ entered: 0, left: 0, over: 0, out: 0, moved: false });
  const [capture, setCapture] = React.useState({ entered: 0, left: 0 });
  return (
    <View style={{ padding: 16, gap: 12, backgroundColor: '#ffffff', flex: 1 }}>
      <View style={{ flexDirection: 'row', gap: 24 }}>
        <View testID="plain" style={{ ...ZONE, backgroundColor: '#e8ecf3' }}>
          <View testID="plain-inner" style={{ flex: 1, backgroundColor: '#d5dbe6', borderRadius: 4 }} />
        </View>
        <View
          testID="card"
          onPointerEnter={() => setCard((c) => ({ ...c, entered: c.entered + 1 }))}
          onPointerLeave={() => setCard((c) => ({ ...c, left: c.left + 1 }))}
          onPointerOver={() => setCard((c) => ({ ...c, over: c.over + 1 }))}
          onPointerOut={() => setCard((c) => ({ ...c, out: c.out + 1 }))}
          style={{ ...ZONE, backgroundColor: '#dfe7f5' }}
        >
          <View
            testID="card-child"
            onPointerMove={() => setCard((c) => (c.moved ? c : { ...c, moved: true }))}
            style={{ flex: 1, backgroundColor: '#c6d4ee', borderRadius: 4 }}
          />
        </View>
        <View
          testID="capture"
          onPointerEnterCapture={() => setCapture((c) => ({ ...c, entered: c.entered + 1 }))}
          onPointerLeave={() => setCapture((c) => ({ ...c, left: c.left + 1 }))}
          style={{ ...ZONE, backgroundColor: '#e9f3e4' }}
        >
          <View testID="capture-child" style={{ flex: 1, backgroundColor: '#cfe6c4', borderRadius: 4 }} />
        </View>
      </View>
      <Text style={{ fontSize: 13 }}>
        {`Card: entered ${card.entered}, left ${card.left}, over ${card.over}, out ${card.out}, moved ${card.moved ? 'yes' : 'no'}`}
      </Text>
      <Text style={{ fontSize: 13 }}>
        {`Capture: entered ${capture.entered}, left ${capture.left}`}
      </Text>
    </View>
  );
}
