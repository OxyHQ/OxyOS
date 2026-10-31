import React, { memo } from "react";
import { Button } from "@oxy.so/bloom/button";
import { Text } from "@oxy.so/bloom/typography";
import { Box } from "../primitives";
import { sheetView, type Language, type SheetText } from "../sheet";

export const SHEET_WIDTH = 380;
/** Every sheet is this tall, its words scrolling within it, and its buttons are this wide at its
    foot: Cancel and the answer are always in the same place, whatever the sheet says. */
export const SHEET_HEIGHT = 220;
export const SHEET_BUTTON_WIDTH = 120;
const INK = "#1d2430";
const QUIET = "#5b6472";

/**
 * A system sheet (docs/adr/0030-installing-applications.mdx, decision 9): the session's question
 * to the person, over everything but the lock screen, in a shell surface no application draws,
 * covers or presses. Its answer is the shell's alone; Cancel is its first, safest button. A sheet
 * that only tells (the install service is not running) has its one button, in the answer's place.
 */
export const SystemSheet = memo(function SystemSheet({
  text,
  language = "es",
  names,
  answer,
}: {
  text: SheetText;
  language?: Language;
  /** The registry's names, by id, for an application asking for a capability. */
  names?: Record<string, string>;
  answer: (accepted: boolean) => void;
}) {
  const view = sheetView(text, language, names);
  return (
    <Box
      testID="system-sheet-backdrop"
      style={{
        position: "absolute",
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        backgroundColor: "#00000033",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <Box
        testID="system-sheet"
        accessibilityViewIsModal
        accessibilityRole="alert"
        accessibilityLabel={view.title}
        style={{
          width: SHEET_WIDTH,
          height: SHEET_HEIGHT,
          backgroundColor: "#ffffffee",
          borderRadius: 18,
          padding: 20,
          gap: 12,
        }}
      >
        <Box style={{ flex: 1, gap: 12, overflow: "hidden" }}>
        <Text style={{ fontSize: 17, fontWeight: "600", color: INK }}>{view.title}</Text>
        {view.body ? <Text style={{ fontSize: 13, color: QUIET }}>{view.body}</Text> : null}
        {view.lines.map((line, index) => (
          <Box key={index} testID={`system-sheet-line-${index}`} style={{ gap: 2 }}>
            <Text style={{ fontSize: 14, color: INK }}>{line.label}</Text>
            {line.detail ? <Text style={{ fontSize: 12, color: QUIET }}>{line.detail}</Text> : null}
          </Box>
        ))}
        </Box>
        <Box style={{ flexDirection: "row", justifyContent: "flex-end", gap: 8 }}>
          {view.refuse ? (
            <Box style={{ width: SHEET_BUTTON_WIDTH }}>
              <Button testID="system-sheet-refuse" appearance="outline" tone="neutral" onPress={() => answer(false)}>
                {view.refuse}
              </Button>
            </Box>
          ) : null}
          <Box style={{ width: SHEET_BUTTON_WIDTH }}>
            <Button
              testID="system-sheet-accept"
              appearance="solid"
              tone={view.destructive ? "danger" : "accent"}
              onPress={() => answer(true)}
            >
              {view.accept}
            </Button>
          </Box>
        </Box>
      </Box>
    </Box>
  );
});
