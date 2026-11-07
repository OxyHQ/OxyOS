import React, { memo, useState } from "react";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { SubtleHover } from "@oxy.so/bloom/subtle-hover";
import { RiArrowLeftSLine } from "@oxy.so/bloom/icons/RiArrowLeftSLine";
import { RiBluetoothLine } from "@oxy.so/bloom/icons/RiBluetoothLine";
import { RiComputerLine } from "@oxy.so/bloom/icons/RiComputerLine";
import { RiHeadphoneLine } from "@oxy.so/bloom/icons/RiHeadphoneLine";
import { RiMicLine } from "@oxy.so/bloom/icons/RiMicLine";
import { RiSpeakerLine } from "@oxy.so/bloom/icons/RiSpeakerLine";
import { RiVolumeUpLine } from "@oxy.so/bloom/icons/RiVolumeUpLine";
import { Box, PressArea, Scroll } from "../primitives";
import type { ShellControls } from "../model";
import { soundDeviceName, type SoundDevice, type SoundKind, type SoundState } from "../sound";
import { ACCENT, FACE, INK, QUIET, card } from "./control-style";
import { LevelSlider, type Icon } from "./LevelSlider";
import { useShellMessages } from "../useShellMessages";

/** The list scrolls past this height, so the panel stays under the top bar on a small display. */
export const SOUND_LIST_MAX_HEIGHT = 360;
export const SOUND_ROW_HEIGHT = 36;

const KIND_ICONS: Record<SoundKind, Icon> = {
  speakers: RiVolumeUpLine,
  headphones: RiHeadphoneLine,
  headset: RiHeadphoneLine,
  hdmi: RiComputerLine,
  line: RiSpeakerLine,
  microphone: RiMicLine,
  other: RiVolumeUpLine,
};

/** A device's icon by what it is; one the system says nothing of, over Bluetooth, Bluetooth's. */
function deviceIcon(device: SoundDevice): Icon {
  if (device.kind === "other" && device.transport === "bluetooth") return RiBluetoothLine;
  return KIND_ICONS[device.kind];
}

/** An output: its icon on a round face (the accent's for the one chosen) and its name, one radio
    button of the list. Pressed, it is chosen; Control Center stays open, as a Mac's does. */
function DeviceRow({ device, choose }: { device: SoundDevice; choose: (id: string) => void }) {
  const messages = useShellMessages();
  const [hovered, setHovered] = useState(false);
  const Glyph = deviceIcon(device);
  return (
    <PressArea
      testID={`sound-output-${device.id}`}
      accessibilityRole="radio"
      accessibilityLabel={soundDeviceName(device, messages)}
      accessibilityState={{ checked: device.selected }}
      onHoverIn={() => setHovered(true)}
      onHoverOut={() => setHovered(false)}
      // The output chosen is chosen already: choosing it again does nothing.
      onPress={() => {
        if (!device.selected) choose(device.id);
      }}
      style={{
        height: SOUND_ROW_HEIGHT,
        flexDirection: "row",
        alignItems: "center",
        gap: 10,
        paddingHorizontal: 6,
        borderRadius: 8,
      }}
    >
      <SubtleHover native active={hovered} style={{ borderRadius: 8 }} />
      <Box
        className="items-center justify-center rounded-full"
        style={{ width: 26, height: 26, backgroundColor: device.selected ? ACCENT : FACE }}
      >
        <Glyph width={15} height={15} fill={device.selected ? "#ffffff" : INK} />
      </Box>
      <Text numberOfLines={1} style={{ flex: 1, color: INK, fontSize: 13 }}>
        {soundDeviceName(device, messages)}
      </Text>
    </PressArea>
  );
}

/**
 * The Control Center's Sound view (docs/adr/0051-sound-devices.mdx), as a Mac's: in place of the
 * modules, the output's volume, then the outputs in the session's order, the one played to
 * checked; "Ajustes de sonido…" at its foot. Choosing an output asks the session to play to it.
 */
export const SoundPanel = memo(function SoundPanel({
  state,
  controls,
  back,
  close,
}: {
  state: SoundState;
  controls: ShellControls;
  /** Back to the modules. */
  back: () => void;
  /** Closes the Control Center (Settings opened). */
  close: () => void;
}) {
  const messages = useShellMessages();
  const words = messages.sound;
  const choose = (id: string) => controls.selectOutput?.(id);
  const volume = controls.levels.output?.available;
  return (
    <Box testID="control-sound-detail" style={{ gap: 10 }}>
      <Box className="flex-row items-center" style={{ height: 32, gap: 6, paddingRight: 4 }}>
        <Button
          testID="sound-back"
          iconOnly
          icon={RiArrowLeftSLine}
          iconSize={18}
          accessibilityLabel={words.back}
          appearance="plain"
          className="border-0 p-0"
          style={{ width: 28, height: 28 }}
          colors={{ background: "transparent", foreground: INK }}
          onPress={back}
        />
        <Text accessibilityRole="header" style={{ flex: 1, color: INK, fontSize: 15, fontWeight: "600" }}>
          {messages.controlCenter.sound}
        </Text>
      </Box>
      {volume && (
        <Box style={card}>
          <LevelSlider
            control="output"
            title={messages.controlCenter.sound}
            icon={RiVolumeUpLine}
            controls={controls}
            mute
            testID="sound-volume"
          />
        </Box>
      )}
      <Box style={{ ...card, paddingHorizontal: 6, paddingVertical: 8 }}>
        <Text
          accessibilityRole="header"
          style={{ color: QUIET, fontSize: 11, fontWeight: "600", paddingHorizontal: 6, marginBottom: 4 }}
        >
          {words.output}
        </Text>
        <Scroll style={{ maxHeight: SOUND_LIST_MAX_HEIGHT }}>
          {state.output.devices.map((device) => (
            <DeviceRow key={device.id} device={device} choose={choose} />
          ))}
        </Scroll>
      </Box>
      {controls.openSoundSettings && (
        <Button
          testID="sound-settings"
          appearance="plain"
          className="border-0"
          style={{ alignSelf: "flex-start", paddingHorizontal: 6, height: 26 }}
          colors={{ background: "transparent", foreground: INK }}
          onPress={() => {
            close();
            controls.openSoundSettings?.();
          }}
        >
          <Text style={{ color: INK, fontSize: 12 }}>{words.settings}</Text>
        </Button>
      )}
    </Box>
  );
});
