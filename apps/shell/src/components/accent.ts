import { APP_COLOR_PRESETS, type AppColorName } from "@oxy.so/bloom/theme";
import { usePlatformAppearanceState } from "@oxy.so/bloom/platform-surface";

/** The person's accent, as the platform tells it, where Bloom has it; else OxyOS's own. */
export function useAccent(): AppColorName {
  const accent = usePlatformAppearanceState()?.accent;
  return accent && accent in APP_COLOR_PRESETS ? (accent as AppColorName) : "oxy";
}
