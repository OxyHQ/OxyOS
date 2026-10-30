/** The Settings application's words, in the session's language: Spanish or English. */
export type Strings = typeof en;

const en = {
  title: "Settings",
  groups: { personal: "Personal", system: "System" },
  pages: {
    appearance: "Appearance",
    wallpaper: "Wallpaper",
    lockScreen: "Lock Screen",
    keyboard: "Keyboard",
    dateTime: "Date & Time",
    accessibility: "Accessibility",
    about: "About",
  },
  appearance: {
    mode: "Appearance",
    light: "Light",
    dark: "Dark",
    auto: "Auto",
    autoDescription: "Light during the day, dark at night, where the clock's time zone is.",
    accent: "Accent colour",
  },
  wallpaper: { current: "Wallpaper", system: "OxyOS", dynamic: "Follows the sun through the day." },
  lockScreen: {
    lockAfter: "Lock after",
    never: "Never",
    minutes: (n: number) => (n === 1 ? "1 minute" : n === 60 ? "1 hour" : `${n} minutes`),
  },
  keyboard: { layout: "Layout" },
  dateTime: {
    timeZone: "Time zone",
    automatic: "Set time automatically",
    clock24h: "24-hour time",
    on: "On",
    off: "Off",
  },
  accessibility: {
    screenReader: "Screen reader",
    screenReaderDescription: "Speaks what is on the screen. Super+F5 turns it on and off.",
    reduceMotion: "Reduce motion",
    reduceMotionDescription: "Windows, menus and the lock screen appear and go at once.",
    reduceTransparency: "Reduce transparency",
    reduceTransparencyDescription: "Glass is drawn opaque.",
  },
  about: {
    name: "Name",
    version: "OxyOS",
    processor: "Processor",
    memory: "Memory",
    display: "Display",
  },
  systemNote: "Changing this needs an administrator, which OxyOS does not ask for yet.",
};

const es: Strings = {
  title: "Ajustes",
  groups: { personal: "Personal", system: "Sistema" },
  pages: {
    appearance: "Apariencia",
    wallpaper: "Fondo de pantalla",
    lockScreen: "Pantalla de bloqueo",
    keyboard: "Teclado",
    dateTime: "Fecha y hora",
    accessibility: "Accesibilidad",
    about: "Información",
  },
  appearance: {
    mode: "Apariencia",
    light: "Clara",
    dark: "Oscura",
    auto: "Automática",
    autoDescription: "Clara de día y oscura de noche, según la zona horaria del reloj.",
    accent: "Color de acento",
  },
  wallpaper: { current: "Fondo", system: "OxyOS", dynamic: "Sigue al sol a lo largo del día." },
  lockScreen: {
    lockAfter: "Bloquear tras",
    never: "Nunca",
    minutes: (n: number) => (n === 1 ? "1 minuto" : n === 60 ? "1 hora" : `${n} minutos`),
  },
  keyboard: { layout: "Distribución" },
  dateTime: {
    timeZone: "Zona horaria",
    automatic: "Ajustar la hora automáticamente",
    clock24h: "Formato de 24 horas",
    on: "Sí",
    off: "No",
  },
  accessibility: {
    screenReader: "Lector de pantalla",
    screenReaderDescription: "Lee en voz alta lo que hay en la pantalla. Super+F5 lo activa y desactiva.",
    reduceMotion: "Reducir movimiento",
    reduceMotionDescription: "Las ventanas, los menús y la pantalla de bloqueo aparecen y se van al instante.",
    reduceTransparency: "Reducir transparencia",
    reduceTransparencyDescription: "El cristal se dibuja opaco.",
  },
  about: {
    name: "Nombre",
    version: "OxyOS",
    processor: "Procesador",
    memory: "Memoria",
    display: "Pantalla",
  },
  systemNote: "Para cambiarlo hace falta un administrador, que OxyOS aún no pide.",
};

/** The words for a BCP 47 locale ("es-ES", "en"): Spanish for Spanish, English otherwise. */
export function stringsFor(locale: string): Strings {
  return locale.toLowerCase().split(/[-_]/)[0] === "es" ? es : en;
}
