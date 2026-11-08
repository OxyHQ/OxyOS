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
  signInAgain: {
    description: (name: string) => `Sign in to ${name}'s Oxy account again to use it on this computer.`,
    action: "Sign In Again",
  },
  migrate: {
    offer: "Migrate from a Backup",
    title: "Migrate from a Backup",
    sourceDescription:
      "Bring the accounts of another computer over from its backup: their files, their pictures and its Wi-Fi networks. Connect its backup disk, or choose a network share.",
    lookingForDisks: "Looking for backup disks…",
    noDisks: "No backup disk is connected.",
    disks: "Backup disks",
    networkShare: "Network Share…",
    lookAgain: "Look Again",
    shareTitle: "Connect to a Network Share",
    shareHost: "Computer",
    shareName: "Shared Folder",
    shareUser: "User",
    sharePassword: "Password",
    connect: "Connect",
    passwordTitle: (place: string) => `Enter the backup password for ${place}`,
    passwordDescription: "The password chosen when the backups were set up. OxyOS doesn't keep it.",
    passwordPlaceholder: "Backup Password",
    opening: "Opening the backups…",
    computers: "Computers",
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
    migrating: "Don't turn off this computer.",
    migrated: (count: number) => (count === 1 ? "1 account was brought over" : `${count} accounts were brought over`),
    startOver: "Start Over",
    problems: {
      notConnected: "The backup disk isn't connected.",
      noBackup: "There are no OxyOS backups there.",
      wrongPassword: "That password doesn't open any of the backups there.",
      shareRefused: "The share didn't accept that user and password.",
      unreachable: "The share's computer can't be reached. Check its name and the network.",
      inUse: "Another computer is using these backups now. Try again in a moment.",
      full: "This computer's disk is full.",
      invalidBackup: "This backup can't be migrated to a computer.",
      clash: "An account of this backup is on this computer already.",
      alreadySetUp: "This computer has an account already.",
      failed: "The migration didn't finish. Try again.",
    },
  },
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
  signInAgain: {
    description: (name: string) => `Vuelve a iniciar sesión en la cuenta de Oxy de ${name} para usarla en este ordenador.`,
    action: "Volver a iniciar sesión",
  },
  migrate: {
    offer: "Migrar desde una copia de seguridad",
    title: "Migrar desde una copia de seguridad",
    sourceDescription:
      "Trae las cuentas de otro ordenador desde su copia de seguridad: sus archivos, sus imágenes y sus redes Wi-Fi. Conecta su disco de copias o elige una carpeta compartida de la red.",
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
    computers: "Ordenadores",
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
    migrated: (count: number) => (count === 1 ? "Se ha traído 1 cuenta" : `Se han traído ${count} cuentas`),
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
