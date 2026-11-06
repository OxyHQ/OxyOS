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
    notConfigured: "This computer isn't set up to add Oxy accounts.",
    passwordTitle: (name: string) => `Enter ${name}'s password`,
    passwordDescription: "OxyOS signs in with your Oxy account's password, even offline. Enter it once more.",
    passwordPlaceholder: "Oxy Account Password",
    unusablePassword: "OxyOS can't use this password. Choose another one for your Oxy account and try again.",
    failed: "The account couldn't be added. Try again.",
    secondFactorTitle: "Enter the code from your authenticator app",
    secondFactorInvalid: "That code isn't right. Wait for a new one and try again.",
    code: "Code",
    continue: "Continue",
    adding: "Adding the account…",
    added: (name: string) => `${name} was added to this computer`,
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
    notConfigured: "Este ordenador no está preparado para añadir cuentas de Oxy.",
    passwordTitle: (name: string) => `Introduce la contraseña de ${name}`,
    passwordDescription: "OxyOS inicia sesión con la contraseña de tu cuenta de Oxy, incluso sin conexión. Introdúcela una vez más.",
    passwordPlaceholder: "Contraseña de la cuenta de Oxy",
    unusablePassword: "OxyOS no puede usar esta contraseña. Elige otra para tu cuenta de Oxy y vuelve a intentarlo.",
    failed: "No se ha podido añadir la cuenta. Vuelve a intentarlo.",
    secondFactorTitle: "Introduce el código de tu app de autenticación",
    secondFactorInvalid: "Ese código no es correcto. Espera a uno nuevo y vuelve a intentarlo.",
    code: "Código",
    continue: "Continuar",
    adding: "Añadiendo la cuenta…",
    added: (name: string) => `Se ha añadido ${name} a este ordenador`,
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
