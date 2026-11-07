/** Development-only catalog and surface fixture. Never imported by apps/shell or the OS session. */
import type {
  ShellApplication,
  ShellWallpaper,
  ShellSnapshot,
  ShellMenu,
} from "../../../apps/shell/src/model";
import type { NetworkState } from "../../../apps/shell/src/network";
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

/** The fixture in the preview's language. */
export function previewSnapshot(messages: ShellMessages): ShellSnapshot {
  const words = FIXTURE_WORDS[messages.language];
  return {
    applications,
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
