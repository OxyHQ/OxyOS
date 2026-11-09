/**
 * Scroll views in scroll views, across, and a glide meeting input elsewhere
 * (docs/adr/0042-scrolling.mdx, decisions 7 to 9), for scenarios/scroll-nested.json,
 * scroll-across.json and glide-elsewhere.json (and their -scaled twins):
 * - `outer`: a vertical scroll view holding `inner`, a vertical one, and `cells`, a horizontal one:
 *   the wheel and the touchpad's fingers over an inner one move it first and, at its end, the outer
 *   one, as a Mac hands them on.
 * - `across`: a horizontal scroll view the wheel, Shift and the wheel, and the fingers move across.
 * - `glider`: a list a flick leaves gliding. With `arm` pressed first, its next glide opens a
 *   popover (`note`, a compositor popup of the window) as it begins, so that input comes on another
 *   surface of the application while it glides: the wheel, fingers or a click there, which leave it
 *   gliding to its end.
 * Rows are 40 high and cells 80 wide, so every offset falls on the display's pixels at 1 and 1.25;
 * `status` says where each glide of the glider ended and whether the popover is open.
 */
import * as React from 'react';
import { ScrollView, Text, View, type NativeScrollEvent, type NativeSyntheticEvent } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { Popover, PopoverContent, PopoverTrigger } from '@oxy.so/bloom/popover';
import { oxyosSurfaces } from './oxyos/platform-surface';

const ROW = 40;
const CELL = 80;

function Lines({ prefix, count }: { prefix: string; count: number }) {
  return (
    <>
      {Array.from({ length: count }, (_, i) => (
        <View key={i} style={{ height: ROW, justifyContent: 'center', paddingLeft: 8 }}>
          <Text style={{ fontSize: 13, color: '#808080' }}>{`${prefix} ${i + 1}`}</Text>
        </View>
      ))}
    </>
  );
}

function Cells({ prefix, count, height }: { prefix: string; count: number; height: number }) {
  return (
    <>
      {Array.from({ length: count }, (_, i) => (
        <View key={i} style={{ width: CELL, height, justifyContent: 'center', paddingLeft: 8 }}>
          <Text style={{ fontSize: 13, color: '#808080' }}>{`${prefix} ${i + 1}`}</Text>
        </View>
      ))}
    </>
  );
}

export default function NestedScrollProbe() {
  const [armed, setArmed] = React.useState(false);
  const [open, setOpen] = React.useState(false);
  const [glide, setGlide] = React.useState('No glide yet');
  const glides = React.useRef(0);
  const onMomentumScrollBegin = () => {
    if (!armed) return;
    setArmed(false);
    setOpen(true);
  };
  const onMomentumScrollEnd = (event: NativeSyntheticEvent<NativeScrollEvent>) => {
    glides.current += 1;
    setGlide(`Glide ${glides.current} ended at ${Math.round(event.nativeEvent.contentOffset.y)}`);
  };
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces} locale="en">
        <View testID="page" style={{ flex: 1, backgroundColor: '#ffffff' }}>
          <ScrollView testID="outer" style={{ position: 'absolute', left: 0, top: 0, width: 220, height: 300 }}>
            <Lines prefix="Outer" count={2} />
            <ScrollView testID="cells" horizontal style={{ height: ROW }}>
              <Cells prefix="Cell" count={8} height={ROW} />
            </ScrollView>
            <Lines prefix="Above" count={1} />
            <ScrollView testID="inner" style={{ height: 3 * ROW, backgroundColor: '#f4f4f4' }}>
              <Lines prefix="Inner" count={10} />
            </ScrollView>
            <Lines prefix="Below" count={6} />
          </ScrollView>
          <ScrollView testID="across" horizontal style={{ position: 'absolute', left: 240, top: 0, width: 380, height: 60 }}>
            <Cells prefix="Across" count={40} height={60} />
          </ScrollView>
          <ScrollView
            testID="glider"
            style={{ position: 'absolute', left: 240, top: 80, width: 200, height: 240 }}
            onMomentumScrollBegin={onMomentumScrollBegin}
            onMomentumScrollEnd={onMomentumScrollEnd}
          >
            <Lines prefix="Row" count={100} />
          </ScrollView>
          <View style={{ position: 'absolute', left: 460, top: 80, width: 160 }}>
            <Popover
              open={open}
              onOpenChange={(next) => {
                // The trigger arms the glider's next glide; the popover closes as Bloom says.
                if (next) setArmed(true);
                else setOpen(false);
              }}
            >
              <PopoverTrigger asChild>
                <Button testID="arm">Arm</Button>
              </PopoverTrigger>
              <PopoverContent label="Note" testID="note">
                <View style={{ width: 160, height: 100, justifyContent: 'center' }}>
                  <Text style={{ fontSize: 13, color: '#808080' }}>A note over the list</Text>
                </View>
              </PopoverContent>
            </Popover>
          </View>
          <Text testID="status" style={{ position: 'absolute', left: 240, top: 340, fontSize: 13, color: '#808080' }}>
            {`${glide}; ${armed ? 'armed' : 'not armed'}; popover ${open ? 'open' : 'closed'}`}
          </Text>
        </View>
      </BloomProvider>
    </SafeAreaProvider>
  );
}
