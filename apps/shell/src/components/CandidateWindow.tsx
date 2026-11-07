import React, { memo } from "react";
import { Pressable } from "react-native";
import { Text } from "@oxy.so/bloom/typography";
import { BloomColorScope, useTheme } from "@oxy.so/bloom/theme";
import { useAccent } from "./accent";
import { resolveSurfaceLevel } from "@oxy.so/bloom/styles";
import { RiArrowLeftSLine } from "@oxy.so/bloom/icons/RiArrowLeftSLine";
import { RiArrowRightSLine } from "@oxy.so/bloom/icons/RiArrowRightSLine";
import { RiArrowUpSLine } from "@oxy.so/bloom/icons/RiArrowUpSLine";
import { RiArrowDownSLine } from "@oxy.so/bloom/icons/RiArrowDownSLine";
import { Box } from "../primitives";
import { candidateNumber, candidatesSaid, type CandidateList } from "../input-sources";
import { useShellMessages } from "../useShellMessages";

/** A floating panel's shadow, light and dark, kept within `ROOM`. */
const SHADOW = {
  light: "0 1px 1px 0 rgba(0, 0, 0, 0.04), 0 2px 6px 0 rgba(0, 0, 0, 0.14)",
  dark: "0 1px 1px 0 rgba(0, 0, 0, 0.14), 0 2px 6px 0 rgba(0, 0, 0, 0.36)",
} as const;
/** The room around the panel in its surface, which the platform places at the caret's
    bottom-left: a gap under the line typed, and the shadow's extent each side and below. */
const ROOM = { paddingTop: 3, paddingLeft: 6, paddingRight: 6, paddingBottom: 8 } as const;
const RADIUS = 10;
const PADDING = 4;
/** A candidate's text, as large as a field's, and its number's. */
const TEXT = { fontSize: 16, lineHeight: 22 } as const;
const NUMBER = { fontSize: 11, lineHeight: 22, fontVariant: ["tabular-nums"] } as const;
const AUX = { fontSize: 12, lineHeight: 16 } as const;
const CHEVRON = 16;

/**
 * The candidate window (docs/adr/0041-input-methods.mdx, decision 5), as a Mac draws its own: the
 * page of candidates an input method hands the system, numbered 1 to 9 then 0 (the digit keys that
 * choose them in the engine), in a strip (or down, when the engine lays it out so), the
 * highlighted one on the accent, the reading above, a chevron each side when there is a page
 * before or after. In Bloom's menus' surface, light and dark. Its keys are the engine's: the window
 * never takes focus, and a press chooses a candidate or turns the page.
 *
 * For the screen reader it is a list named by its highlighted candidate with its place and the
 * count ("你好, 1 of 9"), a polite live region: the reader says each candidate as the highlight or
 * the page changes. Its rows are the list's items, each said with its place and the count, the
 * highlighted one selected; the chevrons are buttons.
 */
/** The window in the person's accent, as a Mac highlights a candidate in its accent colour: the
    shell's own colour is its chrome's, not the highlight's. */
export const CandidateWindow = memo(function CandidateWindow(props: {
  list: CandidateList;
  onChoose: (index: number) => void;
  onTurnPage: (forward: boolean) => void;
}) {
  return (
    <BloomColorScope colorPreset={useAccent()}>
      <CandidatePanel {...props} />
    </BloomColorScope>
  );
});

const CandidatePanel = memo(function CandidatePanel({
  list,
  onChoose,
  onTurnPage,
}: {
  list: CandidateList;
  onChoose: (index: number) => void;
  onTurnPage: (forward: boolean) => void;
}) {
  const theme = useTheme();
  const messages = useShellMessages();
  const surface = resolveSurfaceLevel(theme, 1);
  const vertical = list.vertical;
  const back = vertical ? RiArrowUpSLine : RiArrowLeftSLine;
  const ahead = vertical ? RiArrowDownSLine : RiArrowRightSLine;
  const chevron = (forward: boolean) => {
    const shown = forward ? list.next : list.previous;
    if (!shown) return null;
    const Icon = forward ? ahead : back;
    return (
      <Pressable
        testID={forward ? "shell-candidates-next" : "shell-candidates-previous"}
        accessibilityRole="button"
        accessibilityLabel={forward ? messages.candidates.nextPage : messages.candidates.previousPage}
        onPress={() => onTurnPage(forward)}
        hitSlop={4}
        style={{
          alignItems: "center",
          justifyContent: "center",
          width: vertical ? undefined : 20,
          height: vertical ? 18 : undefined,
          alignSelf: "stretch",
          borderRadius: RADIUS - PADDING,
        }}
      >
        <Icon width={CHEVRON} height={CHEVRON} fill={surface.textSecondary} />
      </Pressable>
    );
  };
  return (
    <Box style={ROOM} pointerEvents="box-none">
      <Box
        testID="shell-candidates"
        collapsable={false}
        accessibilityRole="list"
        accessibilityLabel={candidatesSaid(list, messages)}
        accessibilityLiveRegion="polite"
        style={{
          backgroundColor: surface.background,
          borderColor: surface.border,
          borderWidth: 1,
          borderRadius: RADIUS,
          padding: PADDING,
          boxShadow: theme.isDark ? SHADOW.dark : SHADOW.light,
          alignSelf: "flex-start",
        }}
      >
        {list.aux ? (
          <Text
            testID="shell-candidates-aux"
            style={{ ...AUX, color: surface.textSecondary, paddingHorizontal: 6, paddingTop: 1, paddingBottom: 2 }}
          >
            {list.aux}
          </Text>
        ) : null}
        <Box style={{ flexDirection: vertical ? "column" : "row", alignItems: vertical ? "stretch" : "center" }}>
          {chevron(false)}
          {list.candidates.map((text, index) => {
            const highlighted = index === list.highlighted;
            return (
              <Pressable
                key={index}
                testID={`shell-candidate-${index}`}
                // A row of the list: the reader says its place and the count after it ("你好, 1 de 9").
                accessibilityRole="listitem"
                accessibilityLabel={text}
                accessibilityState={{ selected: highlighted }}
                onPress={() => onChoose(index)}
                style={{
                  flexDirection: "row",
                  alignItems: "baseline",
                  gap: 5,
                  paddingHorizontal: 7,
                  paddingVertical: 1,
                  borderRadius: RADIUS - PADDING,
                  backgroundColor: highlighted ? theme.colors.primary : "transparent",
                }}
              >
                <Text
                  style={{ ...NUMBER, color: highlighted ? theme.colors.primaryForeground : surface.textSecondary }}
                >
                  {candidateNumber(index)}
                </Text>
                <Text style={{ ...TEXT, color: highlighted ? theme.colors.primaryForeground : surface.text }}>
                  {text}
                </Text>
              </Pressable>
            );
          })}
          {chevron(true)}
        </Box>
      </Box>
    </Box>
  );
});
