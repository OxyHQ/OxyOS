/**
 * An Oxy account's picture, or its initials in a circle when it has none: decorative, its name is
 * beside it. Settings draws it in the sidebar's account item, on the account's page and in the
 * Erase Assistant's list of accounts.
 */
import * as React from "react";
import { Image, View } from "react-native";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";

export function AccountPicture({
  picture,
  initials,
  size,
}: {
  picture: string | null;
  initials: string;
  size: number;
}) {
  const theme = useTheme();
  return (
    <View
      importantForAccessibility="no-hide-descendants"
      accessibilityElementsHidden
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        overflow: "hidden",
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: theme.colors.primary,
      }}
    >
      {picture ? (
        <Image source={{ uri: picture }} style={{ width: size, height: size }} />
      ) : (
        <Text style={{ color: theme.colors.primaryForeground, fontSize: size * 0.4, fontWeight: "600" }}>
          {initials}
        </Text>
      )}
    </View>
  );
}
