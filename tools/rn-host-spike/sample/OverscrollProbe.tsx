/**
 * Overscroll on OxyOS, for scenarios/overscroll.json and overscroll-scaled.json: three
 * ScrollViews of rows, as Android 12's ScrollView stretches past its ends (EdgeEffect's stretch)
 * when a touchpad's fingers drag it there or a glide reaches one: `list` (overScrollMode "auto",
 * its content scrolling), `never` (overScrollMode "never") and `short` (its content fitting, so
 * "auto" never stretches it). Rows alternate red and blue, the last green.
 */
import * as React from 'react';
import { ScrollView, View } from 'react-native';

function Rows({ count }: { count: number }) {
  return (
    <>
      {Array.from({ length: count }, (_, i) => (
        <View
          key={i}
          style={{ height: 40, backgroundColor: i === count - 1 ? '#30a050' : i % 2 === 0 ? '#d03030' : '#3050d0' }}
        />
      ))}
    </>
  );
}

const column = { position: 'absolute', top: 20, width: 120, height: 240 } as const;

export default function OverscrollProbe() {
  return (
    <View testID="page" style={{ flex: 1, backgroundColor: '#ffffff' }}>
      <ScrollView testID="list" style={{ ...column, left: 0 }}>
        <Rows count={20} />
      </ScrollView>
      <ScrollView testID="never" overScrollMode="never" style={{ ...column, left: 140 }}>
        <Rows count={20} />
      </ScrollView>
      <ScrollView testID="short" style={{ ...column, left: 280 }}>
        <Rows count={3} />
      </ScrollView>
    </View>
  );
}
