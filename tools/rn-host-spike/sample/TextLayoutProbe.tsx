/**
 * How a paragraph breaks into lines and is cut at a number of lines on OxyOS (text.h), checked
 * against what Android's StaticLayout does with the same props, by scenarios/text-layout.json:
 * - lines break where ICU's line break rules allow (after a hyphen, before an opening bracket),
 *   not only at spaces;
 * - a word wider than its line is broken between its characters, as many on each line as fit;
 * - at `numberOfLines`, the last line shown holds the rest of the paragraph, ellipsized at its
 *   end to fit (`ellipsizeMode` tail); at its start or middle when one line is shown (head,
 *   middle); `clip` cuts without an ellipsis;
 * - no line is wider than its box.
 * Each label is a file's name as a file manager's icon shows it (Stash: two centred lines of
 * 12-point text in 84 points, in Bloom's Inter). Under each, what `onTextLayout` gave: each line's text, as
 * Android gives it (an ellipsized line's text is all it stands for), and whether any line
 * overflowed the box.
 */
import * as React from 'react';
import { Text, View, type NativeSyntheticEvent, type TextLayoutEventData } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';

const WIDTH = 84;

type Case = {
  id: string;
  name: string;
  numberOfLines?: number;
  ellipsizeMode?: 'head' | 'middle' | 'tail' | 'clip';
};

const CASES: Case[] = [
  { id: 'hyphens', name: 'claude-setup-prompt.md', numberOfLines: 2 },
  { id: 'unbreakable', name: 'cgqzbklp_400x400.jpg', numberOfLines: 2 },
  { id: 'tail', name: 'Tierras de Valdoria (1).png', numberOfLines: 2 },
  { id: 'brackets', name: '127.0.0.1_8080_(Nest Hub Max).png', numberOfLines: 2 },
  { id: 'unlimited', name: 'faircoin-v3.0.3-linux-x86_64.tar.gz' },
  { id: 'head', name: 'Tierras de Valdoria (1).png', numberOfLines: 1, ellipsizeMode: 'head' },
  { id: 'middle', name: 'Tierras de Valdoria (1).png', numberOfLines: 1, ellipsizeMode: 'middle' },
  { id: 'clip', name: 'Tierras de Valdoria (1).png', numberOfLines: 1, ellipsizeMode: 'clip' },
  { id: 'short', name: 'src.zip', numberOfLines: 2 },
];

function Label({ item }: { item: Case }) {
  const [said, setSaid] = React.useState('not laid out');
  const laidOut = (event: NativeSyntheticEvent<TextLayoutEventData>) => {
    const { lines } = event.nativeEvent;
    const overflows = lines.some((line) => line.x < -0.5 || line.x + line.width > WIDTH + 0.5);
    setSaid(`${lines.map((line) => line.text).join(' | ')}${overflows ? ' (overflows)' : ''}`);
  };
  return (
    <View style={{ flexDirection: 'row', alignItems: 'flex-start', gap: 12 }}>
      <View testID={`box-${item.id}`} style={{ width: WIDTH, backgroundColor: '#eef1f6' }}>
        <Text
          testID={`label-${item.id}`}
          numberOfLines={item.numberOfLines}
          ellipsizeMode={item.ellipsizeMode}
          onTextLayout={laidOut}
          style={{ fontSize: 12, lineHeight: 16, textAlign: 'center', color: '#1d1f24' }}
        >
          {item.name}
        </Text>
      </View>
      <Text style={{ fontSize: 11, color: '#4b5160' }}>{`${item.id}: ${said}`}</Text>
    </View>
  );
}

export default function TextLayoutProbe() {
  return (
    <BloomProvider>
      <View style={{ padding: 12, gap: 8, backgroundColor: '#ffffff', flex: 1 }}>
        {CASES.map((item) => (
          <Label key={item.id} item={item} />
        ))}
      </View>
    </BloomProvider>
  );
}
