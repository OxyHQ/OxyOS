import { resolveBloomLanguage, useBloomLocale } from "@oxy.so/bloom/locale";
import { shellMessages, type ShellMessages } from "./messages";

/**
 * The session's locale as the shell renders in it (docs/adr/0040-language-and-region.mdx,
 * decision 5): the one its root gave `BloomProvider` (`locale`), published once Bloom's own
 * strings for it are loaded, so the shell and Bloom's components switch in the same render; else
 * the runtime's, `Intl`'s default.
 */
export function useShellLocale(): string {
  return useBloomLocale() ?? Intl.DateTimeFormat().resolvedOptions().locale;
}

/** The shell's catalogue (messages.ts) in the session's locale: its language as Bloom resolves it,
    formatting in the locale. A change of the locale renders whatever reads it again. */
export function useShellMessages(): ShellMessages {
  const locale = useShellLocale();
  return shellMessages(locale, resolveBloomLanguage(locale) === "es" ? "es" : "en");
}
