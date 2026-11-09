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
  /** An account Oxy names nothing (neither a name nor a username): its user is never a name. */
  unnamedAccount: "Oxy Account",
  addAccount: "Add Account",
  unreadable: "The accounts on this computer can't be read.",
  setup: {
    /** The assistant's accessible name. */
    title: "Set Up an Oxy Account",
    back: "Back",
    continue: "Continue",
    retry: "Try Again",
    notNow: "Not Now",
    welcomeTitle: "Welcome to OxyOS",
    welcomeDescription: "Sign in with your Oxy account to make this computer yours. It takes a minute.",
    getStarted: "Get Started",
    checking: "Checking the connection…",
    networkTitle: "Connect to the Internet",
    offline:
      "Setting up an account needs the internet once, to sign in with Oxy. Connect this computer to a network, then try again. After that, it works offline.",
    notConfiguredTitle: "Accounts Can't Be Added",
    notConfigured:
      "This computer isn't set up to add Oxy accounts. Ask whoever installed OxyOS on it to check its configuration.",
    checkingAccount: "Finishing signing in…",
    accountUnchecked:
      "Oxy couldn't be reached to finish signing in. You're still signed in: check the connection, then try again.",
    passwordTitle: (name: string) => (name ? `Enter ${name}'s Oxy Password` : "Enter Your Oxy Password"),
    passwordDescription: "This computer signs you in with your Oxy password, even offline. Enter it once more.",
    passwordPlaceholder: "Oxy Account Password",
    passwordIncorrect: "That isn't the password of this Oxy account. Try again.",
    passwordThrottled: (seconds: number) =>
      seconds > 0 ? `Too many tries. Wait ${seconds} seconds, then try again.` : "Too many tries. Wait a moment, then try again.",
    unusablePassword: "This computer can't use this password. Choose another one for your Oxy account, then try again.",
    pinTitle: "Create a PIN",
    pinAgainTitle: "Enter Your PIN Again",
    pinDescription: "You'll use it to sign in and unlock this computer, even offline. It stays here: Oxy never sees it.",
    pinOptionalDescription: "Unlock faster with a PIN, even offline. It stays on this computer: Oxy never sees it.",
    pinAgainDescription: "To be sure it's the one you meant.",
    pinPlaceholder: "PIN (4 to 16 digits)",
    pinMismatch: "Those PINs don't match. Create it again.",
    unusablePin: "A PIN is 4 to 16 digits, and not your password.",
    appearanceTitle: "Choose Your Look",
    appearanceDescription: "Select an appearance. You can change it later in Settings.",
    appearance: { light: "Light", dark: "Dark", auto: "Auto" },
    appearanceAutoDescription: "Light by day, dark at night.",
    settingUpTitle: "Setting Up Your Account…",
    settingUpDescription: "This only takes a moment.",
    stages: {
      securing: "Protecting your PIN",
      verifying: "Checking your Oxy account",
      preparing: "Creating your home folder",
      signingIn: "Connecting this computer to your account",
      finishing: "Finishing up",
      confirming: "Confirming this computer with Oxy",
      starting: "Opening your desktop",
    },
    stageDone: "done",
    stageNow: "in progress",
    waiting: (seconds: number) =>
      `Oxy is busy right now. Setup goes on by itself in ${seconds} ${seconds === 1 ? "second" : "seconds"}.`,
    problemTitle: "Your Account Isn't Set Up Yet",
    unreachable:
      "Oxy couldn't be reached. Your sign-in, PIN and choices are kept: check the connection, then try again.",
    machine:
      "This computer couldn't create your account, and nothing was changed. Your sign-in is kept: try again. If it happens again, restart the computer.",
    expired: "Your sign-in can no longer be used, because it was more than a few minutes ago. Sign in again to continue.",
    failed: "Something went wrong while setting up your account, and nothing was changed. Try again.",
    signInAgain: "Sign In Again",
    secondFactorTitle: "Two-Factor Authentication",
    secondFactorDescription: "Enter the code from your authenticator app.",
    secondFactorInvalid: "That code isn't right. Wait for a new one, then try again.",
    code: "Code",
    welcomeName: (name: string) => (name ? `Welcome, ${name}` : "Welcome"),
    added: (name: string) => (name ? `${name} was added to this computer` : "The account was added to this computer"),
    step: (title: string, current: number, total: number) => `${title}, step ${current} of ${total}`,
  },
  pinLocked: "Too many incorrect PINs. Sign in with Oxy to create a new one.",
  signInWithOxy: "Sign In with Oxy",
};

/** The catalogue's shape, whatever its language. */
export type LoginMessages = typeof en;

const es: LoginMessages = {
  accounts: "Cuentas",
  unnamedAccount: "Cuenta de Oxy",
  addAccount: "Añadir cuenta",
  unreadable: "No se pueden leer las cuentas de este ordenador.",
  setup: {
    title: "Configurar una cuenta de Oxy",
    back: "Atrás",
    continue: "Continuar",
    retry: "Reintentar",
    notNow: "Ahora no",
    welcomeTitle: "Te damos la bienvenida a OxyOS",
    welcomeDescription: "Inicia sesión con tu cuenta de Oxy para que este ordenador sea tuyo. Solo lleva un minuto.",
    getStarted: "Empezar",
    checking: "Comprobando la conexión…",
    networkTitle: "Conéctate a internet",
    offline:
      "Para configurar una cuenta hace falta internet una vez, para iniciar sesión con Oxy. Conecta este ordenador a una red y vuelve a intentarlo. Después funciona sin conexión.",
    notConfiguredTitle: "No se pueden añadir cuentas",
    notConfigured:
      "Este ordenador no está preparado para añadir cuentas de Oxy. Pide a quien instaló OxyOS que revise su configuración.",
    checkingAccount: "Terminando de iniciar sesión…",
    accountUnchecked:
      "No se ha podido contactar con Oxy para terminar de iniciar sesión. Sigues con la sesión iniciada: revisa la conexión y vuelve a intentarlo.",
    passwordTitle: (name: string) =>
      name ? `Introduce la contraseña de Oxy de ${name}` : "Introduce tu contraseña de Oxy",
    passwordDescription:
      "Este ordenador inicia tu sesión con tu contraseña de Oxy, incluso sin conexión. Introdúcela una vez más.",
    passwordPlaceholder: "Contraseña de la cuenta de Oxy",
    passwordIncorrect: "Esa no es la contraseña de esta cuenta de Oxy. Vuelve a intentarlo.",
    passwordThrottled: (seconds: number) =>
      seconds > 0
        ? `Demasiados intentos. Espera ${seconds} segundos y vuelve a intentarlo.`
        : "Demasiados intentos. Espera un momento y vuelve a intentarlo.",
    unusablePassword:
      "Este ordenador no puede usar esta contraseña. Elige otra para tu cuenta de Oxy y vuelve a intentarlo.",
    pinTitle: "Crea un PIN",
    pinAgainTitle: "Vuelve a introducir tu PIN",
    pinDescription:
      "Lo usarás para iniciar sesión y desbloquear este ordenador, incluso sin conexión. Se queda aquí: Oxy nunca lo ve.",
    pinOptionalDescription:
      "Desbloquea más rápido con un PIN, incluso sin conexión. Se queda en este ordenador: Oxy nunca lo ve.",
    pinAgainDescription: "Para asegurarnos de que es el que querías.",
    pinPlaceholder: "PIN (de 4 a 16 cifras)",
    pinMismatch: "Los PIN no coinciden. Vuelve a crearlo.",
    unusablePin: "Un PIN tiene de 4 a 16 cifras y no es tu contraseña.",
    appearanceTitle: "Elige tu aspecto",
    appearanceDescription: "Selecciona un aspecto. Puedes cambiarlo más tarde en Ajustes.",
    appearance: { light: "Claro", dark: "Oscuro", auto: "Automático" },
    appearanceAutoDescription: "Claro de día, oscuro de noche.",
    settingUpTitle: "Configurando tu cuenta…",
    settingUpDescription: "Solo será un momento.",
    stages: {
      securing: "Protegiendo tu PIN",
      verifying: "Comprobando tu cuenta de Oxy",
      preparing: "Creando tu carpeta personal",
      signingIn: "Conectando este ordenador con tu cuenta",
      finishing: "Terminando",
      confirming: "Confirmando este ordenador con Oxy",
      starting: "Abriendo tu escritorio",
    },
    stageDone: "hecho",
    stageNow: "en curso",
    waiting: (seconds: number) =>
      `Oxy está ocupado ahora mismo. La configuración seguirá sola en ${seconds} ${seconds === 1 ? "segundo" : "segundos"}.`,
    problemTitle: "Tu cuenta aún no está configurada",
    unreachable:
      "No se ha podido contactar con Oxy. Tu inicio de sesión, tu PIN y tus elecciones se conservan: revisa la conexión y vuelve a intentarlo.",
    machine:
      "Este ordenador no ha podido crear tu cuenta y no se ha cambiado nada. Tu inicio de sesión se conserva: vuelve a intentarlo. Si vuelve a pasar, reinicia el ordenador.",
    expired:
      "Tu inicio de sesión ya no se puede usar, porque fue hace más de unos minutos. Vuelve a iniciar sesión para continuar.",
    failed: "Algo ha fallado al configurar tu cuenta y no se ha cambiado nada. Vuelve a intentarlo.",
    signInAgain: "Volver a iniciar sesión",
    secondFactorTitle: "Autenticación en dos pasos",
    secondFactorDescription: "Introduce el código de tu app de autenticación.",
    secondFactorInvalid: "Ese código no es correcto. Espera a uno nuevo y vuelve a intentarlo.",
    code: "Código",
    welcomeName: (name: string) => (name ? `Te damos la bienvenida, ${name}` : "Te damos la bienvenida"),
    added: (name: string) => (name ? `Se ha añadido ${name} a este ordenador` : "Se ha añadido la cuenta a este ordenador"),
    step: (title: string, current: number, total: number) => `${title}, paso ${current} de ${total}`,
  },
  pinLocked: "Demasiados PIN incorrectos. Inicia sesión con Oxy para crear uno nuevo.",
  signInWithOxy: "Iniciar sesión con Oxy",
};

/** Each language's catalogue. */
export const LOGIN_CATALOGUES: Record<Language, LoginMessages> = { en, es };

/** The login screen's words in a language (the one Bloom resolved from the locale,
    useLoginMessages.ts). */
export function loginMessages(language: Language): LoginMessages {
  return LOGIN_CATALOGUES[language];
}
