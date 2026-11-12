/**
 * Frames fast and slow, for the host's frame diagnostics (frame-diagnostics.h), checked by
 * scenarios/frame-diagnostics.json: a press on "fast" counts at once; one on "slow" first holds
 * JavaScript for SLOW_MS, as a slow render would; one on "animate" changes what is drawn for
 * ANIMATED_FRAMES frames of requestAnimationFrame; one on "loop", once the scenario's server answers
 * its request for the cue `loop` (regress.py's ProbeServer, `/cue/loop`, given by a step's
 * `cue_before`), changes it on a timer every LOOP_EVERY_MS until the next press on it, so the window always has a frame to
 * draw, as a clock or data arriving gives it one; each press on it says the loop is on or off, the
 * frame showing JavaScript's answer to it (an input is timed to that frame, frame-diagnostics.h) (scenarios/held-frames.json: a frame the
 * compositor holds while the displays are off; requestAnimationFrame would wait for the
 * compositor's answer instead). It starts when the scenario says, never on a clock of its own: a
 * step waits for its window to stop changing, and a loop begun two seconds after the press began
 * before a loaded machine's step had settled, which then never did.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';

const SLOW_MS = 120;
const ANIMATED_FRAMES = 90;
// The scenario's server (scenarios/held-frames.json's `http`): its answer to the cue starts the loop.
const LOOP_CUE = 'http://127.0.0.1:18660/cue/loop';
const LOOP_EVERY_MS = 50;

function Button({ id, label, onPress }: { id: string; label: string; onPress: () => void }) {
  return (
    <Pressable
      testID={id}
      onPress={onPress}
      style={{ width: 200, height: 60, borderRadius: 10, backgroundColor: '#dfe7f5', justifyContent: 'center' }}
    >
      <Text style={{ textAlign: 'center', fontSize: 15 }}>{label}</Text>
    </Pressable>
  );
}

export default function FrameProbe() {
  const [count, setCount] = React.useState(0);
  const [frame, setFrame] = React.useState(0);
  const looping = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const [loopOn, setLoopOn] = React.useState(false);
  const slow = () => {
    const until = performance.now() + SLOW_MS;
    while (performance.now() < until) {
      // busy, as a slow render is
    }
    setCount((c) => c + 1);
  };
  const animate = () => {
    let left = ANIMATED_FRAMES;
    const tick = () => {
      setFrame((f) => f + 1);
      if (--left > 0) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  };
  const loop = () => {
    if (looping.current !== null) {
      clearTimeout(looping.current);
      looping.current = null;
      setLoopOn(false);
      return;
    }
    setLoopOn(true);
    // On, waiting for the cue: a press now stops it before it began.
    const waiting = setTimeout(() => {}, 0);
    looping.current = waiting;
    const tick = () => {
      setFrame((f) => f + 1);
      looping.current = setTimeout(tick, LOOP_EVERY_MS);
    };
    fetch(LOOP_CUE)
      .then(() => {
        if (looping.current === waiting) tick();
      })
      .catch(() => {});
  };
  return (
    <View style={{ padding: 16, gap: 12, backgroundColor: '#ffffff', flex: 1 }}>
      <Button id="fast" label="Fast" onPress={() => setCount((c) => c + 1)} />
      <Button id="slow" label="Slow" onPress={slow} />
      <Button id="animate" label="Animate" onPress={animate} />
      <Text style={{ fontSize: 13 }}>{`Count ${count}, frame ${frame}`}</Text>
      <View style={{ width: 20 + (frame % 200), height: 6, backgroundColor: '#3b6fd8' }} />
      <Button id="loop" label="Loop" onPress={loop} />
      <Text style={{ fontSize: 13 }}>{`Loop ${loopOn ? 'on' : 'off'}`}</Text>
    </View>
  );
}
