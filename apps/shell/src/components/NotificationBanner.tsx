import React, { memo, useEffect, useRef, useState } from "react";
import { Button, CloseButton } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { TextField, TextFieldInput } from "@oxy.so/bloom/text-field";
import { Box, PressArea } from "../primitives";
import { AppIcon } from "../apps";
import type { ShellApplication, ShellNotification } from "../model";
import { ago, applicationOf, spoken } from "../notifications";
import { useShellMessages } from "../useShellMessages";
import { BloomColorScope } from "@oxy.so/bloom/theme";
import { useAccent } from "./accent";
import { GlassMaterial, useGlass } from "./GlassMaterial";

// A banner's glass's fill, and its shape within its 1-pixel border, which the material under it
// takes.
const BANNER_GLASS = "#f5f7fbf2";
const BANNER_SHAPE = { borderRadius: 17 } as const;

export const BANNER_WIDTH = 360;

/**
 * One notification as a card (docs/adr/0026-notifications.mdx), as a Mac's: the application's own
 * icon and name, when, its title, subtitle and body, and its actions. Pressing the card opens it
 * in its application (`activate(0)`); an action is `activate(n)`, and a reply action first opens a
 * field whose text goes with it. The card is one button for a screen reader, named by whose it is
 * and what it says; its actions and close are buttons of their own, its close named by the
 * notification it closes ("Borrar la notificación de Allo: Ana"), as the screen reader reads it on
 * its own, after the card.
 */
export const NotificationCard = memo(function NotificationCard({
  notification,
  applications,
  now,
  activate,
  close,
  closeAction,
  onHeld,
  testID,
}: {
  notification: ShellNotification;
  applications: readonly ShellApplication[];
  now: Date;
  activate: (action: number, reply?: string) => void;
  close: () => void;
  /** What its close does, the verb its button is named by ("Borrar", "Ocultar"). */
  closeAction: string;
  /** Whether the person has it: the pointer over it, or a reply being typed in it (its field is
      what gives a banner the keyboard). A banner stays while it is held, as a Mac's stays while
      the pointer is on it, and leaves once it is let go. */
  onHeld?: (held: boolean) => void;
  testID: string;
}) {
  const bannerGlass = useGlass(BANNER_GLASS);
  const accentPreset = useAccent();
  const messages = useShellMessages();
  const words = messages.notifications;
  const { name, app } = applicationOf(notification, applications);
  const [replying, setReplying] = useState<number | null>(null);
  const [reply, setReply] = useState("");
  const [hovered, setHovered] = useState(false);
  const held = hovered || replying !== null;
  const latestHeld = useRef(onHeld);
  latestHeld.current = onHeld;
  useEffect(() => {
    latestHeld.current?.(held);
  }, [held]);
  const startReply = (action: number) => setReplying(action);
  const send = (text: string) => {
    if (replying === null) return;
    activate(replying, text);
  };
  // Its actions in the person's accent, as a Mac's notification buttons.
  return (
    <BloomColorScope colorPreset={accentPreset} asChild>
    <Box
      testID={testID}
      onPointerEnter={() => setHovered(true)}
      onPointerLeave={() => setHovered(false)}
      className="overflow-hidden rounded-[18px] border border-white/70"
      style={{
        width: BANNER_WIDTH,
        backgroundColor: bannerGlass.backgroundColor,
        boxShadow: "0 12px 40px #0b183a33, 0 0 0 .5px #0b183a1f",
      }}
    >
      <GlassMaterial testID={`${testID}-material`} fill={BANNER_GLASS} shape={BANNER_SHAPE} />
      <PressArea
        testID={`${testID}-open`}
        accessibilityRole="button"
        accessibilityLabel={spoken(notification, name)}
        onPress={() => activate(0)}
        className="flex-row gap-3 p-3 pr-9"
      >
        {app ? <AppIcon app={app} size={36} /> : <Box style={{ width: 36, height: 36 }} />}
        <Box className="min-w-0 flex-1">
          <Box className="flex-row items-center justify-between">
            <Text numberOfLines={1} className="text-[11px] text-[#5b6573]" style={{ flexShrink: 1 }}>
              {name}
            </Text>
            <Text className="ml-2 text-[11px] text-[#5b6573]">{ago(notification.postedAt, now.getTime(), messages)}</Text>
          </Box>
          <Text numberOfLines={1} className="text-[13px] font-semibold text-[#1d2530]">
            {notification.title}
          </Text>
          {notification.subtitle ? (
            <Text numberOfLines={1} className="text-[13px] text-[#1d2530]">
              {notification.subtitle}
            </Text>
          ) : null}
          {notification.body ? (
            <Text numberOfLines={3} className="text-[13px] text-[#3a4452]">
              {notification.body}
            </Text>
          ) : null}
        </Box>
      </PressArea>
      <CloseButton
        testID={`${testID}-close`}
        size="xs"
        accessibilityLabel={words.closeNotification(closeAction, name, notification.title)}
        onPress={close}
        style={{ position: "absolute", top: 8, right: 8 }}
      />
      {replying !== null ? (
        <Box className="flex-row items-center gap-2 px-3 pb-3">
          <Box className="flex-1">
            <TextField size="sm">
              <TextFieldInput
                testID={`${testID}-reply`}
                label={notification.actions[replying - 1]?.label ?? words.reply}
                placeholder={notification.actions[replying - 1]?.reply || words.message}
                value={reply}
                onChangeText={setReply}
                autoFocus
                multiline={false}
                // Enter sends, as a Mac's reply field does.
                submitBehavior="submit"
                returnKeyType="send"
                // What the field holds as Enter is pressed, as the lock screen sends its password.
                onSubmitEditing={({ nativeEvent }) => send(nativeEvent.text)}
              />
            </TextField>
          </Box>
          <Button testID={`${testID}-send`} size="sm" onPress={() => send(reply)}>
            {words.send}
          </Button>
        </Box>
      ) : notification.actions.length > 0 ? (
        <Box className="flex-row flex-wrap gap-2 px-3 pb-3">
          {notification.actions.map((action, index) => (
            <Button
              key={index}
              testID={`${testID}-action-${index + 1}`}
              size="sm"
              appearance="subtle"
              onPress={() => (action.reply !== undefined ? startReply(index + 1) : activate(index + 1))}
            >
              {action.label}
            </Button>
          ))}
        </Box>
      ) : null}
    </Box>
    </BloomColorScope>
  );
});
