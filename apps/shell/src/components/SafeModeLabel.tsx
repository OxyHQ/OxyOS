/**
 * Safe Mode's word (docs/adr/0054-recovery-and-erasing.mdx, decision 10), in red, as a Mac shows
 * "Safe Boot" in its menu bar and login window for the whole of a start in Safe Mode: in the top
 * bar beside the status items, on the lock screen and on the login screen. Text a screen reader
 * reads where it is; nothing to press.
 */
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import { useShellMessages } from "../useShellMessages";

export function SafeModeLabel({ size = 13 }: { size?: number }) {
  const theme = useTheme();
  const messages = useShellMessages();
  return (
    <Text
      testID="safe-mode"
      accessibilityRole="text"
      style={{ color: theme.colors.error, fontSize: size, fontWeight: "600" }}
    >
      {messages.safeMode}
    </Text>
  );
}
