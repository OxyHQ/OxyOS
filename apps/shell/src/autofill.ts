/**
 * AutoFill's list (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 6), as the session
 * tells the shell to draw it at a field's caret: the suggestions for the field's application (a
 * login's title and username, never its password), a strong password for a new password, the
 * logins whose verification code a code field may take; the highlighted one, which the session
 * moves with ↑ and ↓ and picks with Return. What the list window shows and says; nothing here asks
 * anything but the press on an item (`choose`, the session's `chooseAutofill`).
 */

import type { ShellMessages } from "./messages";

/** What an item fills when picked. */
export type AutofillKind = "login" | "strongPassword" | "code";

/** One item: a login's title and username, a strong password (no title) and the password itself,
    a code's login. */
export type AutofillItem = {
  kind: AutofillKind;
  title: string;
  detail: string;
};

/** The list the session tells (`{"autofill": …}`); null hides it. */
export type AutofillList = {
  /** What the field holds: `username`, `email`, `password`, `newPassword`, `oneTimeCode`. */
  content: string;
  items: readonly AutofillItem[];
  /** The highlighted item's index. */
  highlighted: number;
};

const KINDS: readonly AutofillKind[] = ["login", "strongPassword", "code"];

/** The list as the session told it, read strictly: null for none, or for anything that is not one. */
export function readAutofill(told: unknown): AutofillList | null {
  if (!told || typeof told !== "object") return null;
  const { content, items, highlighted } = told as Record<string, unknown>;
  if (typeof content !== "string" || !Array.isArray(items) || typeof highlighted !== "number") return null;
  const read: AutofillItem[] = [];
  for (const item of items) {
    if (!item || typeof item !== "object") return null;
    const { kind, title, detail } = item as Record<string, unknown>;
    if (!KINDS.includes(kind as AutofillKind) || typeof title !== "string" || typeof detail !== "string") return null;
    read.push({ kind: kind as AutofillKind, title, detail });
  }
  if (read.length === 0 || highlighted < 0 || highlighted >= read.length) return null;
  return { content, items: read, highlighted };
}

/** What a row shows: its first line and its second. A login, its username over its title, as a
    Mac's list shows the account first; a strong password, "Use Strong Password" over the password;
    a code, the login's title over "Verification code for" its username. */
export function autofillRow(item: AutofillItem, messages: ShellMessages): { primary: string; secondary: string } {
  const words = messages.autofill;
  switch (item.kind) {
    case "strongPassword":
      return { primary: words.strongPassword, secondary: item.detail };
    case "code":
      return { primary: item.title || item.detail, secondary: words.code(item.detail) };
    default:
      return { primary: item.detail || item.title, secondary: item.title };
  }
}

/** An item as the screen reader says it, with its place and the list's count:
    "bea@example.com, example.com, 1 of 2". */
export function autofillLabel(list: AutofillList, index: number, messages: ShellMessages): string {
  const item = list.items[index];
  if (!item) return messages.autofill.list;
  const { primary, secondary } = autofillRow(item, messages);
  return messages.autofill.item(primary, secondary, index + 1, list.items.length);
}

/** What the list says of itself: its highlighted item with its place, said again as the highlight
    moves (a polite live region). */
export function autofillSaid(list: AutofillList, messages: ShellMessages): string {
  return autofillLabel(list, list.highlighted, messages);
}
