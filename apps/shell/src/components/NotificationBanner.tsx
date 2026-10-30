import React, { useState } from "react";
import { Button, CloseButton } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { TextField, TextFieldInput } from "@oxy.so/bloom/text-field";
import { Box, PressArea } from "../primitives";
import { AppIcon } from "../apps";
import type { ShellApplication, ShellNotification } from "../model";
import { ago, applicationOf, spoken } from "../notifications";

export const BANNER_WIDTH = 360;

/**
 * One notification as a card (docs/adr/0026-notifications.mdx), as a Mac's: the application's own
 * icon and name, when, its title, subtitle and body, and its actions. Pressing the card opens it
 * in its application (`activate(0)`); an action is `activate(n)`, and a reply action first opens a
 * field whose text goes with it. The card is one button for a screen reader, named by whose it is
 * and what it says; its actions and close are buttons of their own.
 */
export function NotificationCard({
  notification,
  applications,
  now,
  activate,
  close,
  closeLabel,
  onReplying,
  testID,
}: {
  notification: ShellNotification;
  applications: readonly ShellApplication[];
  now: Date;
  activate: (action: number, reply?: string) => void;
  close: () => void;
  closeLabel: string;
  /** Whether the person is typing a reply in it (a banner then stays). */
  onReplying?: (replying: boolean) => void;
  testID: string;
}) {
  const { name, app } = applicationOf(notification, applications);
  const [replying, setReplying] = useState<number | null>(null);
  const [reply, setReply] = useState("");
  const startReply = (action: number) => {
    setReplying(action);
    onReplying?.(true);
  };
  const send = (text: string) => {
    if (replying === null) return;
    activate(replying, text);
  };
  return (
    <Box
      testID={testID}
      className="overflow-hidden rounded-[18px] border border-white/70"
      style={{
        width: BANNER_WIDTH,
        backgroundColor: "#f5f7fbf2",
        boxShadow: "0 12px 40px #0b183a33, 0 0 0 .5px #0b183a1f",
      }}
    >
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
            <Text className="ml-2 text-[11px] text-[#5b6573]">{ago(notification.postedAt, now.getTime())}</Text>
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
        size="xs"
        accessibilityLabel={closeLabel}
        onPress={close}
        style={{ position: "absolute", top: 8, right: 8 }}
      />
      {replying !== null ? (
        <Box className="flex-row items-center gap-2 px-3 pb-3">
          <Box className="flex-1">
            <TextField size="sm">
              <TextFieldInput
                testID={`${testID}-reply`}
                label={notification.actions[replying - 1]?.label ?? "Responder"}
                placeholder={notification.actions[replying - 1]?.reply || "Mensaje"}
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
            Enviar
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
  );
}
