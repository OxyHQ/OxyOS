/**
 * The login screen's Sleep, Restart and Shut Down (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx,
 * decision 8), as a Mac's login window has them at its foot: what each is called, and what the
 * session's answer says when it did not happen. Their words are the shell's catalogue's (the Oxy
 * menu and the session's question say the same), so each lives in one place.
 */
import type { ShellMessages } from "../../shell/src/messages";

/** What the login screen's buttons, and its power key's choice, ask of the session. */
export type LoginPowerAction = "sleep" | "restart" | "shutDown";

/** The buttons at the foot, in a Mac's order. */
export const LOGIN_POWER_ACTIONS: readonly LoginPowerAction[] = ["sleep", "restart", "shutDown"];

/** A button's name: Sleep, Restart, Shut Down (Reposo, Reiniciar, Apagar). */
export function loginPowerLabel(action: LoginPowerAction, messages: ShellMessages): string {
  const words = messages.power;
  return action === "sleep" ? words.sleep : action === "restart" ? words.restartNow : words.shutDownButton;
}

/** The session's request for an action, as the login screen sends it: nothing to confirm, as no
    one is signed in (the session refuses `logOut` here). */
export function loginPowerRequest(action: LoginPowerAction): Record<string, unknown> {
  return action === "sleep" ? { sleep: {} } : { endSession: { action, confirm: false } };
}

/**
 * What the session's answer says, when the action did not happen: what could not be done and why,
 * as the session's sheet would say it in a session. Null when it is happening (`{ending}`,
 * `{sleeping}`, `{confirming}`).
 */
export function loginPowerRefusal(action: LoginPowerAction, answer: unknown, messages: ShellMessages): string | null {
  if (!answer || typeof answer !== "object") return null;
  const told = answer as { refused?: { reason?: unknown }; failed?: { reason?: unknown } };
  const why = told.refused ?? told.failed;
  if (!why) return null;
  const words = messages.power;
  const reason =
    why.reason === "notPermitted" ? words.notPermitted[action] : why.reason === "busy" ? words.busy : words.unavailable;
  return `${words.failed[action]} ${reason}`;
}

/** Whether a session's unasked word (`SystemText` serial 0) is the power key pressed at the login
    screen (`{"powerKey": {}}`): the login screen then shows its choice. */
export function isPowerKey(told: unknown): boolean {
  return !!told && typeof told === "object" && "powerKey" in (told as Record<string, unknown>);
}
