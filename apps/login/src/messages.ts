/**
 * The login screen's words (docs/adr/0040-language-and-region.mdx, decision 5), in the system's
 * language: before anyone signs in, the session's locale is the system's alone. English is the
 * shape; Spanish is typed to it, so a missing or extra string fails the type-check. What the
 * login screen shares with the lock screen (the clock, the password field, the wallpaper) speaks
 * through the shell's catalogue (apps/shell/src/messages.ts): each string lives in one.
 */
import type { Language } from "../../shell/src/messages";

const en = {
  accounts: "Accounts",
  addAccount: "Add Account",
  unreadable: "The accounts on this computer can't be read.",
  setup: {
    title: "Add an Oxy Account",
    checking: "Checking the connection…",
    offline: "Adding an account needs an internet connection. Connect and try again.",
    unavailable: "Signing in with Oxy isn't available on this screen yet.",
    retry: "Try Again",
    back: "Back",
  },
};

/** The catalogue's shape, whatever its language. */
export type LoginMessages = typeof en;

const es: LoginMessages = {
  accounts: "Cuentas",
  addAccount: "Añadir cuenta",
  unreadable: "No se pueden leer las cuentas de este ordenador.",
  setup: {
    title: "Añadir una cuenta de Oxy",
    checking: "Comprobando la conexión…",
    offline: "Para añadir una cuenta hace falta conexión a internet. Conéctate y vuelve a intentarlo.",
    unavailable: "El inicio de sesión con Oxy aún no está disponible en esta pantalla.",
    retry: "Reintentar",
    back: "Volver",
  },
};

/** Each language's catalogue. */
export const LOGIN_CATALOGUES: Record<Language, LoginMessages> = { en, es };

/** The login screen's words in a language (the one Bloom resolved from the locale,
    useLoginMessages.ts). */
export function loginMessages(language: Language): LoginMessages {
  return LOGIN_CATALOGUES[language];
}
