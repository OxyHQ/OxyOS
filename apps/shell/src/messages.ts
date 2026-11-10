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
    /** A window of an application that does not answer (docs/adr/0067-hangs-and-force-quit.mdx). */
    notRespondingTitle: (title: string) => `${title} (Not Responding)`,
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
    forceQuit: "Force Quit…",
    /** The battery, a level indicator: its charge and state are its value. */
    battery: "Battery",
    /** The wired network's value, as the Wi-Fi item's is the network joined. */
    connected: "Connected",
    /** The clock's hint: what pressing it does, which its name (the date and time) does not say. */
    clockHint: "Opens Notification Center",
  },
  /** The session started in Safe Mode (docs/adr/0054-recovery-and-erasing.mdx, decision 10), as a
      Mac's menu bar says "Safe Boot". */
  safeMode: "Safe Mode",
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
  /** AutoFill's list at a field (docs/adr/0050-keychain-autofill-and-passkeys.mdx). */
  autofill: {
    /** The list's name while nothing is highlighted. */
    list: "Passwords",
    /** A new password's suggestion, as Safari offers it. */
    strongPassword: "Use Strong Password",
    /** What a code's row is, for its login's username. */
    code: (username: string) => (username ? `Verification code for ${username}` : "Verification code"),
    /** An item, its second line, its place and how many there are:
        "bea@example.com, example.com, 1 of 2". */
    item: (primary: string, secondary: string, place: number, count: number) =>
      `${primary}, ${secondary}, ${f.number(place)} of ${f.number(count)}`,
  },
  /** An application's own menu, the desktop's while no application is active
      (docs/adr/0021-menu-bar.mdx, app-menu.ts), and the standard About panel. */
  menus: {
    /** The desktop's own menu, Finder's on a Mac, while no window is active. */
    desktop: "Desktop",
    view: "View",
    showApplications: "Show Applications",
    aboutApp: (name: string) => `About ${name}`,
    settings: "Settings…",
    hideApp: (name: string) => `Hide ${name}`,
    hideOthers: "Hide Others",
    showAll: "Show All",
    quitApp: (name: string) => `Quit ${name}`,
    emptyTrash: "Empty Trash…",
    /** The About panel's version line: the application's version and, where the platform knows
        it, its build (the package's version code), as a Mac's "Version 1.2 (345)". */
    version: (version: string, build?: number) =>
      build === undefined ? `Version ${version}` : `Version ${version} (${build})`,
    /** Its close button. */
    closeAbout: "Close",
  },
  /** The keys of a chord as menus show them (chord.ts): as a PC keyboard of the language labels
      them, its modifiers in a Mac's order. */
  keys: {
    ctrl: "Ctrl",
    alt: "Alt",
    shift: "Shift",
    super: "Super",
    enter: "Enter",
    escape: "Esc",
    tab: "Tab",
    space: "Space",
    backspace: "Backspace",
    delete: "Del",
    insert: "Ins",
    home: "Home",
    end: "End",
    pageUp: "PgUp",
    pageDown: "PgDn",
  },
  /** Sleep, restart, shut down and log out (docs/adr/0064-sleep-restart-shut-down-and-log-out.mdx):
      the Oxy menu's rows, the session's question and its countdown, and what it says when an
      application or the system kept it from ending. Each `action` is "restart", "shutDown" or
      "logOut" (and "sleep" for what failed). */
  power: {
    sleep: "Sleep",
    restartMenu: "Restart…",
    /** Option's alternate, and the question's and the login screen's button. */
    restartNow: "Restart",
    shutDownMenu: "Shut Down…",
    shutDownNow: "Shut Down",
    /** The question's and the login screen's button. */
    shutDownButton: "Shut Down",
    /** Log Out *name*…, as a Mac's always names the person (docs/adr/0020-oxy-account-login.mdx,
        decision 1 as amended on 2026-11-09): `name` their display name, else their username, else
        `unnamedAccount`, never the Unix user's. */
    logOutMenu: (name: string) => `Log Out ${name}…`,
    logOutNow: (name: string) => `Log Out ${name}`,
    /** The person with neither a display name nor a username, as the login screen calls them. */
    unnamedAccount: "Oxy Account",
    logOutButton: "Log Out",
    question: {
      shutDown: "Are you sure you want to shut down your computer now?",
      restart: "Are you sure you want to restart your computer now?",
      logOut: "Are you sure you want to quit all apps and log out now?",
    },
    /** What happens by itself, `seconds` from now, if the person does nothing. */
    countdown: {
      shutDown: (seconds: number) =>
        `If you do nothing, the computer will shut down automatically in ${f.plural(seconds, { one: "1 second", other: `${f.number(seconds)} seconds` })}.`,
      restart: (seconds: number) =>
        `If you do nothing, the computer will restart automatically in ${f.plural(seconds, { one: "1 second", other: `${f.number(seconds)} seconds` })}.`,
      logOut: (seconds: number) =>
        `If you do nothing, you will be logged out automatically in ${f.plural(seconds, { one: "1 second", other: `${f.number(seconds)} seconds` })}.`,
    },
    reopenWindows: "Reopen windows when signing back in",
    /** The checkbox's state, said with the question. */
    reopenWindowsOn: "Windows will reopen when you sign back in.",
    reopenWindowsOff: "Windows won’t reopen when you sign back in.",
    cancelled: {
      shutDown: (name: string) => `“${name}” canceled shut down.`,
      restart: (name: string) => `“${name}” canceled restart.`,
      logOut: (name: string) => `“${name}” canceled log out.`,
    },
    timedOut: {
      shutDown: (name: string) => `Shut down has timed out because the app “${name}” failed to quit.`,
      restart: (name: string) => `Restart has timed out because the app “${name}” failed to quit.`,
      logOut: (name: string) => `Log out has timed out because the app “${name}” failed to quit.`,
    },
    tryAgain: {
      shutDown: (name: string) => `To try again, quit ${name} and shut down again.`,
      restart: (name: string) => `To try again, quit ${name} and restart again.`,
      logOut: (name: string) => `To try again, quit ${name} and log out again.`,
    },
    failed: {
      shutDown: "Your computer can’t shut down now.",
      restart: "Your computer can’t restart now.",
      logOut: "You can’t log out now.",
      sleep: "Your computer can’t sleep now.",
    },
    /** Why: logind's `Can…` was not `yes`. */
    notPermitted: {
      shutDown: "Someone else is signed in to this computer, or an app is keeping it on. An administrator must shut it down.",
      restart: "Someone else is signed in to this computer, or an app is keeping it on. An administrator must restart it.",
      logOut: "OxyOS isn’t allowed to end this session.",
      sleep: "An app is keeping this computer awake, or it can’t sleep.",
    },
    /** Why: no logind, or the call failed. */
    unavailable: "The system didn’t respond. Try again in a moment.",
    /** Why: applications are being asked to quit already. */
    busy: "Apps are already being asked to quit.",
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
  /** The menu bar's backup item and its menu (docs/adr/0059-backups.mdx, backup.ts). */
  backup: {
    /** The item's name and its menu's. */
    title: "Backups",
    backingUp: "Backing Up…",
    preparing: "Preparing Backup…",
    cleaningUp: "Cleaning Up…",
    checking: "Verifying Backups…",
    /** How far a backup is: "12.3 MB of 240 MB", then the time left after a dash. */
    progress: (done: string, total: string) => `${done} of ${total}`,
    progressRemaining: (done: string, total: string, remaining: string) => `${done} of ${total} – ${remaining}`,
    lessThanAMinute: "Less than a minute remaining",
    minutesRemaining: (minutes: number) =>
      f.plural(minutes, { one: "About 1 minute remaining", other: `About ${f.number(minutes)} minutes remaining` }),
    hoursRemaining: (hours: number) =>
      f.plural(hours, { one: "About 1 hour remaining", other: `About ${f.number(hours)} hours remaining` }),
    daysRemaining: (days: number) =>
      f.plural(days, { one: "About 1 day remaining", other: `About ${f.number(days)} days remaining` }),
    latestTo: (name: string) => `Latest Backup to “${name}”:`,
    today: (time: string) => `Today, ${time}`,
    yesterday: (time: string) => `Yesterday, ${time}`,
    on: (date: string, time: string) => `${date}, ${time}`,
    noBackups: "No Backups Yet",
    failed: "Latest Backup Failed",
    /** Why the latest backup failed, as the service names it; any other is `failed`'s. */
    reasons: {
      full: "The backup disk is full.",
      unreachable: "The computer with the share could not be reached.",
      shareRefused: "The share did not accept the user name or password.",
      wrongPassword: "The backup password was not accepted.",
      inUse: "The backup is in use by another computer.",
      noBackup: "The backup could not be found.",
      damaged: "The backups could not be verified.",
      failed: "The backup could not be completed.",
    },
    diskNotConnected: "Backup Disk Not Connected",
    notAvailable: "Backup Not Available",
    held: "Waiting for Low Power Mode to Turn Off",
    backUpNow: "Back Up Now",
    skip: "Skip This Backup",
    browse: "Browse Backups",
    settings: "Open Backups Settings…",
    /** The item's value, as a screen reader says it. */
    saidBackingUp: (percent: string) => `Backing up, ${percent}`,
    saidPreparing: "Preparing backup",
    saidCleaningUp: "Cleaning up",
    saidChecking: "Verifying backups",
    saidToday: (time: string) => `Latest backup today, ${time}`,
    saidYesterday: (time: string) => `Latest backup yesterday, ${time}`,
    saidOn: (date: string, time: string) => `Latest backup on ${date}, ${time}`,
    saidNone: "No backups yet",
    saidFailed: "Latest backup failed",
    /** Sizes, in decimal units as a Mac shows them. */
    bytes: (count: number) => f.plural(count, { one: "1 byte", other: `${f.number(count)} bytes` }),
    size: (value: string, unit: string) => `${value} ${unit}`,
    units: ["KB", "MB", "GB", "TB"] as readonly string[],
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
    serviceDown: "The network service isn’t responding.",
    hardwareBlocked: "Wi-Fi is turned off with the computer’s switch.",
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
    back: "Back to Control Center",
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
    unavailable: "Bluetooth isn’t available.",
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
    notResponding: "App Not Responding",
    untitled: "Untitled",
    emptyTrash: "Empty Trash…",
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
  /** Spotlight (spotlight.ts, docs/adr/0051-search.mdx): its field, its sections and what a
      result's preview says of it. */
  spotlight: {
    label: "Spotlight",
    placeholder: "Spotlight Search",
    topHit: "Top Hit",
    /** Each kind's section, as a Mac's Spotlight titles them. */
    sections: {
      application: "Applications",
      settings: "System Settings",
      folder: "Folders",
      document: "Documents",
      pdf: "PDF Documents",
      text: "Text",
      source: "Developer",
      presentation: "Presentations",
      spreadsheet: "Spreadsheets",
      image: "Images",
      movie: "Movies",
      music: "Music",
      archive: "Archives",
      contact: "Contacts",
      event: "Events & Reminders",
      mail: "Mail",
      message: "Messages",
      other: "Other",
    },
    /** What one result of each kind is. */
    kinds: {
      application: "Application",
      settings: "System Settings",
      folder: "Folder",
      document: "Document",
      pdf: "PDF document",
      text: "Plain text",
      source: "Source code",
      presentation: "Presentation",
      spreadsheet: "Spreadsheet",
      image: "Image",
      movie: "Movie",
      music: "Music",
      archive: "Archive",
      contact: "Contact",
      event: "Event",
      mail: "Mail message",
      message: "Message",
      other: "Item",
    },
    /** The preview's facts. */
    kind: "Kind",
    size: "Size",
    created: "Created",
    modified: "Modified",
    lastOpened: "Last opened",
    where: "Where",
    tags: "Tags",
    /** Below the results while the index is still being built. */
    indexing: "Indexing — some results may be missing",
    /** A row's hint: what Return does, and what dragging a file's row does. */
    openHint: "Opens it",
    dragHint: "Drag to move or copy it",
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
  switcher: { label: "App Switcher" },
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
    incorrect: "Incorrect password. Try again.",
    tryAgainIn: (wait: string) => `Try again in ${wait}`,
    incorrectTryAgainIn: (wait: string) => `Incorrect password. Try again in ${wait}`,
    tryAgainInSeconds: (seconds: number) => `Try again in ${f.number(seconds)} s`,
    canTryAgain: "You can try again now",
    capsLock: "Caps Lock is on",
    notChecked: "The password couldn’t be checked",
    checking: "Checking…",
    pinPlaceholder: "Enter PIN",
    pinLabel: "PIN",
    pinIncorrect: "Incorrect PIN. Try again.",
    pinIncorrectTryAgainIn: (wait: string) => `Incorrect PIN. Try again in ${wait}`,
    usePassword: "Use Password",
    usePin: "Use PIN",
    pinLocked: "Too many incorrect PINs. Use your password.",
    pinLockedNoPassword: "Too many incorrect PINs. Log out, then sign in with Oxy at the login window to choose a new PIN.",
  },
  /** The home's standard folders as a person reads them, by their key (oxyos-files' favourites),
      whatever they are called on disk, as Finder names them. */
  places: {
    desktop: "Desktop",
    documents: "Documents",
    downloads: "Downloads",
    pictures: "Pictures",
    music: "Music",
    videos: "Videos",
  } as Record<string, string>,
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
    intoItself: "An item can’t be moved into itself",
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
    dontAllow: "Don’t Allow",
    cancel: "Cancel",
    version: (version: string, size: string) => `Version ${version} · ${size}`,
    uninstallBody: "The app and its data in this account will be deleted. You can install it again from Atlas.",
    consentBody: (version: string) => `Version ${version} uses:`,
    keyRotation: "The developer signs this version with a new key of its lineage.",
    later: "Asked when the app uses it",
    unavailable: {
      install: "Can’t Install the App",
      uninstall: "Can’t Delete the App",
      update: "Can’t Update",
    },
    unavailableBody: "The service that installs apps on this computer isn’t running. Try again once it’s available.",
    ok: "OK",
    emptyTrash: "Are you sure you want to permanently erase the items in the Trash?",
    emptyTrashBody: (items: number) =>
      `${f.plural(items, { one: "1 item will be erased", other: `${f.number(items)} items will be erased` })}. You can’t undo this action.`,
    emptyTrashButton: "Empty",
    reopenAgain: "The last time you logged in, OxyOS quit unexpectedly while reopening your apps.",
    reopenAgainBody: "Do you want to try to reopen them again?",
    reopenAgainButton: "Reopen",
    dontReopen: "Don’t Reopen",
    forceQuitTitle: "Force Quit Applications",
    forceQuitBody: "If an app doesn’t respond for a while, select its name and click Force Quit.",
    forceQuitNone: "No apps are open.",
    forceQuitList: "Apps",
    forceQuitButton: "Force Quit",
    notResponding: "Not Responding",
    wifiPassword: (ssid: string, kind: string) => `The Wi-Fi network “${ssid}” requires a ${kind} password.`,
    wifiIncorrect: "Incorrect password. Try again.",
    wifiJoin: "Join",
    wifiField: "Password",
    showPassword: "Show password",
    hidePassword: "Hide password",
    wifiFailed: (ssid: string) => `Couldn’t join the network “${ssid}”.`,
    wifiUnsupported: (ssid: string) => `OxyOS can’t join “${ssid}” yet.`,
    wifiEnterprise: "Enterprise (802.1X) networks aren’t supported yet.",
    wifiWep: "WEP networks aren’t supported: they aren’t secure.",
    wifiOtherUnsupported: "This network isn’t supported yet.",
    /** Passkeys (docs/adr/0050-keychain-autofill-and-passkeys.mdx, decision 7): the relying party
        (a domain), the account, the application asking, and the Oxy account's password. */
    passkeyGet: (site: string) => `Use your passkey for ${site}?`,
    passkeyCreate: (site: string) => `Create a passkey for ${site}?`,
    passkeyAccount: (account: string) => `Account: ${account}.`,
    passkeyCreateBody: "It’s saved in your Oxy account’s keychain, end-to-end encrypted, on all your devices.",
    passkeyRequestedBy: (app: string) => `Requested by ${app}.`,
    passkeyExcluded: (site: string) => `You already have a passkey for ${site}.`,
    passkeyExcludedBody: "There’s no need to create another one.",
    passkeyNone: (site: string) => `You don’t have a passkey for ${site}.`,
    passkeyNoneBody: "Sign in another way.",
    passkeyChoose: "Choose an account",
    passkeyField: "Oxy account password",
    passkeyContinue: "Continue",
    passkeyIncorrect: "Incorrect password. Try again.",
    passkeyThrottled: (seconds: number) =>
      `Too many attempts. Try again in ${f.plural(seconds, { one: "1 second", other: `${f.number(seconds)} seconds` })}.`,
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
    bluetoothPinHint: "If you don’t know it, try 0000 or 1234.",
    bluetoothCodeField: "Code",
    bluetoothPinField: "PIN",
    bluetoothFailed: (name: string) => `Couldn’t connect to “${name}”.`,
    bluetoothRejected: "The code didn’t match, or the device refused the connection.",
    bluetoothTimeout: "Make sure it’s turned on, nearby and in pairing mode.",
    bluetoothOtherFailure: "Make sure it’s turned on and nearby.",
    /** A code said digit by digit: "Code: 1 2 3 4 5 6". */
    bluetoothCodeSaid: (digits: string) => `Code: ${digits}`,
    /** An application asks to save a login it used (docs/adr/0050-keychain-autofill-and-passkeys.mdx,
        decision 5): its name, then the username (and the website it names). */
    savePassword: (name: string) => `Save password for ${name}?`,
    savePasswordNote: "It’s kept in your Oxy account’s keychain, end-to-end encrypted, and filled in for you next time.",
    save: "Save",
    notNow: "Not Now",
    /** An application that quit unexpectedly, and resetting one (docs/adr/0061). */
    crash: (name: string) => `${name} quit unexpectedly`,
    crashBody: "You can reopen it, or see what it reported.",
    crashAgain: (name: string) => `${name} keeps quitting unexpectedly`,
    crashAgainBody: (times: number) =>
      `It quit ${f.number(times)} times in the last 10 minutes. If it keeps quitting, resetting it may help.`,
    reopen: "Reopen",
    ignore: "Ignore",
    resetButtonEllipsis: "Reset…",
    reportButton: "Report…",
    reportTitle: (name: string) => `What ${name} reported`,
    copy: "Copy",
    copied: "Copied",
    done: "Done",
    reset: (name: string) => `Reset “${name}”?`,
    resetBody: (name: string) =>
      `${name}’s data and settings on this computer will be erased, and it will open as new. Your documents and files, and your passwords, are kept.`,
    resetQuits: (name: string) => `${name} will quit first.`,
    resetButton: "Reset",
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
      "login-item": ["At login", "open at login"],
    } as Record<string, readonly [string, string]>,
  },
});

/** The catalogue's shape, whatever its language. */
type Catalogue = ReturnType<typeof en>;

const es = (f: Formats): Catalogue => ({
  common: {
    close: "Cerrar",
    options: "Opciones",
    notRespondingTitle: (title) => `${title} (No responde)`,
    systemSettings: "Ajustes del Sistema",
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
    forceQuit: "Forzar salida…",
    battery: "Batería",
    connected: "Conectado",
    clockHint: "Abre el centro de notificaciones",
  },
  safeMode: "Modo seguro",
  inputSources: {
    item: "Fuente de entrada",
    pinyin: "Pinyin – Simplificado",
    japanese: "Japonés – Romaji",
    korean: "Coreano de 2 conjuntos",
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
  autofill: {
    list: "Contraseñas",
    strongPassword: "Usar contraseña segura",
    code: (username) => (username ? `Código de verificación de ${username}` : "Código de verificación"),
    item: (primary, secondary, place, count) => `${primary}, ${secondary}, ${f.number(place)} de ${f.number(count)}`,
  },
  menus: {
    desktop: "Escritorio",
    view: "Ver",
    showApplications: "Ver aplicaciones",
    aboutApp: (name) => `Acerca de ${name}`,
    settings: "Ajustes…",
    hideApp: (name) => `Ocultar ${name}`,
    hideOthers: "Ocultar otros",
    showAll: "Mostrar todo",
    quitApp: (name) => `Salir de ${name}`,
    emptyTrash: "Vaciar Papelera…",
    version: (version, build) =>
      build === undefined ? `Versión ${version}` : `Versión ${version} (${build})`,
    closeAbout: "Cerrar",
  },
  keys: {
    ctrl: "Ctrl",
    alt: "Alt",
    shift: "Mayús",
    super: "Super",
    enter: "Intro",
    escape: "Esc",
    tab: "Tab",
    space: "Espacio",
    backspace: "Retroceso",
    delete: "Supr",
    insert: "Insert",
    home: "Inicio",
    end: "Fin",
    pageUp: "RePág",
    pageDown: "AvPág",
  },
  power: {
    sleep: "Reposo",
    restartMenu: "Reiniciar…",
    restartNow: "Reiniciar",
    shutDownMenu: "Apagar equipo…",
    shutDownNow: "Apagar equipo",
    shutDownButton: "Apagar",
    logOutMenu: (name) => `Cerrar sesión de ${name}…`,
    logOutNow: (name) => `Cerrar sesión de ${name}`,
    unnamedAccount: "Cuenta de Oxy",
    logOutButton: "Cerrar sesión",
    question: {
      shutDown: "¿Seguro que quieres apagar el ordenador ahora?",
      restart: "¿Seguro que quieres reiniciar el ordenador ahora?",
      logOut: "¿Seguro que quieres salir de todas las apps y cerrar la sesión ahora?",
    },
    countdown: {
      shutDown: (seconds) =>
        `Si no haces nada, el ordenador se apagará automáticamente en ${f.plural(seconds, { one: "1 segundo", other: `${f.number(seconds)} segundos` })}.`,
      restart: (seconds) =>
        `Si no haces nada, el ordenador se reiniciará automáticamente en ${f.plural(seconds, { one: "1 segundo", other: `${f.number(seconds)} segundos` })}.`,
      logOut: (seconds) =>
        `Si no haces nada, la sesión se cerrará automáticamente en ${f.plural(seconds, { one: "1 segundo", other: `${f.number(seconds)} segundos` })}.`,
    },
    reopenWindows: "Volver a abrir las ventanas al volver a iniciar sesión",
    reopenWindowsOn: "Las ventanas se volverán a abrir al volver a iniciar sesión.",
    reopenWindowsOff: "Las ventanas no se volverán a abrir al volver a iniciar sesión.",
    cancelled: {
      shutDown: (name) => `“${name}” ha cancelado el apagado.`,
      restart: (name) => `“${name}” ha cancelado el reinicio.`,
      logOut: (name) => `“${name}” ha cancelado el cierre de sesión.`,
    },
    timedOut: {
      shutDown: (name) => `Se ha agotado el tiempo para apagar el ordenador porque la app “${name}” no se ha cerrado.`,
      restart: (name) => `Se ha agotado el tiempo para reiniciar porque la app “${name}” no se ha cerrado.`,
      logOut: (name) => `Se ha agotado el tiempo para cerrar la sesión porque la app “${name}” no se ha cerrado.`,
    },
    tryAgain: {
      shutDown: (name) => `Para volver a intentarlo, cierra ${name} y vuelve a apagar el ordenador.`,
      restart: (name) => `Para volver a intentarlo, cierra ${name} y reinicia de nuevo.`,
      logOut: (name) => `Para volver a intentarlo, cierra ${name} y vuelve a cerrar la sesión.`,
    },
    failed: {
      shutDown: "El ordenador no puede apagarse ahora.",
      restart: "El ordenador no puede reiniciarse ahora.",
      logOut: "No puedes cerrar la sesión ahora.",
      sleep: "El ordenador no puede entrar en reposo ahora.",
    },
    notPermitted: {
      shutDown: "Hay otra persona con la sesión iniciada en este ordenador, o una app lo mantiene encendido. Tiene que apagarlo un administrador.",
      restart: "Hay otra persona con la sesión iniciada en este ordenador, o una app lo mantiene encendido. Tiene que reiniciarlo un administrador.",
      logOut: "OxyOS no puede terminar esta sesión.",
      sleep: "Una app mantiene activo este ordenador, o no puede entrar en reposo.",
    },
    unavailable: "El sistema no ha respondido. Vuelve a intentarlo en un momento.",
    busy: "Ya se está pidiendo a las apps que se cierren.",
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
  backup: {
    title: "Copias de seguridad",
    backingUp: "Realizando copia de seguridad…",
    preparing: "Preparando copia de seguridad…",
    cleaningUp: "Limpiando…",
    checking: "Verificando copias de seguridad…",
    progress: (done, total) => `${done} de ${total}`,
    progressRemaining: (done, total, remaining) => `${done} de ${total} – ${remaining}`,
    lessThanAMinute: "Queda menos de un minuto",
    minutesRemaining: (minutes) =>
      f.plural(minutes, { one: "Queda 1 minuto aproximadamente", other: `Quedan unos ${f.number(minutes)} minutos` }),
    hoursRemaining: (hours) =>
      f.plural(hours, { one: "Queda 1 hora aproximadamente", other: `Quedan unas ${f.number(hours)} horas` }),
    daysRemaining: (days) =>
      f.plural(days, { one: "Queda 1 día aproximadamente", other: `Quedan unos ${f.number(days)} días` }),
    latestTo: (name) => `Última copia de seguridad en “${name}”:`,
    today: (time) => `Hoy, ${time}`,
    yesterday: (time) => `Ayer, ${time}`,
    on: (date, time) => `${date}, ${time}`,
    noBackups: "Aún no hay copias de seguridad",
    failed: "La última copia de seguridad ha fallado",
    reasons: {
      full: "El disco de copia de seguridad está lleno.",
      unreachable: "No se ha podido conectar con el ordenador del recurso compartido.",
      shareRefused: "El recurso compartido no ha aceptado el usuario o la contraseña.",
      wrongPassword: "No se ha aceptado la contraseña de la copia de seguridad.",
      inUse: "Otro ordenador está usando la copia de seguridad.",
      noBackup: "No se ha encontrado la copia de seguridad.",
      damaged: "No se han podido verificar las copias de seguridad.",
      failed: "No se ha podido completar la copia de seguridad.",
    },
    diskNotConnected: "El disco de copia de seguridad no está conectado",
    notAvailable: "La copia de seguridad no está disponible",
    held: "Esperando a que se desactive el modo de bajo consumo",
    backUpNow: "Hacer copia de seguridad ahora",
    skip: "Omitir esta copia de seguridad",
    browse: "Explorar copias de seguridad",
    settings: "Abrir ajustes de Copias de seguridad…",
    saidBackingUp: (percent) => `Realizando copia de seguridad, ${percent}`,
    saidPreparing: "Preparando copia de seguridad",
    saidCleaningUp: "Limpiando",
    saidChecking: "Verificando copias de seguridad",
    saidToday: (time) => `Última copia de seguridad hoy, ${time}`,
    saidYesterday: (time) => `Última copia de seguridad ayer, ${time}`,
    saidOn: (date, time) => `Última copia de seguridad el ${date}, ${time}`,
    saidNone: "Aún no hay copias de seguridad",
    saidFailed: "La última copia de seguridad ha fallado",
    bytes: (count) => f.plural(count, { one: "1 byte", other: `${f.number(count)} bytes` }),
    size: (value, unit) => `${value} ${unit}`,
    units: ["KB", "MB", "GB", "TB"],
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
    systemSettings: "Ajustes del Sistema…",
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
    hardwareBlocked: "La Wi-Fi está desactivada con el interruptor del ordenador.",
    off: "Wi-Fi desactivada",
    searching: "Buscando redes…",
    joined: (ssid) => `Conectado a ${ssid}`,
  },
  sound: {
    back: "Volver al centro de control",
    output: "Salida",
    settings: "Ajustes de Sonido…",
    builtInSpeakers: "Altavoces integrados",
    externalHeadphones: "Auriculares externos",
    lineOut: "Salida de línea",
  },
  bluetooth: {
    back: "Volver al centro de control",
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
    off: "El Bluetooth está desactivado.",
    blocked: "El Bluetooth está desactivado por un interruptor del equipo o por el modo avión.",
    unavailable: "El Bluetooth no está disponible.",
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
    notResponding: "La app no responde",
    untitled: "Sin título",
    emptyTrash: "Vaciar la Papelera…",
    moveLeft: "Mover a la izquierda",
    moveRight: "Mover a la derecha",
    removeFromDock: "Quitar del Dock",
    moved: (name, place, count) => `${name}, posición ${f.number(place)} de ${f.number(count)}`,
  },
  launchpad: {
    close: "Cerrar aplicaciones",
    search: "Buscar",
    folder: (name) => `Carpeta ${name}`,
    noMatches: (query) => `No hay apps que coincidan con “${query}”.`,
    page: (page) => `Página ${f.number(page)}`,
    results: (count) =>
      count === 0 ? "No hay resultados" : f.plural(count, { one: "1 resultado", other: `${f.number(count)} resultados` }),
  },
  spotlight: {
    label: "Spotlight",
    placeholder: "Búsqueda en Spotlight",
    topHit: "Mejor resultado",
    sections: {
      application: "Aplicaciones",
      settings: "Ajustes del Sistema",
      folder: "Carpetas",
      document: "Documentos",
      pdf: "Documentos PDF",
      text: "Texto",
      source: "Desarrollo",
      presentation: "Presentaciones",
      spreadsheet: "Hojas de cálculo",
      image: "Imágenes",
      movie: "Películas",
      music: "Música",
      archive: "Archivos comprimidos",
      contact: "Contactos",
      event: "Eventos y recordatorios",
      mail: "Correo",
      message: "Mensajes",
      other: "Otros",
    },
    kinds: {
      application: "Aplicación",
      settings: "Ajustes del Sistema",
      folder: "Carpeta",
      document: "Documento",
      pdf: "Documento PDF",
      text: "Texto sin formato",
      source: "Código fuente",
      presentation: "Presentación",
      spreadsheet: "Hoja de cálculo",
      image: "Imagen",
      movie: "Película",
      music: "Música",
      archive: "Archivo comprimido",
      contact: "Contacto",
      event: "Evento",
      mail: "Mensaje de correo",
      message: "Mensaje",
      other: "Ítem",
    },
    kind: "Clase",
    size: "Tamaño",
    created: "Creado",
    modified: "Modificado",
    lastOpened: "Última apertura",
    where: "Ubicación",
    tags: "Etiquetas",
    indexing: "Indexando: puede que falten resultados",
    openHint: "Lo abre",
    dragHint: "Arrástralo para trasladarlo o copiarlo",
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
  switcher: { label: "Selector de apps" },
  notifications: {
    count: (count) => f.plural(count, { one: "1 notificación", other: `${f.number(count)} notificaciones` }),
    now: "ahora",
    minutesAgo: (minutes) => `hace ${f.number(minutes)} min`,
    hoursAgo: (hours) => `hace ${f.number(hours)} h`,
    yesterday: "ayer",
    daysAgo: (days) => f.plural(days, { one: "hace 1 día", other: `hace ${f.number(days)} días` }),
    closeBanner: "Cerrar",
    closeNotification: (action, name, title) => `${action} la notificación de ${name}: ${title}`,
    reply: "Responder",
    message: "Mensaje",
    send: "Enviar",
    none: "No hay notificaciones",
    clear: "Borrar",
    clearApp: (name) => `Borrar las notificaciones de ${name}`,
  },
  password: {
    placeholder: "Introducir contraseña",
    label: "Contraseña",
    incorrect: "Contraseña incorrecta. Vuelve a intentarlo.",
    tryAgainIn: (wait) => `Vuelve a intentarlo en ${wait}`,
    incorrectTryAgainIn: (wait) => `Contraseña incorrecta. Vuelve a intentarlo en ${wait}`,
    tryAgainInSeconds: (seconds) => `Vuelve a intentarlo en ${f.number(seconds)} s`,
    canTryAgain: "Ya puedes volver a intentarlo",
    capsLock: "Bloq Mayús activado",
    notChecked: "No se ha podido comprobar la contraseña",
    checking: "Comprobando…",
    pinPlaceholder: "Introducir PIN",
    pinLabel: "PIN",
    pinIncorrect: "PIN incorrecto. Vuelve a intentarlo.",
    pinIncorrectTryAgainIn: (wait) => `PIN incorrecto. Vuelve a intentarlo en ${wait}`,
    usePassword: "Usar contraseña",
    usePin: "Usar PIN",
    pinLocked: "Demasiados PIN incorrectos. Usa tu contraseña.",
    pinLockedNoPassword: "Demasiados PIN incorrectos. Cierra la sesión e inicia sesión con Oxy en la ventana de inicio de sesión para elegir un PIN nuevo.",
  },
  places: {
    desktop: "Escritorio",
    documents: "Documentos",
    downloads: "Descargas",
    pictures: "Imágenes",
    music: "Música",
    videos: "Vídeos",
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
  wallpaper: "Fondo de pantalla",
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
    moveToTrash: "Trasladar a la Papelera",
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
      "El servicio que instala las apps en este ordenador no está funcionando. Vuelve a intentarlo cuando vuelva a estar disponible.",
    ok: "Aceptar",
    emptyTrash: "¿Seguro que quieres eliminar permanentemente los ítems de la Papelera?",
    emptyTrashBody: (items) =>
      `${f.plural(items, { one: "Se eliminará 1 ítem", other: `Se eliminarán ${f.number(items)} ítems` })}. No puedes deshacer esta acción.`,
    emptyTrashButton: "Vaciar",
    reopenAgain: "La última vez que iniciaste sesión, OxyOS se cerró inesperadamente al volver a abrir tus apps.",
    reopenAgainBody: "¿Quieres intentar volver a abrirlas?",
    reopenAgainButton: "Volver a abrir",
    dontReopen: "No volver a abrir",
    forceQuitTitle: "Forzar salida de las aplicaciones",
    forceQuitBody: "Si una app no responde durante un tiempo, selecciona su nombre y haz clic en Forzar salida.",
    forceQuitNone: "No hay apps abiertas.",
    forceQuitList: "Apps",
    forceQuitButton: "Forzar salida",
    notResponding: "No responde",
    wifiPassword: (ssid, kind) => `La red Wi-Fi “${ssid}” requiere una contraseña ${kind}.`,
    wifiIncorrect: "Contraseña incorrecta. Vuelve a intentarlo.",
    wifiJoin: "Conectarse",
    wifiField: "Contraseña",
    showPassword: "Mostrar contraseña",
    hidePassword: "Ocultar contraseña",
    wifiFailed: (ssid) => `No se ha podido conectar a la red “${ssid}”.`,
    wifiUnsupported: (ssid) => `OxyOS todavía no puede conectarse a “${ssid}”.`,
    wifiEnterprise: "Las redes empresariales (802.1X) aún no son compatibles.",
    wifiWep: "Las redes WEP no son compatibles: no son seguras.",
    wifiOtherUnsupported: "Esta red aún no es compatible.",
    passkeyGet: (site) => `¿Usar tu llave de acceso para ${site}?`,
    passkeyCreate: (site) => `¿Crear una llave de acceso para ${site}?`,
    passkeyAccount: (account) => `Cuenta: ${account}.`,
    passkeyCreateBody: "Se guarda en el llavero de tu cuenta de Oxy, cifrada de extremo a extremo, en todos tus dispositivos.",
    passkeyRequestedBy: (app) => `Lo pide ${app}.`,
    passkeyExcluded: (site) => `Ya tienes una llave de acceso para ${site}.`,
    passkeyExcludedBody: "No hace falta crear otra.",
    passkeyNone: (site) => `No tienes ninguna llave de acceso para ${site}.`,
    passkeyNoneBody: "Inicia sesión de otra forma.",
    passkeyChoose: "Elige una cuenta",
    passkeyField: "Contraseña de la cuenta de Oxy",
    passkeyContinue: "Continuar",
    passkeyIncorrect: "Contraseña incorrecta. Vuelve a intentarlo.",
    passkeyThrottled: (seconds) =>
      `Demasiados intentos. Vuelve a intentarlo en ${f.plural(seconds, { one: "1 segundo", other: `${f.number(seconds)} segundos` })}.`,
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
    savePassword: (name) => `¿Guardar la contraseña para ${name}?`,
    savePasswordNote: "Se guarda en el llavero de tu cuenta de Oxy, cifrada de extremo a extremo, y se rellena por ti la próxima vez.",
    save: "Guardar",
    notNow: "Ahora no",
    crash: (name) => `${name} se cerró inesperadamente`,
    crashBody: "Puedes volver a abrirla o ver lo que informó.",
    crashAgain: (name) => `${name} sigue cerrándose inesperadamente`,
    crashAgainBody: (times) =>
      `Se ha cerrado ${f.number(times)} veces en los últimos 10 minutos. Si sigue cerrándose, restablecerla puede ayudar.`,
    reopen: "Volver a abrir",
    ignore: "Ignorar",
    resetButtonEllipsis: "Restablecer…",
    reportButton: "Informe…",
    reportTitle: (name) => `Lo que informó ${name}`,
    copy: "Copiar",
    copied: "Copiado",
    done: "OK",
    reset: (name) => `¿Restablecer “${name}”?`,
    resetBody: (name) =>
      `Se borrarán los datos y los ajustes de ${name} en este ordenador, y se abrirá como nueva. Tus documentos y archivos, y tus contraseñas, se conservan.`,
    resetQuits: (name) => `Antes se cerrará ${name}.`,
    resetButton: "Restablecer",
    units: ["bytes", "kB", "MB", "GB"],
    capabilities: {
      network: ["Internet", "conectarse a Internet"],
      notifications: ["Notificaciones", "enviarte notificaciones"],
      files: ["Tus archivos", "acceder a tus archivos"],
      camera: ["Cámara", "usar la cámara"],
      microphone: ["Micrófono", "usar el micrófono"],
      location: ["Ubicación", "usar tu ubicación"],
      "oxy-account": ["Tu cuenta de Oxy", "usar tu cuenta de Oxy"],
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
