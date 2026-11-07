import React from "react";
import { Button } from "@oxy.so/bloom/button";
import { Slider } from "@oxy.so/bloom/slider";
import { RiVolumeMuteLine } from "@oxy.so/bloom/icons/RiVolumeMuteLine";
import type { RiWifiLine } from "@oxy.so/bloom/icons/RiWifiLine";
import { Box } from "../primitives";
import { ACCENT, INK } from "./control-style";
import { percent, silent } from "../controls";
import type { ShellControl, ShellControls } from "../model";
import { useShellMessages } from "../useShellMessages";

/** One of Bloom's icons. */
export type Icon = typeof RiWifiLine;

/** A level's row: its icon (a volume's mutes it and unmutes it) and its slider, then `children`
    (the Sound card's chevron). Nothing where the session has not told the level. The Control
    Center's level cards and its Sound view's volume (docs/adr/0025-hardware-keys-and-control-center.mdx). */
export function LevelSlider({
  control,
  title,
  icon,
  controls,
  mute,
  testID = `control-${control}`,
  children,
}: {
  control: ShellControl;
  /** The slider's name. */
  title: string;
  icon: Icon;
  controls: ShellControls;
  /** A volume: its icon mutes it and unmutes it. */
  mute?: boolean;
  /** The slider's; its mute button's is this with `-mute`. */
  testID?: string;
  children?: React.ReactNode;
}) {
  const messages = useShellMessages();
  const level = controls.levels[control];
  if (!level?.available) return null;
  const Glyph = icon;
  const quiet = mute && silent(level);
  return (
    <Box className="flex-row items-center" style={{ gap: 8 }}>
      {mute ? (
        <Button
          testID={`${testID}-mute`}
          iconOnly
          icon={quiet ? RiVolumeMuteLine : Glyph}
          iconSize={16}
          accessibilityLabel={level.muted ? messages.controlCenter.unmute : messages.controlCenter.mute}
          appearance="plain"
          className="border-0 p-0"
          style={{ width: 24, height: 24 }}
          colors={{ background: "transparent", foreground: INK }}
          onPress={() => controls.setMuted?.(control, !level.muted)}
        />
      ) : (
        <Glyph width={16} height={16} fill={INK} />
      )}
      {/* One element for the assistive technology, Bloom's slider: the level as its value
          ("50 %"), a sixteenth up or down (as the keys) as its increment and decrement, which
          the arrows step with the keyboard on it. A drag lands on the keys' finest step, a
          quarter of a sixteenth. */}
      <Box className="flex-1">
        <Slider
          testID={testID}
          value={level.muted ? 0 : level.level * 100}
          min={0}
          max={100}
          step={100 / 64}
          keyboardStep={100 / 16}
          showTooltip={false}
          formatValue={(value) => messages.common.percent(percent(value / 100))}
          accessibilityLabel={title}
          minimumTrackTintColor={ACCENT}
          onValueChange={(value) => controls.setLevel?.(control, value / 100)}
        />
      </Box>
      {children}
    </Box>
  );
}
