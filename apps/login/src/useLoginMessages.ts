import { useShellMessages } from "../../shell/src/useShellMessages";
import { loginMessages, type LoginMessages } from "./messages";

/** The login screen's catalogue (messages.ts) in the locale its root gave Bloom: the same language
    as the shell's words it shares and Bloom's own. */
export function useLoginMessages(): LoginMessages {
  return loginMessages(useShellMessages().language);
}
