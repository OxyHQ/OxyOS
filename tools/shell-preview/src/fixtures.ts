/** Development-only catalog and surface fixture. Never imported by apps/shell or the OS session. */
import type {
  ShellApplication,
  ShellWallpaper,
  ShellSnapshot,
  ShellMenu,
} from "../../../apps/shell/src/model";
import type { NetworkState } from "../../../apps/shell/src/network";
import type { SearchItem, SearchResults, ShellSearch } from "../../../apps/shell/src/platform";
import settingsPanes from "../../../apps/settings/searchable.json";
import type { BluetoothState } from "../../../apps/shell/src/bluetooth";
import type { SoundState } from "../../../apps/shell/src/sound";
import type { Language, ShellMessages } from "../../../apps/shell/src/messages";
import astro from "../assets/apps/astro.png";
import inbox from "../assets/apps/inbox.png";
import mention from "../assets/apps/mention.png";
import alia from "../assets/apps/alia.svg?raw";
import allo from "../assets/apps/allo.png";
import clarity from "../assets/apps/clarity.png";
import noted from "../assets/apps/noted.png";
import schedio from "../assets/apps/schedio.png";
import codea from "../assets/apps/codea.png";
import homiio from "../assets/apps/homiio.png";
import moovo from "../assets/apps/moovo.png";
import mercaria from "../assets/apps/mercaria.png";
import peable from "../assets/apps/peable.png";
import wallet from "../assets/apps/faircoin-wallet.png";
import goway from "../assets/apps/goway.png";
import commons from "../assets/apps/commons-app.png";
import accounts from "../assets/apps/accounts.png";
import settings from "../assets/apps/settings.png";
import dropsea from "../assets/apps/dropsea.png";
import astroThumbnail from "../assets/surfaces/astro.webp";
import inboxThumbnail from "../assets/surfaces/inbox.png";
import mentionThumbnail from "../assets/surfaces/mention.png";
import aliaThumbnail from "../assets/surfaces/alia.png";
import splitThumbnail from "../assets/surfaces/astro-split.webp";
import workThumbnail from "../assets/surfaces/astro-work.webp";
import wallpaper from "../../../config/includes.chroot_after_packages/usr/share/oxyos/wallpapers/oxyos/light.svg";
/** The catalog's default, light (docs/adr/0048-wallpapers.mdx): the preview has no session to tell
    it another. */
export const previewWallpaper: ShellWallpaper = { key: "oxyos/light", source: { uri: wallpaper }, fit: "fill", color: "#000000" };
const raster = (
  id: string,
  name: string,
  uri: string,
  pinned = false,
): ShellApplication => ({ id, name, icon: { uri }, pinned });
const applications: ShellApplication[] = [
  raster("astro", "Astro", astro, true),
  raster("inbox", "Inbox", inbox, true),
  raster("mention", "Mention", mention, true),
  { id: "alia", name: "Alia", icon: { svg: alia }, pinned: true },
  raster("allo", "Allo", allo, true),
  raster("clarity", "Clarity", clarity, true),
  raster("noted", "Noted", noted, true),
  raster("schedio", "Schedio", schedio, true),
  raster("codea", "Codea", codea, true),
  raster("homiio", "Homiio", homiio),
  raster("moovo", "Moovo", moovo),
  raster("mercaria", "Mercaria", mercaria),
  raster("peable", "Peable", peable),
  raster("fairwallet", "FAIRWallet", wallet),
  raster("goway", "GoWay", goway),
  raster("commons", "Commons", commons),
  raster("accounts", "Oxy", accounts),
];
/** The fixture's own words, which stand for an application's (its menus, a folder, window
    titles): the shell's words come from its catalogue. */
const FIXTURE_ENGLISH = {
  file: "File",
  newWindow: "New Window",
  edit: "Edit",
  undo: "Undo",
  copy: "Copy",
  paste: "Paste",
  go: "Go",
  window: "Window",
  allWindows: "All Windows",
  help: "Help",
  oxyosHelp: "OxyOS Help",
  finance: "Finance",
  documentation: "Documentation",
  work: "Work",
};
const FIXTURE_WORDS: Record<Language, typeof FIXTURE_ENGLISH> = {
  en: FIXTURE_ENGLISH,
  es: {
    file: "Archivo",
    newWindow: "Nueva ventana",
    edit: "Edición",
    undo: "Deshacer",
    copy: "Copiar",
    paste: "Pegar",
    go: "Ir",
    window: "Ventana",
    allWindows: "Todas las ventanas",
    help: "Ayuda",
    oxyosHelp: "Ayuda de OxyOS",
    finance: "Finanzas",
    documentation: "Documentación",
    work: "Trabajo",
  },
};

function previewMenus(messages: ShellMessages): ShellMenu[] {
  const words = FIXTURE_WORDS[messages.language];
  return [
    {
      label: "OxyOS",
      items: [{ id: "system.about", label: messages.menus.about, enabled: false }],
    },
    {
      label: words.file,
      items: [{ id: "app.new", label: words.newWindow, enabled: false }],
    },
    {
      label: words.edit,
      items: [
        { id: "edit.undo", label: words.undo, enabled: false },
        { id: "edit.copy", label: words.copy, enabled: false },
        { id: "edit.paste", label: words.paste, enabled: false },
      ],
    },
    {
      label: messages.menus.view,
      items: [
        { id: "view.overview", label: messages.common.showDesktops },
        { id: "view.launcher", label: messages.menus.showApplications },
      ],
    },
    { label: words.go, items: [{ id: "view.launcher", label: messages.topBar.applications }] },
    {
      label: words.window,
      items: [{ id: "view.overview", label: words.allWindows }],
    },
    {
      label: words.help,
      items: [{ id: "help", label: words.oxyosHelp, enabled: false }],
    },
  ];
}

/** The system's applications, named in the preview's language as the registry names them. */
function systemApplications(language: Language): ShellApplication[] {
  return [
    raster("so.oxy.settings", language === "es" ? "Ajustes" : "Settings", settings),
    { ...raster("so.oxy.dropsea", "Dropsea", dropsea), opens: ["inode/directory"] },
  ];
}

/** The fixture in the preview's language. */
export function previewSnapshot(messages: ShellMessages): ShellSnapshot {
  const words = FIXTURE_WORDS[messages.language];
  return {
    applications: [...applications, ...systemApplications(messages.language)],
    folders: [
      { id: "finance", name: words.finance, appIds: ["peable", "fairwallet"] },
    ],
    // Published Oxy app screenshots for overview composition, not live processes.
    windows: [
      {
        id: "preview-inbox",
        appId: "inbox",
        title: "Inbox",
        workspaceId: "1",
        thumbnail: { uri: inboxThumbnail },
        thumbnailSize: { width: 1600, height: 1066 },
      },
      {
        id: "preview-astro",
        appId: "astro",
        title: "Astro",
        workspaceId: "1",
        thumbnail: { uri: astroThumbnail },
        thumbnailSize: { width: 2880, height: 1800 },
      },
      {
        id: "preview-alia",
        appId: "alia",
        title: "Alia",
        workspaceId: "1",
        thumbnail: { uri: aliaThumbnail },
        thumbnailSize: { width: 2303, height: 1535 },
      },
      {
        id: "preview-mention",
        appId: "mention",
        title: "Mention",
        workspaceId: "1",
        thumbnail: { uri: mentionThumbnail },
        thumbnailSize: { width: 1600, height: 1066 },
      },
      {
        id: "preview-split",
        appId: "astro",
        title: `Astro · ${words.documentation}`,
        workspaceId: "1",
        thumbnail: { uri: splitThumbnail },
        thumbnailSize: { width: 2880, height: 1800 },
      },
      {
        id: "preview-work",
        appId: "astro",
        title: `Astro · ${words.work}`,
        workspaceId: "2",
        thumbnail: { uri: workThumbnail },
        thumbnailSize: { width: 2880, height: 1800 },
      },
    ],
    workspaces: [
      { id: "1", name: messages.overview.desktop(1) },
      { id: "2", name: messages.overview.desktop(2) },
    ],
    workspaceId: "1",
    menus: previewMenus(messages),
  };
}

/** A played machine's networks (docs/adr/0038-networks.mdx), so the Control Center's Wi-Fi view
    can be tried in the browser: one joined, two more known, and others nearby. */
export const previewNetworks: NetworkState = {
  available: true,
  wifi: {
    device: true,
    enabled: true,
    hardwareBlocked: false,
    state: "connected",
    current: { ssid: "Oxy", signal: 73, security: "wpa", known: true, current: true, supported: true },
    networks: [
      { ssid: "Oxy", signal: 73, security: "wpa", known: true, current: true, supported: true },
      { ssid: "Vecinos 5G", signal: 81, security: "wpa3", known: false, current: false, supported: true },
      { ssid: "Casa", signal: 58, security: "wpa", known: true, current: false, supported: true },
      { ssid: "Café Central", signal: 45, security: "open", known: false, current: false, supported: true },
      { ssid: "Oficina", signal: 38, security: "enterprise", known: false, current: false, supported: false },
      { ssid: "Biblioteca", signal: 22, security: "owe", known: true, current: false, supported: true },
      { ssid: "Antiguo", signal: 12, security: "wep", known: false, current: false, supported: false },
    ],
    known: [
      { id: "1d0e3f7a-0000-4000-8000-000000000001", ssid: "Oxy", autoJoin: true },
      { id: "1d0e3f7a-0000-4000-8000-000000000002", ssid: "Casa", autoJoin: true },
      { id: "1d0e3f7a-0000-4000-8000-000000000003", ssid: "Biblioteca", autoJoin: false },
    ],
    details: {
      interface: "wlp0s20f3",
      hardwareAddress: "3C:A9:F4:12:34:56",
      ipv4: { address: "192.168.1.42", prefix: 24, subnetMask: "255.255.255.0" },
      router: "192.168.1.1",
      dns: ["192.168.1.1"],
      ipv6: [],
    },
  },
  ethernet: { device: true, state: "unplugged", speed: 0, details: null },
  joining: null,
};

const DAY = 24 * 60 * 60 * 1000;
const HOME = "file:///home/ana";
/** A played file in the person's home. */
const file = (
  path: string,
  category: SearchItem["category"],
  extra: Partial<SearchItem> = {},
): SearchItem => {
  const uri = `${HOME}/${path.split("/").map(encodeURIComponent).join("/")}`;
  const parts = path.split("/");
  return {
    id: uri,
    uri,
    category,
    title: parts[parts.length - 1],
    location: parts.length > 1 ? parts[parts.length - 2] : "ana",
    ...extra,
  };
};

/** The played documents, folders and applications' items a person's index would hold. */
function previewDocuments(now: number): SearchItem[] {
  return [
    file("Documents/Proyectos", "folder", { modified: now - 3 * DAY, created: now - 200 * DAY }),
    file("Documents/Presupuesto 2026.pdf", "pdf", {
      contentType: "application/pdf",
      size: 482_133,
      modified: now - 2 * DAY,
      created: now - 40 * DAY,
      lastUsed: now - DAY,
      tags: ["Work"],
      snippet: "Presupuesto anual del equipo de producto: infraestructura, diseño y lanzamiento de OxyOS.",
    }),
    file("Documents/Spotlight design review.pdf", "pdf", {
      contentType: "application/pdf",
      size: 2_301_442,
      modified: now - 5 * DAY,
      created: now - 6 * DAY,
      snippet: "Spotlight: a floating panel, its field a fifth of the way down, results grouped by kind beside a preview.",
    }),
    file("Documents/Oxy Roadmap.docx", "document", {
      contentType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      size: 88_410,
      modified: now - 9 * DAY,
      created: now - 120 * DAY,
    }),
    file("Desktop/notas.txt", "text", {
      contentType: "text/plain",
      size: 1_204,
      modified: now - 4 * 60 * 60 * 1000,
      created: now - 30 * DAY,
      snippet: "Comprar pan. Llamar a Marta. Revisar el diseño de Spotlight y los atajos de teclado.",
    }),
    file("Code/OxyOS/apps/shell/src/spotlight.ts", "source", {
      contentType: "text/x-typescript",
      size: 13_870,
      modified: now - 60 * 60 * 1000,
      created: now - DAY,
    }),
    file("Documents/Keynote launch.pptx", "presentation", {
      contentType: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
      size: 12_840_221,
      modified: now - 12 * DAY,
    }),
    file("Documents/Gastos.xlsx", "spreadsheet", {
      contentType: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
      size: 40_112,
      modified: now - 20 * DAY,
    }),
    file("Pictures/Astro.webp", "image", { contentType: "image/webp", size: 361_442, icon: astroThumbnail, modified: now - 7 * DAY }),
    file("Pictures/Alia.png", "image", { contentType: "image/png", size: 1_093_002, icon: aliaThumbnail, modified: now - 8 * DAY }),
    file("Movies/Demo OxyOS.mp4", "movie", { contentType: "video/mp4", size: 84_200_113, modified: now - 15 * DAY }),
    file("Music/Allo theme.mp3", "music", { contentType: "audio/mpeg", size: 5_402_884, modified: now - 50 * DAY }),
    file("Downloads/Backup.zip", "archive", { contentType: "application/zip", size: 230_118_222, modified: now - 60 * DAY }),
    {
      id: "inbox:message-1",
      category: "mail",
      title: "Re: Spotlight design review",
      application: "inbox",
      url: "inbox://message/1",
      snippet: "The sections look right. Can the preview show the file's tags too?",
      modified: now - 3 * 60 * 60 * 1000,
    },
    {
      id: "schedio:event-1",
      category: "event",
      title: "Spotlight review",
      application: "schedio",
      url: "schedio://event/1",
      location: "Schedio",
      modified: now - DAY,
    },
  ];
}

const plain = (value: string) =>
  value
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .trim();

/**
 * A played search service (docs/adr/0051-search.mdx), so Spotlight and the launcher's search can be
 * tried in the browser: the fixture's applications, Settings' declared pages (apps/settings/
 * searchable.json, as the service reads each package's) and a few played documents, ranked as a
 * person would expect (a title that begins with the text, a word of it that does, a title that
 * holds it, a keyword, then what the item says), applications and settings first among equals,
 * then what was opened last. Never imported by apps/shell or the OS session.
 */
export function previewSearch(snapshot: ShellSnapshot, language: Language): ShellSearch {
  const used = new Map<string, number>();
  const now = Date.now();
  const items: (SearchItem & { keywords?: readonly string[] })[] = [
    ...snapshot.applications.map((app) => ({
      id: `application:${app.id}`,
      category: "application" as const,
      title: app.name,
      application: app.id,
    })),
    ...settingsPanes.map((pane) => ({
      id: pane.id,
      category: "settings" as const,
      title: pane.titles[language],
      url: pane.url,
      application: "so.oxy.settings",
      keywords: pane.keywords[language],
    })),
    ...previewDocuments(now),
  ];
  const first = (category: SearchItem["category"]) => (category === "application" ? 0 : category === "settings" ? 1 : 2);
  return {
    query: ({ text, categories, limit = 50 }) => {
      const wanted = plain(text);
      const scored: { item: SearchItem; score: number }[] = [];
      for (const { keywords, ...item } of items) {
        if (categories && !categories.includes(item.category)) continue;
        const title = plain(item.title);
        const score = title.startsWith(wanted)
          ? 0
          : title.split(/[\s\-_.]+/).some((word) => word.startsWith(wanted))
            ? 1
            : title.includes(wanted)
              ? 2
              : keywords?.some((word) => plain(word).startsWith(wanted))
                ? 3
                : plain(item.snippet ?? "").includes(wanted)
                  ? 4
                  : -1;
        if (wanted && score >= 0) scored.push({ item: { ...item, lastUsed: used.get(item.id) ?? item.lastUsed }, score });
      }
      scored.sort(
        (a, b) =>
          a.score - b.score ||
          first(a.item.category) - first(b.item.category) ||
          (b.item.lastUsed ?? 0) - (a.item.lastUsed ?? 0) ||
          a.item.title.localeCompare(b.item.title),
      );
      const results: SearchResults = {
        items: scored.slice(0, limit).map(({ item }) => item),
        topHit: scored.length > 0 && scored[0].score <= 1 ? scored[0].item.id : null,
        indexing: false,
      };
      return new Promise((resolve) => setTimeout(() => resolve(results), 30));
    },
    watch: () => () => {},
    used: (id) => {
      used.set(id, Date.now());
      console.info("[shell preview] search used", id);
    },
  };
}
/** A played machine's sound (docs/adr/0056-sound-devices.mdx), so the Control Center's Sound view
    can be tried in the browser: the built-in speakers chosen, a display over HDMI, and AirPods. */
export const previewSound: SoundState = {
  available: true,
  output: {
    devices: [
      {
        id: "alsa_output.pci-0000_00_1f.3.analog-stereo",
        name: "Built-in Audio Analog Stereo",
        kind: "speakers",
        transport: "builtIn",
        selected: true,
      },
      {
        id: "alsa_output.pci-0000_00_1f.3.hdmi-stereo",
        name: "DELL U2720Q",
        kind: "hdmi",
        transport: "hdmi",
        selected: false,
      },
      {
        id: "bluez_output.AC_90_85_12_34_56.1",
        name: "AirPods Pro",
        kind: "headphones",
        transport: "bluetooth",
        selected: false,
      },
    ],
    volume: 0.5,
    muted: false,
    balance: 0,
  },
  input: {
    devices: [
      {
        id: "alsa_input.pci-0000_00_1f.3.analog-stereo",
        name: "Built-in Audio Analog Stereo",
        kind: "microphone",
        transport: "builtIn",
        selected: true,
      },
    ],
    volume: 0.7,
    muted: false,
    balance: null,
  },
};

/** A played machine's Bluetooth (docs/adr/0055-bluetooth.mdx), so the Control Center's Bluetooth
    view can be tried in the browser: headphones and a trackpad connected, a keyboard and a mouse
    paired, and a speaker nearby, which the view does not list. */
export const previewBluetooth: BluetoothState = {
  available: true,
  adapter: {
    name: "OxyOS",
    address: "3C:A9:F4:12:34:57",
    powered: true,
    blocked: false,
    discovering: false,
    discoverable: false,
  },
  devices: [
    {
      id: "4C:87:5D:10:20:30",
      address: "4C:87:5D:10:20:30",
      name: "Headphones Pro",
      kind: "headphones",
      paired: true,
      connected: true,
      busy: null,
      battery: 82,
      audio: true,
      input: false,
    },
    {
      id: "F0:B3:EC:11:22:33",
      address: "F0:B3:EC:11:22:33",
      name: "Speaker",
      kind: "speaker",
      paired: false,
      connected: false,
      busy: null,
      battery: null,
      audio: true,
      input: false,
    },
    {
      id: "D4:61:9D:44:55:66",
      address: "D4:61:9D:44:55:66",
      name: "Trackpad",
      kind: "trackpad",
      paired: true,
      connected: true,
      busy: null,
      battery: 9,
      audio: false,
      input: true,
    },
    {
      id: "D4:61:9D:77:88:99",
      address: "D4:61:9D:77:88:99",
      name: "Wireless Keyboard",
      kind: "keyboard",
      paired: true,
      connected: false,
      busy: null,
      battery: 64,
      audio: false,
      input: true,
    },
    {
      id: "D4:61:9D:AA:BB:CC",
      address: "D4:61:9D:AA:BB:CC",
      name: "Wireless Mouse",
      kind: "mouse",
      paired: true,
      connected: false,
      busy: null,
      battery: null,
      audio: false,
      input: true,
    },
  ],
};
