/**
 * The last session ended because of a problem (docs/adr/0065-the-session-ending-because-of-a-problem.mdx,
 * decision 2): the login screen asks its session once it is shown (`{"lastSession": {}}`), and says
 * so quietly under its field when the answer is `{"lastSession": {"problem": true}}`, as the login
 * program saw the session end (a crash, a hang, a failure), never when the person signed out.
 */

/** The login screen's question. */
export const LAST_SESSION_REQUEST: Record<string, unknown> = { lastSession: {} };

/** Whether the session's answer says the last session ended because of a problem. */
export function endedBecauseOfAProblem(answer: unknown): boolean {
  if (!answer || typeof answer !== "object") return false;
  const told = (answer as { lastSession?: unknown }).lastSession;
  return !!told && typeof told === "object" && (told as { problem?: unknown }).problem === true;
}
