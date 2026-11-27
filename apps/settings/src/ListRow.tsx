/**
 * A row of one of Settings' list boxes (Input Sources, Spotlight's and Backups' excluded folders,
 * Storage's sheets, Login Items), as a Mac's table in a settings pane draws one: one geometry for
 * every list, its text in the type scale's roles (the body for its name, the small caption for what
 * it is), the selection filled with the accent and its text in the accent's foreground. Decorative:
 * the list's option (Bloom's `ChoiceList`) is the control, named by its label.
 */
import * as React from "react";
import { View, type StyleProp, type ViewStyle } from "react-native";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";

/** The colours of a row's name and of its detail, selected or not. */
export function useListRowInk(selected: boolean): { ink: string; detail: string } {
  const theme = useTheme();
  const ink = selected ? theme.colors.primaryForeground : theme.colors.text;
  return { ink, detail: selected ? ink : theme.colors.textSecondary };
}

export function ListRow({
  selected,
  children,
  style,
}: {
  selected: boolean;
  children: React.ReactNode;
  style?: StyleProp<ViewStyle>;
}) {
  const theme = useTheme();
  return (
    <View style={[ROW, { backgroundColor: selected ? theme.colors.primary : "transparent" }, style]}>{children}</View>
  );
}

/** A row's name: the body style, one line. */
export function ListRowTitle({ color, children, style }: { color: string; children: React.ReactNode; style?: object }) {
  return (
    <Text variant="body-regular" numberOfLines={1} style={[{ color }, style]}>
      {children}
    </Text>
  );
}

/** What a row is, small, one line. */
export function ListRowDetail({ color, children }: { color: string; children: React.ReactNode }) {
  return (
    <Text variant="caption-2-regular" numberOfLines={1} style={{ color }}>
      {children}
    </Text>
  );
}

const ROW: ViewStyle = {
  flexDirection: "row",
  alignItems: "center",
  gap: 10,
  paddingHorizontal: 10,
  paddingVertical: 6,
  borderRadius: 8,
};
