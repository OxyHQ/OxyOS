// `shell` (docs/adr/0058-the-platform-api.mdx, decision 1): what only the session's shell does, with
// its `shell` grant (the session refuses anyone else). Over the one connection to the session: the
// system's sheets the shell draws and answers (ADR 0030's consent and permission sheets, ADR 0038's
// Wi-Fi password, ADR 0055's pairing code), AutoFill's list at a field's caret (ADR 0050, decision
// 6) and the emoji picker (ADR 0041). The person's answers go to the session alone; the
// application a sheet is about never sees them.
import { errorOf, PlatformError } from './errors.ts';
import { followed, type Followed } from './followed.ts';
import type { CallOptions } from './frameworks.ts';
import type { Events, Json, Session } from './session.ts';

/** A system sheet waiting for the person: its number, and what it asks (the session's JSON). */
export interface SystemSheet {
  readonly sheet: number;
  readonly text: string;
}

export function createShell(session: Session, answerSheet: ((sheet: number, accepted: boolean) => void) | null, events: Events) {
  const { available, state } = session;
  // The sheets waiting, in the order the session showed them, until answered or dismissed.
  const sheets = followed<readonly SystemSheet[]>();
  const waiting = () => sheets.current() ?? [];
  if (available) {
    sheets.tell([]);
    events.addListener('OxyOSSheet', ({ sheet, text }: SystemSheet) => {
      if (!waiting().some((shown) => shown.sheet === sheet)) sheets.tell([...waiting(), { sheet, text }]);
    });
    events.addListener('OxyOSSheetDismissed', ({ sheet }: { sheet: number }) => gone(sheet));
  }
  const gone = (sheet: number) => {
    if (waiting().some((shown) => shown.sheet === sheet)) sheets.tell(waiting().filter((shown) => shown.sheet !== sheet));
  };
  /** Sends one of the person's answers to the session, citing the press or key that gave it. */
  const tell = (body: Json, sheet?: number) => {
    if (!available) throw new PlatformError('shell', 'unavailable');
    if (!session.tell(body)) throw new PlatformError('shell', 'unavailable', 'notSent');
    if (sheet !== undefined) gone(sheet);
  };

  /** A request the session answers: resolved once it did, rejected with its refusal. */
  const ask = async (body: Json, options?: CallOptions): Promise<void> => {
    const error = errorOf('shell', await session.request(body, options));
    if (error) throw error;
  };

  return {
    available,
    /** The system's sheets waiting for the person, in order. */
    sheets: sheets as Followed<readonly SystemSheet[]>,
    /** The person's answer to a sheet: accepted or not. */
    answerSheet(sheet: number, accepted: boolean): void {
      if (!available || !answerSheet) throw new PlatformError('shell', 'unavailable');
      answerSheet(sheet, accepted);
      gone(sheet);
    },
    /** A Wi-Fi password sheet's answer: the password the person typed, to the session alone. */
    answerPassword: (sheet: number, password: string) => tell({ password: { sheet, password } }, sheet),
    /** A Bluetooth pairing sheet's answer: the PIN or passkey the person typed, to the session
        alone. */
    answerCode: (sheet: number, code: string) => tell({ bluetoothCode: { sheet, code } }, sheet),
    /** AutoFill's list the session tells this shell to draw at a field's caret, null for none. The
        application whose field it is never sees it. */
    autofill: state.autofill as Followed<Json | null>,
    /** Picks the AutoFill item `index`, citing the press that chose it. */
    chooseAutofill: (index: number, options?: CallOptions) => ask({ chooseAutofill: { index } }, options),
    /** Shows the emoji picker, citing the press or key that asked. */
    showEmojiPicker: (options?: CallOptions) => ask({ showEmoji: {} }, options),
    /** A passkey sheet's answer (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7): the
        passkey chosen and the Oxy account's password the person typed, to the session alone, which
        hands them to the accounts' service, never to the application. */
    answerPasskey: (sheet: number, choice: number | undefined, password: string) =>
      tell({ passkey: choice === undefined ? { sheet, password } : { sheet, choice, password } }, sheet),
    /** An application that quit (docs/adr/0061-crashes-and-resetting-an-application.mdx): its
        notice's Reset… answers the notice no and asks the session to reset it, which asks the
        person in a sheet of its own and opens it again once it is reset. */
    resetApplication(sheet: number, id: string): void {
      if (!available || !answerSheet) throw new PlatformError('shell', 'unavailable');
      answerSheet(sheet, false);
      gone(sheet);
      void session.request({ resetApplication: { id, reopen: true } });
    },
    /** The power key's sheet (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx, decision 7):
        its Restart and Sleep answer it, the session taking them only for the sheet it shows. */
    choosePowerKey: (sheet: number, choice: string) => tell({ powerKeyChoice: { sheet, choice } }, sheet),
    /** Reopen windows when logging back in, as the person toggled it in the session's question:
        their `reopen_windows`, which the session keeps. */
    setReopenWindows: (enabled: boolean) => tell({ reopenWindows: { enabled } }),
    /**
     * Sleep, restart, shut down and log out (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx):
     * the shell's Oxy menu and the login screen's buttons. The session decides: it asks its own
     * question (`confirm`), every application, then logind. Resolves once it answered
     * (`{confirming}`, `{ending: {action}}`), rejects with its refusal.
     */
    endSession: (action: string, confirm: boolean, options?: CallOptions) =>
      ask({ endSession: { action, confirm } }, options),
    /** Sleeps the computer at once (`{sleeping}`). */
    sleep: (options?: CallOptions) => ask({ sleep: {} }, options),
  };
}

export type Shell = ReturnType<typeof createShell>;
