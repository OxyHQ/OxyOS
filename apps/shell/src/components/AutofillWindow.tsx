import React, { memo } from "react";
import { Pressable } from "react-native";
import { Text } from "@oxy.so/bloom/typography";
import { BloomColorScope, useTheme } from "@oxy.so/bloom/theme";
import { resolveSurfaceLevel } from "@oxy.so/bloom/styles";
import { RiKey2Line } from "@oxy.so/bloom/icons/RiKey2Line";
import { useAccent } from "./accent";
import { Box } from "../primitives";
import { autofillLabel, autofillRow, autofillSaid, type AutofillList } from "../autofill";
import { useShellMessages } from "../useShellMessages";

/** A floating panel's shadow, light and dark, kept within `ROOM` (as the candidate window's). */
const SHADOW = {
  light: "0 1px 1px 0 rgba(0, 0, 0, 0.04), 0 2px 6px 0 rgba(0, 0, 0, 0.14)",
  dark: "0 1px 1px 0 rgba(0, 0, 0, 0.14), 0 2px 6px 0 rgba(0, 0, 0, 0.36)",
} as const;
/** The room around the panel in its surface, which the platform places at the caret's
    bottom-left: a gap under the line typed, and the shadow's extent each side and below. */
const ROOM = { paddingTop: 3, paddingLeft: 6, paddingRight: 6, paddingBottom: 8 } as const;
const RADIUS = 10;
const PADDING = 4;
/** As wide as its longest row, within these. */
const MIN_WIDTH = 240;
const MAX_WIDTH = 380;
const ICON = 18;
const PRIMARY = { fontSize: 13, lineHeight: 17, fontWeight: "500" } as const;
const SECONDARY = { fontSize: 11, lineHeight: 14 } as const;

/**
 * AutoFill's list (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 6), as a Mac shows
 * its passwords under a login field: a key and, for each login, its username over its site's
 * title; for a new password, Use Strong Password over the password it will be; for a code, its
 * login over its verification code's account. The highlighted row on the person's accent; Bloom's menus'
 * surface, light and dark. The session moves the highlight (↑ ↓) and picks it (Return); a press
 * on a row picks it. The window never takes focus: the field keeps it.
 *
 * For the screen reader it is a list named by its highlighted item with its place and the count
 * ("bea@example.com, example.com, 1 of 2"), a polite live region the reader says at each move;
 * its rows are the list's items, the highlighted one selected.
 */
export const AutofillWindow = memo(function AutofillWindow(props: {
  list: AutofillList;
  onChoose: (index: number) => void;
}) {
  return (
    <BloomColorScope colorPreset={useAccent()}>
      <AutofillPanel {...props} />
    </BloomColorScope>
  );
});

const AutofillPanel = memo(function AutofillPanel({
  list,
  onChoose,
}: {
  list: AutofillList;
  onChoose: (index: number) => void;
}) {
  const theme = useTheme();
  const messages = useShellMessages();
  const surface = resolveSurfaceLevel(theme, 1);
  return (
    <Box style={ROOM} pointerEvents="box-none">
      <Box
        testID="shell-autofill"
        collapsable={false}
        accessibilityRole="list"
        accessibilityLabel={autofillSaid(list, messages)}
        accessibilityLiveRegion="polite"
        style={{
          minWidth: MIN_WIDTH,
          maxWidth: MAX_WIDTH,
          backgroundColor: surface.background,
          borderColor: surface.border,
          borderWidth: 1,
          borderRadius: RADIUS,
          padding: PADDING,
          boxShadow: theme.isDark ? SHADOW.dark : SHADOW.light,
          alignSelf: "flex-start",
        }}
      >
        {list.items.map((item, index) => {
          const highlighted = index === list.highlighted;
          const { primary, secondary } = autofillRow(item, messages);
          const ink = highlighted ? theme.colors.primaryForeground : surface.text;
          const quiet = highlighted ? theme.colors.primaryForeground : surface.textSecondary;
          return (
            <Pressable
              key={index}
              testID={`shell-autofill-${index}`}
              accessibilityRole="listitem"
              accessibilityLabel={autofillLabel(list, index, messages)}
              accessibilityState={{ selected: highlighted }}
              onPress={() => onChoose(index)}
              style={{
                flexDirection: "row",
                alignItems: "center",
                gap: 8,
                paddingHorizontal: 8,
                paddingVertical: 4,
                borderRadius: RADIUS - PADDING,
                backgroundColor: highlighted ? theme.colors.primary : "transparent",
              }}
            >
              <RiKey2Line width={ICON} height={ICON} fill={quiet} />
              <Box style={{ flexShrink: 1 }}>
                <Text numberOfLines={1} style={{ ...PRIMARY, color: ink }}>
                  {primary}
                </Text>
                <Text numberOfLines={1} style={{ ...SECONDARY, color: quiet }}>
                  {secondary}
                </Text>
              </Box>
            </Pressable>
          );
        })}
      </Box>
    </Box>
  );
});
