/**
 * The login screen's words (docs/adr/0040-language-and-region.mdx, decision 5), in the system's
 * language: before anyone signs in, the session's locale is the system's alone. English is the
 * shape; Spanish is typed to it, so a missing or extra string fails the type-check. What the
 * login screen shares with the lock screen (the clock, the password field, the wallpaper) speaks
 * through the shell's catalogue (apps/shell/src/messages.ts): each string lives in one.
 */
import type { Language } from "../../shell/src/messages";

/** A count's words in a language: `one` where its CLDR plural rules say "one", else `other`. */
function plural(language: Language, count: number, one: string, other: string): string {
  return new Intl.PluralRules(language).select(count) === "one" ? one : other;
}

const en = {
  accounts: "Accounts",
  /** An account Oxy names nothing (neither a name nor a username): its user is never a name. */
  unnamedAccount: "Oxy Account",
  addAccount: "Add Account",
  unreadable: "The accounts on this computer can’t be read.",
  signInAgain: {
    description: (name: string) => `Sign in to ${name}’s Oxy account again to use it on this computer.`,
    action: "Sign In Again",
  },
  migrate: {
    title: "Migrate from a Backup",
    transferTitle: "Transfer Your Data to This Computer",
    transferDescription:
      "Bring another computer’s accounts over from its backup, with their files, their pictures and its Wi-Fi networks. You can also set this computer up as new.",
    fromBackup: "From a Backup",
    fromBackupDetail: "A backup disk or a network share",
    asNew: "Not Now",
    asNewDetail: "Set up this computer as new",
    sourceTitle: "Where Are the Backups?",
    sourceDescription: "Connect the other computer’s backup disk, or choose a network share.",
    lookingForDisks: "Looking for backup disks…",
    noDisks: "No backup disk is connected.",
    disks: "Backup Disks",
    networkShare: "Network Share…",
    lookAgain: "Look Again",
    shareTitle: "Connect to a Network Share",
    shareHost: "Computer",
    shareName: "Shared Folder",
    shareUser: "User",
    sharePassword: "Password",
    connect: "Connect",
    passwordTitle: (place: string) => `Enter the backup password for ${place}`,
    passwordDescription: "The password chosen when the backups were set up. OxyOS doesn’t keep it.",
    passwordPlaceholder: "Backup Password",
    opening: "Opening the backups…",
    computers: "Choose a Computer",
    backups: (computer: string) => `Backups of ${computer}`,
    accounts: (names: string) => `Accounts: ${names}`,
    list: (names: string[]) =>
      names.length < 2 ? names.join("") : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`,
    confirmTitle: (computer: string) => `Migrate ${computer}?`,
    confirmDescription: (date: string, names: string) =>
      `Its backup of ${date} brings over ${names}, with their files, pictures and the Wi-Fi networks. Each person then signs in again with their Oxy account.`,
    migrateAction: "Migrate",
    steps: {
      accounts: "Adding the accounts…",
      files: (percent: string) => `Restoring files… ${percent}`,
      finishing: "Finishing…",
    },
    migrating: "Don’t turn off this computer.",
    failedTitle: "The Migration Didn’t Finish",
    migrated: (count: number) => plural("en", count, "1 account was brought over", `${count} accounts were brought over`),
    startOver: "Start Over",
    problems: {
      notConnected: "The backup disk isn’t connected.",
      noBackup: "There are no OxyOS backups there.",
      wrongPassword: "That password doesn’t open any of the backups there.",
      shareRefused: "The share didn’t accept that user and password.",
      unreachable: "The share’s computer can’t be reached. Check its name and the network.",
      inUse: "Another computer is using these backups now. Try again in a moment.",
      full: "This computer’s disk is full.",
      invalidBackup: "This backup can’t be migrated to a computer.",
      clash: "An account of this backup is on this computer already.",
      alreadySetUp: "This computer has an account already.",
      failed: "The migration didn’t finish. Try again.",
    },
  },
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
    notConfiguredTitle: "Accounts Can’t Be Added",
    notConfigured:
      "This computer isn’t set up to add Oxy accounts. Ask whoever installed OxyOS on it to check its configuration.",
    checkingAccount: "Finishing signing in…",
    accountUnchecked:
      "Oxy couldn’t be reached to finish signing in. You’re still signed in: check the connection, then try again.",
    passwordTitle: (name: string) => (name ? `Enter ${name}’s Oxy Password` : "Enter Your Oxy Password"),
    passwordDescription: "This computer signs you in with your Oxy password, even offline. Enter it once more.",
    passwordPlaceholder: "Oxy Account Password",
    passwordIncorrect: "Incorrect password. Try again.",
    passwordThrottled: (seconds: number) =>
      seconds > 0
        ? plural(
            "en",
            seconds,
            "Too many attempts. Wait 1 second, then try again.",
            `Too many attempts. Wait ${seconds} seconds, then try again.`,
          )
        : "Too many attempts. Wait a moment, then try again.",
    unusablePassword: "This computer can’t use this password. Choose another one for your Oxy account, then try again.",
    pinTitle: "Create a PIN",
    pinAgainTitle: "Enter Your PIN Again",
    pinDescription: "You’ll use it to sign in and unlock this computer, even offline. It stays here: Oxy never sees it.",
    pinOptionalDescription: "Unlock faster with a PIN, even offline. It stays on this computer: Oxy never sees it.",
    pinAgainDescription: "To be sure it’s the one you meant.",
    pinPlaceholder: "PIN (4 to 16 digits)",
    pinMismatch: "Those PINs don’t match. Create it again.",
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
      `Oxy is busy right now. Setup goes on by itself in ${plural("en", seconds, "1 second", `${seconds} seconds`)}.`,
    problemTitle: "Your Account Isn’t Set Up Yet",
    unreachable:
      "Oxy couldn’t be reached. You’re still signed in, and your PIN and choices are kept: check the connection, then try again.",
    machine:
      "This computer couldn’t create your account, and nothing was changed. You’re still signed in: try again. If it happens again, restart the computer.",
    expired: "You signed in more than a few minutes ago, so you need to sign in again to continue.",
    failed: "Your account couldn’t be set up, and nothing was changed. Try again.",
    signInAgain: "Sign In Again",
    secondFactorTitle: "Two-Factor Authentication",
    secondFactorDescription: "Enter the code from your authenticator app.",
    secondFactorInvalid: "That code isn’t right. Wait for a new one, then try again.",
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
  signInAgain: {
    description: (name: string) => `Vuelve a iniciar sesión en la cuenta de Oxy de ${name} para usarla en este ordenador.`,
    action: "Volver a iniciar sesión",
  },
  migrate: {
    title: "Migrar desde una copia de seguridad",
    transferTitle: "Transfiere tus datos a este ordenador",
    transferDescription:
      "Trae las cuentas de otro ordenador desde su copia de seguridad, con sus archivos, sus imágenes y sus redes Wi-Fi. También puedes configurar este ordenador como nuevo.",
    fromBackup: "Desde una copia de seguridad",
    fromBackupDetail: "Un disco de copias o una carpeta compartida de la red",
    asNew: "Ahora no",
    asNewDetail: "Configurar este ordenador como nuevo",
    sourceTitle: "¿Dónde están las copias?",
    sourceDescription: "Conecta el disco de copias del otro ordenador o elige una carpeta compartida de la red.",
    lookingForDisks: "Buscando discos de copias…",
    noDisks: "No hay ningún disco de copias conectado.",
    disks: "Discos de copias",
    networkShare: "Carpeta compartida de la red…",
    lookAgain: "Buscar otra vez",
    shareTitle: "Conectarse a una carpeta compartida",
    shareHost: "Ordenador",
    shareName: "Carpeta compartida",
    shareUser: "Usuario",
    sharePassword: "Contraseña",
    connect: "Conectar",
    passwordTitle: (place: string) => `Introduce la contraseña de las copias de ${place}`,
    passwordDescription: "La contraseña que se eligió al configurar las copias. OxyOS no la guarda.",
    passwordPlaceholder: "Contraseña de las copias",
    opening: "Abriendo las copias…",
    computers: "Elige un ordenador",
    backups: (computer: string) => `Copias de ${computer}`,
    accounts: (names: string) => `Cuentas: ${names}`,
    list: (names: string[]) =>
      names.length < 2 ? names.join("") : `${names.slice(0, -1).join(", ")} y ${names[names.length - 1]}`,
    confirmTitle: (computer: string) => `¿Migrar ${computer}?`,
    confirmDescription: (date: string, names: string) =>
      `Su copia del ${date} trae a ${names}, con sus archivos, sus imágenes y las redes Wi-Fi. Después cada persona vuelve a iniciar sesión con su cuenta de Oxy.`,
    migrateAction: "Migrar",
    steps: {
      accounts: "Añadiendo las cuentas…",
      files: (percent: string) => `Recuperando archivos… ${percent}`,
      finishing: "Terminando…",
    },
    migrating: "No apagues este ordenador.",
    failedTitle: "La migración no ha terminado",
    migrated: (count: number) => plural("es", count, "Se ha traído 1 cuenta", `Se han traído ${count} cuentas`),
    startOver: "Empezar de nuevo",
    problems: {
      notConnected: "El disco de copias no está conectado.",
      noBackup: "Ahí no hay copias de seguridad de OxyOS.",
      wrongPassword: "Esa contraseña no abre ninguna de las copias que hay ahí.",
      shareRefused: "La carpeta compartida no ha aceptado ese usuario y contraseña.",
      unreachable: "No se puede llegar al ordenador de la carpeta compartida. Comprueba su nombre y la red.",
      inUse: "Otro ordenador está usando estas copias ahora. Vuelve a intentarlo en un momento.",
      full: "El disco de este ordenador está lleno.",
      invalidBackup: "Esta copia no se puede migrar a un ordenador.",
      clash: "Una cuenta de esta copia ya está en este ordenador.",
      alreadySetUp: "Este ordenador ya tiene una cuenta.",
      failed: "La migración no ha terminado. Vuelve a intentarlo.",
    },
  },
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
    passwordIncorrect: "Contraseña incorrecta. Vuelve a intentarlo.",
    passwordThrottled: (seconds: number) =>
      seconds > 0
        ? plural(
            "es",
            seconds,
            "Demasiados intentos. Espera 1 segundo y vuelve a intentarlo.",
            `Demasiados intentos. Espera ${seconds} segundos y vuelve a intentarlo.`,
          )
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
    appearanceDescription: "Selecciona una apariencia. Puedes cambiarla más tarde en Ajustes.",
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
      `Oxy está ocupado ahora mismo. La configuración seguirá sola en ${plural("es", seconds, "1 segundo", `${seconds} segundos`)}.`,
    problemTitle: "Tu cuenta aún no está configurada",
    unreachable:
      "No se ha podido contactar con Oxy. Sigues con la sesión iniciada, y tu PIN y tus elecciones se conservan: revisa la conexión y vuelve a intentarlo.",
    machine:
      "Este ordenador no ha podido crear tu cuenta y no se ha cambiado nada. Sigues con la sesión iniciada: vuelve a intentarlo. Si vuelve a pasar, reinicia el ordenador.",
    expired:
      "Iniciaste sesión hace más de unos minutos, así que tienes que volver a iniciarla para continuar.",
    failed: "No se ha podido configurar tu cuenta y no se ha cambiado nada. Vuelve a intentarlo.",
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
