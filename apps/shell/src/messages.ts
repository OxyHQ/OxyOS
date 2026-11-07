/**
 * The shell's words (docs/adr/0040-language-and-region.mdx, decision 5): one catalogue, in the
 * session's language. English is the shape; Spanish is typed to it, so a missing or extra string
 * fails the type-check. A count is a function of `Intl.PluralRules`, and a number goes through
 * `Intl.NumberFormat`, both in the session's locale (its region's digits and separators).
 *
 * The catalogue is made for a locale (`shellMessages`); components take it from
 * `useShellMessages()` (useShellMessages.ts), pure helpers as a parameter. It carries its
 * `language` and `locale` with it, so a helper that formats or picks a translated text
 * (`localized`) needs nothing else.
 */

/** The languages OxyOS speaks. */
export type Language = "en" | "es";

/** The language a BCP 47 tag asks for, among OxyOS's: its primary subtag (`es-MX` → `es`, as
    Bloom's `resolveBloomLanguage` matches it); anything else is English. */
export function languageOf(locale: string | null | undefined): Language {
  return locale?.toLowerCase().split(/[-_]/)[0] === "es" ? "es" : "en";
}

/** The plural forms a count picks from, as CLDR names them; `other` is every one a language
    does not name. */
export type PluralForms = { one: string; other: string; many?: string; two?: string; few?: string; zero?: string };

/** How a catalogue formats in its locale. */
export type Formats = {
  /** A number in the locale's digits and separators, with `fractionDigits` decimals. */
  number: (value: number, fractionDigits?: number) => string;
  /** The form of `count` the locale's plural rules choose. */
  plural: (count: number, forms: PluralForms) => string;
};

/** A tag `Intl` takes: the given one, else its language alone (a malformed tag is no reason for
    the shell not to draw). */
function usable(locale: string, language: Language): string {
  try {
    return Intl.getCanonicalLocales(locale)[0] ?? language;
  } catch {
    return language;
  }
}

function formats(locale: string): Formats {
  const numbers = new Map<number, Intl.NumberFormat>();
  const rules = new Intl.PluralRules(locale);
  return {
    number: (value, fractionDigits = 0) => {
      let format = numbers.get(fractionDigits);
      if (!format) {
        format = new Intl.NumberFormat(locale, {
          minimumFractionDigits: fractionDigits,
          maximumFractionDigits: fractionDigits,
        });
        numbers.set(fractionDigits, format);
      }
      return format.format(value);
    },
    plural: (count, forms) => forms[rules.select(count) as keyof PluralForms] ?? forms.other,
  };
}

const en = (f: Formats) => ({
  /** Words several parts say. */
  common: {
    close: "Close",
    options: "Options",
    systemSettings: "System Settings",
    showDesktops: "Show Desktops",
    searchApplications: "Search Applications",
    controlCenter: "Control Center",
    notificationCenter: "Notification Center",
    notifications: "Notifications",
    doNotDisturb: "Do Not Disturb",
    trash: "Trash",
    /** A switch's state, said of Bluetooth, Dark Mode and Do Not Disturb. */
    on: "On",
    off: "Off",
    /** A level in whole percent. */
    percent: (value: number) => `${f.number(value)}%`,
    seconds: (count: number) => f.plural(count, { one: "1 second", other: `${f.number(count)} seconds` }),
    minutes: (count: number) => f.plural(count, { one: "1 minute", other: `${f.number(count)} minutes` }),
  },
  /** The top bar, its Oxy menu and its status items. */
  topBar: {
    menuBar: "Menu Bar",
    oxyMenu: "Oxy Menu",
    applications: "Applications",
    lockScreen: "Lock Screen",
    /** The battery, a level indicator: its charge and state are its value. */
    battery: "Battery",
    /** The wired network's value, as the Wi-Fi item's is the network joined. */
    connected: "Connected",
    /** The clock's hint: what pressing it does, which its name (the date and time) does not say. */
    clockHint: "Opens Notification Center",
  },
  /** The input sources (docs/adr/0041-input-methods.mdx): the top bar's item, its menu, and what
      is said of a switch. */
  inputSources: {
    /** The item's name; its value is the current source's name ("Input source, Pinyin – Simplified"). */
    item: "Input source",
    pinyin: "Pinyin – Simplified",
    japanese: "Japanese – Romaji",
    korean: "2-Set Korean",
    /** A keyboard layout by its XKB name, as a Mac names its layouts; one not named here is
        "Keyboard (XX)". */
    layouts: {
      us: "U.S.",
      gb: "British",
      es: "Spanish",
      latam: "Latin American",
      fr: "French",
      de: "German",
      it: "Italian",
      pt: "Portuguese",
      br: "Brazilian",
    } as Record<string, string>,
    layout: (code: string) => `Keyboard (${code})`,
    showEmoji: "Show Emoji & Symbols",
    openKeyboardSettings: "Open Keyboard Settings…",
  },
  /** The candidate window (docs/adr/0041-input-methods.mdx). */
  candidates: {
    /** The list's name while nothing is highlighted. */
    list: "Candidates",
    /** A candidate, its place on the page and how many there are: "你好, 1 of 9". */
    candidate: (text: string, place: number, count: number) => `${text}, ${f.number(place)} of ${f.number(count)}`,
    previousPage: "Previous Page",
    nextPage: "Next Page",
  },
  /** The shell's own menus, while no application is active, and an application's own menu. */
  menus: {
    about: "About OxyOS",
    view: "View",
    showApplications: "Show Applications",
    aboutApp: (name: string) => `About ${name}`,
    hideApp: (name: string) => `Hide ${name}`,
    quitApp: (name: string) => `Quit ${name}`,
    /** The Shift key in a menu's chord. */
    shift: "Shift",
  },
  /** The machine's levels, battery, Wi-Fi and Bluetooth (controls.ts). */
  controls: {
    names: {
      output: "Volume",
      input: "Microphone",
      display: "Brightness",
      keyboard: "Keyboard Brightness",
    },
    level: (name: string, percent: string) => `${name}, ${percent}`,
    microphoneMuted: "Microphone muted",
    soundMuted: "Sound muted",
    muted: "Muted",
    duration: (hours: number, minutes: number) =>
      hours === 0
        ? `${f.number(minutes)} min`
        : minutes === 0
          ? `${f.number(hours)} h`
          : `${f.number(hours)} h ${f.number(minutes)} min`,
    charging: (charge: string) => `${charge}, charging`,
    chargingFor: (charge: string, duration: string) => `${charge}, charging (${duration} until full)`,
    charged: (charge: string) => `${charge}, charged`,
    notCharging: (charge: string) => `${charge}, plugged in, not charging`,
    remaining: (charge: string, duration: string) => `${charge}, ${duration} remaining`,
    wifiOff: "Off",
    wifiDisconnected: "Not Connected",
    wifiConnecting: "Connecting",
    wifiConnectingTo: (ssid: string) => `Connecting to ${ssid}`,
    wifiConnected: "Connected",
    devices: (count: number) => f.plural(count, { one: "1 device", other: `${f.number(count)} devices` }),
  },
  /** The battery's menu (docs/adr/0057-battery-and-energy.mdx). */
  battery: {
    title: "Battery",
    sourceBattery: "Power Source: Battery",
    sourceAdapter: "Power Source: Power Adapter",
    remaining: (duration: string) => `${duration} remaining`,
    untilFull: (duration: string) => `${duration} until full`,
    calculating: "Calculating…",
    fullyCharged: "Fully Charged",
    notCharging: "Not Charging",
    significant: "Using Significant Energy",
    noSignificant: "No Apps Using Significant Energy",
    settings: "Battery Settings…",
    lowPower: "Low Power Mode",
  },
  /** The Control Center's modules. */
  controlCenter: {
    wifiNetworks: "Wi-Fi Networks",
    bluetoothDevices: "Bluetooth Devices",
    outputDevices: "Output Devices",
    darkMode: "Dark Mode",
    display: "Display",
    keyboard: "Keyboard",
    sound: "Sound",
    mute: "Mute",
    unmute: "Unmute",
    battery: "Battery",
    systemSettings: "System Settings…",
  },
  /** The Control Center's Wi-Fi view (network.ts, WifiPanel). */
  wifi: {
    back: "Back to Control Center",
    knownNetworks: "Known Networks",
    otherNetworks: "Other Networks",
    settings: "Wi-Fi Settings…",
    connecting: "connecting",
    connected: "connected",
    secure: "secure",
    unsupported: "not supported",
    signal: (arcs: number) => `signal ${f.number(arcs)} of 3`,
    serviceDown: "The network service isn't responding.",
    hardwareBlocked: "Wi-Fi is turned off with the computer's switch.",
    off: "Wi-Fi is off",
    searching: "Looking for networks…",
    joined: (ssid: string) => `Connected to ${ssid}`,
  },
  /** The Control Center's Sound view (sound.ts, SoundPanel). */
  sound: {
    back: "Back to Control Center",
    output: "Output",
    settings: "Sound Settings…",
    builtInSpeakers: "Built-in Speakers",
    externalHeadphones: "External Headphones",
    lineOut: "Line Out",
  },
  /** The Control Center's Bluetooth view (bluetooth.ts, BluetoothPanel, docs/adr/0055-bluetooth.mdx). */
  bluetooth: {
    back: "Back",
    devices: "Devices",
    settings: "Bluetooth Settings…",
    connected: "Connected",
    notConnected: "Not Connected",
    /** A device's state while the session connects or disconnects it, said with its name. */
    connecting: "Connecting",
    disconnecting: "Disconnecting",
    /** The same, shown. */
    connectingShown: "Connecting…",
    disconnectingShown: "Disconnecting…",
    battery: (percent: string) => `Battery ${percent}`,
    none: "No Devices",
    off: "Bluetooth is off.",
    blocked: "Bluetooth is turned off by a hardware switch or airplane mode.",
    unavailable: "Bluetooth isn't available.",
  },
  /** The dock and its menus (dock.ts). */
  dock: {
    menuHint: "To open its menu, press Up Arrow",
    openApplications: "Open Applications",
    running: "running",
    trashFull: "full",
    trashEmpty: "empty",
    open: "Open",
    showAllWindows: "Show All Windows",
    hide: "Hide",
    keepInDock: "Keep in Dock",
    openAtLogin: "Open at Login",
    quit: "Quit",
    forceQuit: "Force Quit",
    notResponding: "Application Not Responding",
    untitled: "Untitled",
    emptyTrash: "Empty Trash",
    moveLeft: "Move Left",
    moveRight: "Move Right",
    removeFromDock: "Remove from Dock",
    /** Said once an icon moved from the keyboard or an assistive technology: its new place. */
    moved: (name: string, place: number, count: number) =>
      `${name}, position ${f.number(place)} of ${f.number(count)}`,
  },
  launchpad: {
    close: "Close Applications",
    search: "Search",
    folder: (name: string) => `${name} folder`,
    noMatches: (query: string) => `No apps match “${query}”.`,
    page: (page: number) => `Page ${f.number(page)}`,
    /** What a search found, said politely as it changes ("3 results"). */
    results: (count: number) =>
      count === 0 ? "No Results" : f.plural(count, { one: "1 result", other: `${f.number(count)} results` }),
  },
  overview: {
    desktop: (number: number) => `Desktop ${f.number(number)}`,
    noPreview: "Preview Not Available",
    closeWindow: (title: string) => `Close ${title}`,
    openDesktop: (name: string) => `Open ${name}`,
    openEmptyDesktop: (name: string) => `Open ${name}, empty`,
    desktopOptions: (name: string) => `${name} Options`,
    openThisDesktop: "Open This Desktop",
    goTo: (name: string) => `Go to ${name}`,
    noWindows: "No Open Windows",
    addDesktop: "Add Desktop",
    newDesktop: "New Desktop",
  },
  /** App Exposé (docs/adr/0045-app-expose.mdx): a dialog named by its application. */
  appExpose: {
    title: "App Exposé",
    label: (name: string) => `${name} Windows`,
    open: "Open Windows",
    minimized: "Minimized Windows",
  },
  switcher: { label: "Application Switcher" },
  /** Banners, the Notification Center and the lock screen's summary (notifications.ts). */
  notifications: {
    count: (count: number) =>
      f.plural(count, { one: "1 notification", other: `${f.number(count)} notifications` }),
    now: "now",
    minutesAgo: (minutes: number) => `${f.number(minutes)} min ago`,
    hoursAgo: (hours: number) => `${f.number(hours)} h ago`,
    yesterday: "yesterday",
    daysAgo: (days: number) => f.plural(days, { one: "1 day ago", other: `${f.number(days)} days ago` }),
    /** The verb a banner's close button says, and the one the Notification Center's says
        (`clear`), named with the notification it closes. */
    closeBanner: "Close",
    closeNotification: (action: string, name: string, title: string) => `${action} the ${name} notification: ${title}`,
    reply: "Reply",
    message: "Message",
    send: "Send",
    none: "No Notifications",
    clear: "Clear",
    clearApp: (name: string) => `Clear ${name} notifications`,
  },
  /** The lock and login screens' password field (lock-wait.ts, PasswordField). */
  password: {
    placeholder: "Enter Password",
    label: "Password",
    incorrect: "Incorrect password",
    tryAgainIn: (wait: string) => `Try again in ${wait}`,
    incorrectTryAgainIn: (wait: string) => `Incorrect password. Try again in ${wait}`,
    tryAgainInSeconds: (seconds: number) => `Try again in ${f.number(seconds)} s`,
    canTryAgain: "You can try again now",
    capsLock: "Caps Lock is on",
    notChecked: "The password couldn't be checked",
    checking: "Checking…",
  },
  screenshots: {
    captureScreen: "Capture Entire Screen",
    captureWindow: "Capture Selected Window",
    captureSelection: "Capture Selected Portion",
    screen: "Screen",
    window: "Window",
    selection: "Selection",
    desktop: "Desktop",
    documents: "Documents",
    none: "None",
    chooseWindow: "Choose a window",
    selectPortion: "Select a portion of the screen",
    screenshot: "Screenshot",
    capture: "Capture",
    saveTo: "Save to",
    timer: "Timer",
    openHint: "Opens it",
    saved: "Screenshot saved",
  },
  wallpaper: "Wallpaper",
  /** The desktop's files (desktop.ts, docs/adr/0047-finder-drags-and-the-desktop.mdx). */
  desktop: {
    kinds: {
      folder: "Folder",
      image: "Image",
      audio: "Audio",
      video: "Video",
      pdf: "PDF document",
      text: "Text document",
      archive: "Archive",
      document: "Document",
    },
    open: "Open",
    moveToTrash: "Move to Trash",
    selectAll: "Select All",
    file: "File",
    edit: "Edit",
    /** Why a drop did nothing (ADR 0035, decision 13). */
    intoItself: "An item can't be moved into itself",
    place: (name: string) => `“${name}” stays where it is`,
    taken: (name: string) => `An item named “${name}” is already there`,
    /** What a copy made on the desktop is named with, as Finder names it (`notas copy.txt`). */
    copy: "copy",
  },
  /** The system's sheets (sheet.ts). */
  sheet: {
    install: (name: string) => `Install ${name}?`,
    update: (name: string) => `Update ${name}?`,
    uninstall: (name: string) => `Delete ${name} and its data?`,
    consent: (name: string) => `${name} asks for more access`,
    permission: (name: string, what: string) => `Allow ${name} to ${what}?`,
    installButton: "Install",
    updateButton: "Update",
    uninstallButton: "Delete",
    allow: "Allow",
    dontAllow: "Don't Allow",
    cancel: "Cancel",
    version: (version: string, size: string) => `Version ${version} · ${size}`,
    uninstallBody: "The app and its data in this account will be deleted. You can install it again from Atlas.",
    consentBody: (version: string) => `Version ${version} uses:`,
    keyRotation: "The developer signs this version with a new key of its lineage.",
    later: "Asked when the app uses it",
    unavailable: {
      install: "Can't Install the App",
      uninstall: "Can't Delete the App",
      update: "Can't Update",
    },
    unavailableBody: "The service that installs apps on this computer isn't running. Try again once it's available.",
    ok: "OK",
    emptyTrash: "Are you sure you want to permanently erase the items in the Trash?",
    emptyTrashBody: (items: number) =>
      `${f.plural(items, { one: "1 item will be erased", other: `${f.number(items)} items will be erased` })}. You can't undo this action.`,
    emptyTrashButton: "Empty",
    wifiPassword: (ssid: string, kind: string) => `The Wi-Fi network “${ssid}” requires a ${kind} password.`,
    wifiIncorrect: "The password is incorrect. Try again.",
    wifiJoin: "Join",
    wifiField: "Password",
    showPassword: "Show password",
    hidePassword: "Hide password",
    wifiFailed: (ssid: string) => `Couldn't join the network “${ssid}”.`,
    wifiUnsupported: (ssid: string) => `OxyOS can't join “${ssid}” yet.`,
    wifiEnterprise: "Enterprise (802.1X) networks aren't supported yet.",
    wifiWep: "WEP networks aren't supported: they aren't secure.",
    wifiOtherUnsupported: "This network isn't supported yet.",
    /** Bluetooth's pairing (docs/adr/0055-bluetooth.mdx): a code to compare, a device asking to
        pair or to use a service, a code to type on it or here, and a connection that failed. */
    bluetoothPair: "Pair",
    bluetoothConfirm: (name: string) => `Confirm that “${name}” shows this code.`,
    bluetoothAuthorize: (name: string) => `“${name}” wants to pair with this computer.`,
    bluetoothService: (name: string, what: string) => `Allow “${name}” to ${what}?`,
    /** What a device asks to do, completing "Allow “X” to …?". */
    bluetoothServices: {
      audio: "play and record audio",
      input: "be used as an input device",
      phone: "connect for phone calls",
      files: "send and receive files",
      network: "share a network connection",
      other: "connect to this computer",
    } as Record<string, string>,
    bluetoothDisplay: (name: string) => `Type this code on “${name}”, then press Return.`,
    bluetoothPasskey: (name: string) => `Enter the code shown on “${name}”.`,
    bluetoothPin: (name: string) => `Enter the PIN for “${name}”.`,
    bluetoothPinHint: "If you don't know it, try 0000 or 1234.",
    bluetoothCodeField: "Code",
    bluetoothPinField: "PIN",
    bluetoothFailed: (name: string) => `Couldn't connect to “${name}”.`,
    bluetoothRejected: "The code didn't match, or the device refused the connection.",
    bluetoothTimeout: "Make sure it's turned on, nearby and in pairing mode.",
    bluetoothOtherFailure: "Make sure it's turned on and nearby.",
    /** A code said digit by digit: "Code: 1 2 3 4 5 6". */
    bluetoothCodeSaid: (digits: string) => `Code: ${digits}`,
    /** A size's units, from bytes up. */
    units: ["bytes", "KB", "MB", "GB"] as readonly string[],
    /** Each capability's name, and what an application asking for it would do. */
    capabilities: {
      network: ["The Internet", "connect to the Internet"],
      notifications: ["Notifications", "send you notifications"],
      files: ["Your files", "access your files"],
      camera: ["Camera", "use the camera"],
      microphone: ["Microphone", "use the microphone"],
      location: ["Location", "use your location"],
      "oxy-account": ["Your Oxy account", "use your Oxy account"],
      "open-files": ["Opening files", "open files"],
      "url-schemes": ["Opening links", "open links"],
      background: ["In the background", "run in the background"],
      "login-item": ["At sign-in", "open at sign-in"],
    } as Record<string, readonly [string, string]>,
  },
});

/** The catalogue's shape, whatever its language. */
type Catalogue = ReturnType<typeof en>;

const es = (f: Formats): Catalogue => ({
  common: {
    close: "Cerrar",
    options: "Opciones",
    systemSettings: "Ajustes del sistema",
    showDesktops: "Ver escritorios",
    searchApplications: "Buscar aplicaciones",
    controlCenter: "Centro de control",
    notificationCenter: "Centro de notificaciones",
    notifications: "Notificaciones",
    doNotDisturb: "No molestar",
    trash: "Papelera",
    on: "Activado",
    off: "Desactivado",
    percent: (value) => `${f.number(value)} %`,
    seconds: (count) => f.plural(count, { one: "1 segundo", other: `${f.number(count)} segundos` }),
    minutes: (count) => f.plural(count, { one: "1 minuto", other: `${f.number(count)} minutos` }),
  },
  topBar: {
    menuBar: "Barra de menús",
    oxyMenu: "Menú de Oxy",
    applications: "Aplicaciones",
    lockScreen: "Bloquear pantalla",
    battery: "Batería",
    connected: "Conectado",
    clockHint: "Abre el centro de notificaciones",
  },
  inputSources: {
    item: "Fuente de entrada",
    pinyin: "Pinyin – Simplificado",
    japanese: "Japonés – Romaji",
    korean: "Coreano de 2 juegos",
    layouts: {
      us: "EE. UU.",
      gb: "Británico",
      es: "Español",
      latam: "Latinoamericano",
      fr: "Francés",
      de: "Alemán",
      it: "Italiano",
      pt: "Portugués",
      br: "Brasileño",
    },
    layout: (code) => `Teclado (${code})`,
    showEmoji: "Mostrar emojis y símbolos",
    openKeyboardSettings: "Abrir ajustes de Teclado…",
  },
  candidates: {
    list: "Candidatos",
    candidate: (text, place, count) => `${text}, ${f.number(place)} de ${f.number(count)}`,
    previousPage: "Página anterior",
    nextPage: "Página siguiente",
  },
  menus: {
    about: "Acerca de OxyOS",
    view: "Ver",
    showApplications: "Ver aplicaciones",
    aboutApp: (name) => `Acerca de ${name}`,
    hideApp: (name) => `Ocultar ${name}`,
    quitApp: (name) => `Salir de ${name}`,
    shift: "Mayús",
  },
  controls: {
    names: {
      output: "Volumen",
      input: "Micrófono",
      display: "Brillo",
      keyboard: "Retroiluminación del teclado",
    },
    level: (name, percent) => `${name}, ${percent}`,
    microphoneMuted: "Micrófono silenciado",
    soundMuted: "Sonido silenciado",
    muted: "Silenciado",
    duration: (hours, minutes) =>
      hours === 0
        ? `${f.number(minutes)} min`
        : minutes === 0
          ? `${f.number(hours)} h`
          : `${f.number(hours)} h ${f.number(minutes)} min`,
    charging: (charge) => `${charge}, cargando`,
    chargingFor: (charge, duration) => `${charge}, cargando (${duration} para cargarse)`,
    charged: (charge) => `${charge}, cargada`,
    notCharging: (charge) => `${charge}, conectada, sin cargar`,
    remaining: (charge, duration) => `${charge}, ${duration} restantes`,
    wifiOff: "Desactivada",
    wifiDisconnected: "Sin conexión",
    wifiConnecting: "Conectando",
    wifiConnectingTo: (ssid) => `Conectando a ${ssid}`,
    wifiConnected: "Conectada",
    devices: (count) => f.plural(count, { one: "1 dispositivo", other: `${f.number(count)} dispositivos` }),
  },
  battery: {
    title: "Batería",
    sourceBattery: "Fuente de alimentación: batería",
    sourceAdapter: "Fuente de alimentación: adaptador de corriente",
    remaining: (duration) => `Quedan ${duration}`,
    untilFull: (duration) => `${duration} para cargarse`,
    calculating: "Calculando…",
    fullyCharged: "Cargada por completo",
    notCharging: "Sin cargar",
    significant: "Apps que consumen mucha energía",
    noSignificant: "No hay apps que consuman mucha energía",
    settings: "Ajustes de Batería…",
    lowPower: "Modo de bajo consumo",
  },
  controlCenter: {
    wifiNetworks: "Redes Wi-Fi",
    bluetoothDevices: "Dispositivos Bluetooth",
    outputDevices: "Dispositivos de salida",
    darkMode: "Modo oscuro",
    display: "Pantalla",
    keyboard: "Teclado",
    sound: "Sonido",
    mute: "Silenciar",
    unmute: "Activar sonido",
    battery: "Batería",
    systemSettings: "Ajustes del sistema…",
  },
  wifi: {
    back: "Volver al centro de control",
    knownNetworks: "Redes conocidas",
    otherNetworks: "Otras redes",
    settings: "Ajustes de Wi-Fi…",
    connecting: "conectando",
    connected: "conectada",
    secure: "protegida",
    unsupported: "no compatible",
    signal: (arcs) => `señal ${f.number(arcs)} de 3`,
    serviceDown: "El servicio de red no responde.",
    hardwareBlocked: "Wi-Fi está desactivada con el interruptor del ordenador.",
    off: "Wi-Fi desactivada",
    searching: "Buscando redes…",
    joined: (ssid) => `Conectado a ${ssid}`,
  },
  sound: {
    back: "Volver al centro de control",
    output: "Salida",
    settings: "Ajustes de sonido…",
    builtInSpeakers: "Altavoces integrados",
    externalHeadphones: "Auriculares externos",
    lineOut: "Salida de línea",
  },
  bluetooth: {
    back: "Atrás",
    devices: "Dispositivos",
    settings: "Ajustes de Bluetooth…",
    connected: "Conectado",
    notConnected: "No conectado",
    connecting: "Conectando",
    disconnecting: "Desconectando",
    connectingShown: "Conectando…",
    disconnectingShown: "Desconectando…",
    battery: (percent) => `Batería ${percent}`,
    none: "No hay dispositivos",
    off: "Bluetooth está desactivado.",
    blocked: "Bluetooth está desactivado por un interruptor del equipo o por el modo avión.",
    unavailable: "Bluetooth no está disponible.",
  },
  dock: {
    menuHint: "Para abrir su menú, pulsa Flecha arriba",
    openApplications: "Abrir aplicaciones",
    running: "en ejecución",
    trashFull: "llena",
    trashEmpty: "vacía",
    open: "Abrir",
    showAllWindows: "Mostrar todas las ventanas",
    hide: "Ocultar",
    keepInDock: "Mantener en el Dock",
    openAtLogin: "Abrir al iniciar sesión",
    quit: "Salir",
    forceQuit: "Forzar salida",
    notResponding: "La aplicación no responde",
    untitled: "Sin título",
    emptyTrash: "Vaciar Papelera",
    moveLeft: "Mover a la izquierda",
    moveRight: "Mover a la derecha",
    removeFromDock: "Quitar del Dock",
    moved: (name, place, count) => `${name}, posición ${f.number(place)} de ${f.number(count)}`,
  },
  launchpad: {
    close: "Cerrar aplicaciones",
    search: "Buscar",
    folder: (name) => `Carpeta ${name}`,
    noMatches: (query) => `No hay aplicaciones que coincidan con «${query}».`,
    page: (page) => `Página ${f.number(page)}`,
    results: (count) =>
      count === 0 ? "Ningún resultado" : f.plural(count, { one: "1 resultado", other: `${f.number(count)} resultados` }),
  },
  overview: {
    desktop: (number) => `Escritorio ${f.number(number)}`,
    noPreview: "Vista previa no disponible",
    closeWindow: (title) => `Cerrar ${title}`,
    openDesktop: (name) => `Abrir ${name}`,
    openEmptyDesktop: (name) => `Abrir ${name} vacío`,
    desktopOptions: (name) => `Opciones de ${name}`,
    openThisDesktop: "Abrir este escritorio",
    goTo: (name) => `Ver ${name.toLocaleLowerCase()}`,
    noWindows: "No hay ventanas abiertas",
    addDesktop: "Añadir escritorio",
    newDesktop: "Nuevo escritorio",
  },
  appExpose: {
    title: "App Exposé",
    label: (name) => `Ventanas de ${name}`,
    open: "Ventanas abiertas",
    minimized: "Ventanas minimizadas",
  },
  switcher: { label: "Cambiar de aplicación" },
  notifications: {
    count: (count) => f.plural(count, { one: "1 notificación", other: `${f.number(count)} notificaciones` }),
    now: "ahora",
    minutesAgo: (minutes) => `hace ${f.number(minutes)} min`,
    hoursAgo: (hours) => `hace ${f.number(hours)} h`,
    yesterday: "ayer",
    daysAgo: (days) => f.plural(days, { one: "hace 1 día", other: `hace ${f.number(days)} días` }),
    closeBanner: "Ocultar",
    closeNotification: (action, name, title) => `${action} la notificación de ${name}: ${title}`,
    reply: "Responder",
    message: "Mensaje",
    send: "Enviar",
    none: "No hay notificaciones",
    clear: "Borrar",
    clearApp: (name) => `Borrar las notificaciones de ${name}`,
  },
  password: {
    placeholder: "Introduce la contraseña",
    label: "Contraseña",
    incorrect: "Contraseña incorrecta",
    tryAgainIn: (wait) => `Vuelve a intentarlo en ${wait}`,
    incorrectTryAgainIn: (wait) => `Contraseña incorrecta. Vuelve a intentarlo en ${wait}`,
    tryAgainInSeconds: (seconds) => `Vuelve a intentarlo en ${f.number(seconds)} s`,
    canTryAgain: "Ya puedes volver a intentarlo",
    capsLock: "Bloq Mayús activado",
    notChecked: "No se ha podido comprobar la contraseña",
    checking: "Comprobando…",
  },
  screenshots: {
    captureScreen: "Capturar pantalla completa",
    captureWindow: "Capturar ventana seleccionada",
    captureSelection: "Capturar parte seleccionada",
    screen: "Pantalla",
    window: "Ventana",
    selection: "Selección",
    desktop: "Escritorio",
    documents: "Documentos",
    none: "Ninguno",
    chooseWindow: "Elegir una ventana",
    selectPortion: "Seleccionar una parte de la pantalla",
    screenshot: "Captura de pantalla",
    capture: "Capturar",
    saveTo: "Guardar en",
    timer: "Temporizador",
    openHint: "Ábrela",
    saved: "Captura de pantalla guardada",
  },
  wallpaper: "Fondo de escritorio",
  desktop: {
    kinds: {
      folder: "Carpeta",
      image: "Imagen",
      audio: "Audio",
      video: "Vídeo",
      pdf: "Documento PDF",
      text: "Documento de texto",
      archive: "Archivo comprimido",
      document: "Documento",
    },
    open: "Abrir",
    moveToTrash: "Trasladar a la papelera",
    selectAll: "Seleccionar todo",
    file: "Archivo",
    edit: "Edición",
    intoItself: "Un ítem no se puede trasladar dentro de sí mismo",
    place: (name) => `“${name}” se queda donde está`,
    taken: (name) => `Ya hay un ítem llamado “${name}”`,
    copy: "copia",
  },
  sheet: {
    install: (name) => `¿Instalar ${name}?`,
    update: (name) => `¿Actualizar ${name}?`,
    uninstall: (name) => `¿Eliminar ${name} y sus datos?`,
    consent: (name) => `${name} pide más acceso`,
    permission: (name, what) => `¿Permitir que ${name} ${what}?`,
    installButton: "Instalar",
    updateButton: "Actualizar",
    uninstallButton: "Eliminar",
    allow: "Permitir",
    dontAllow: "No permitir",
    cancel: "Cancelar",
    version: (version, size) => `Versión ${version} · ${size}`,
    uninstallBody: "Se eliminarán la app y sus datos en esta cuenta. Puedes volver a instalarla desde Atlas.",
    consentBody: (version) => `La versión ${version} usa:`,
    keyRotation: "El desarrollador firma esta versión con una clave nueva de su linaje.",
    later: "Se preguntará cuando la app lo use",
    unavailable: {
      install: "No se puede instalar la app",
      uninstall: "No se puede eliminar la app",
      update: "No se puede actualizar",
    },
    unavailableBody:
      "El servicio que instala las apps en este ordenador no está funcionando. Inténtalo de nuevo cuando vuelva a estar disponible.",
    ok: "Aceptar",
    emptyTrash: "¿Seguro que quieres eliminar permanentemente los ítems de la Papelera?",
    emptyTrashBody: (items) =>
      `${f.plural(items, { one: "Se eliminará 1 ítem", other: `Se eliminarán ${f.number(items)} ítems` })}. No puedes deshacer esta acción.`,
    emptyTrashButton: "Vaciar",
    wifiPassword: (ssid, kind) => `La red Wi-Fi “${ssid}” requiere una contraseña ${kind}.`,
    wifiIncorrect: "La contraseña no es correcta. Inténtalo de nuevo.",
    wifiJoin: "Conectarse",
    wifiField: "Contraseña",
    showPassword: "Mostrar contraseña",
    hidePassword: "Ocultar contraseña",
    wifiFailed: (ssid) => `No se ha podido conectar a la red “${ssid}”.`,
    wifiUnsupported: (ssid) => `OxyOS todavía no puede conectarse a “${ssid}”.`,
    wifiEnterprise: "Las redes empresariales (802.1X) aún no son compatibles.",
    wifiWep: "Las redes WEP no son compatibles: no son seguras.",
    wifiOtherUnsupported: "Esta red aún no es compatible.",
    bluetoothPair: "Enlazar",
    bluetoothConfirm: (name) => `Confirma que “${name}” muestra este código.`,
    bluetoothAuthorize: (name) => `“${name}” quiere enlazarse con este ordenador.`,
    bluetoothService: (name, what) => `¿Permitir que “${name}” ${what}?`,
    bluetoothServices: {
      audio: "reproduzca y grabe audio",
      input: "se use como dispositivo de entrada",
      phone: "se conecte para hacer llamadas",
      files: "envíe y reciba archivos",
      network: "comparta una conexión de red",
      other: "se conecte a este ordenador",
    },
    bluetoothDisplay: (name) => `Escribe este código en “${name}” y pulsa Retorno.`,
    bluetoothPasskey: (name) => `Introduce el código que muestra “${name}”.`,
    bluetoothPin: (name) => `Introduce el PIN de “${name}”.`,
    bluetoothPinHint: "Si no lo sabes, prueba con 0000 o 1234.",
    bluetoothCodeField: "Código",
    bluetoothPinField: "PIN",
    bluetoothFailed: (name) => `No se ha podido conectar a “${name}”.`,
    bluetoothRejected: "El código no coincidía o el dispositivo ha rechazado la conexión.",
    bluetoothTimeout: "Asegúrate de que está encendido, cerca y en modo de enlace.",
    bluetoothOtherFailure: "Asegúrate de que está encendido y cerca.",
    bluetoothCodeSaid: (digits) => `Código: ${digits}`,
    units: ["bytes", "kB", "MB", "GB"],
    capabilities: {
      network: ["Internet", "conectarse a Internet"],
      notifications: ["Notificaciones", "enviarte notificaciones"],
      files: ["Tus archivos", "acceder a tus archivos"],
      camera: ["Cámara", "usar la cámara"],
      microphone: ["Micrófono", "usar el micrófono"],
      location: ["Ubicación", "usar tu ubicación"],
      "oxy-account": ["Tu cuenta Oxy", "usar tu cuenta Oxy"],
      "open-files": ["Abrir archivos", "abrir archivos"],
      "url-schemes": ["Abrir enlaces", "abrir enlaces"],
      background: ["En segundo plano", "funcionar en segundo plano"],
      "login-item": ["Al iniciar sesión", "abrirse al iniciar sesión"],
    },
  },
});

/** Each language's catalogue, made for a locale's formats. */
export const CATALOGUES: Record<Language, (formats: Formats) => Catalogue> = { en, es };

/** The shell's words in a locale, with the language they are in, the locale they format in and
    its formats. */
export type ShellMessages = Catalogue & { language: Language; locale: string; format: Formats };

const made = new Map<string, ShellMessages>();

/**
 * The shell's catalogue for a BCP 47 locale (`es-ES`, `en-US`, `es-MX-u-hc-h12`): its language's
 * words, formatting in the locale. `language` is the one Bloom resolved where the caller has it,
 * so the shell and Bloom's own components always speak the same. Made once per locale.
 */
export function shellMessages(locale: string, language: Language = languageOf(locale)): ShellMessages {
  const key = `${language}|${locale}`;
  let messages = made.get(key);
  if (!messages) {
    const tag = usable(locale, language);
    const format = formats(tag);
    messages = { ...CATALOGUES[language](format), language, locale: tag, format };
    made.set(key, messages);
  }
  return messages;
}
