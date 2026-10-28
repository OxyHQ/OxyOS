/** Development-only catalog and surface fixture. Never imported by apps/shell or the OS session. */
import type {
  ShellApplication,
  ShellSnapshot,
  ShellMenu,
} from "../../../apps/shell/src/model";
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
import wallpaper from "../../../assets/wallpapers/solar-gradients/frames/frame-08.jpg";
export const previewWallpaper = { uri: wallpaper };
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
const menus: ShellMenu[] = [
  {
    label: "OxyOS",
    items: [{ id: "system.about", label: "Acerca de OxyOS", enabled: false }],
  },
  {
    label: "Archivo",
    items: [{ id: "app.new", label: "Nueva ventana", enabled: false }],
  },
  {
    label: "Edición",
    items: [
      { id: "edit.undo", label: "Deshacer", enabled: false },
      { id: "edit.copy", label: "Copiar", enabled: false },
      { id: "edit.paste", label: "Pegar", enabled: false },
    ],
  },
  {
    label: "Ver",
    items: [
      { id: "view.overview", label: "Ver escritorios" },
      { id: "view.launcher", label: "Ver aplicaciones" },
    ],
  },
  { label: "Ir", items: [{ id: "view.launcher", label: "Aplicaciones" }] },
  {
    label: "Ventana",
    items: [{ id: "view.overview", label: "Todas las ventanas" }],
  },
  {
    label: "Ayuda",
    items: [{ id: "help", label: "Ayuda de OxyOS", enabled: false }],
  },
];
export const previewSnapshot: ShellSnapshot = {
  applications,
  folders: [
    { id: "finance", name: "Finanzas", appIds: ["peable", "fairwallet"] },
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
      title: "Astro · Documentación",
      workspaceId: "1",
      thumbnail: { uri: splitThumbnail },
      thumbnailSize: { width: 2880, height: 1800 },
    },
    {
      id: "preview-work",
      appId: "astro",
      title: "Astro · Trabajo",
      workspaceId: "2",
      thumbnail: { uri: workThumbnail },
      thumbnailSize: { width: 2880, height: 1800 },
    },
  ],
  workspaces: [
    { id: "1", name: "Escritorio 1" },
    { id: "2", name: "Escritorio 2" },
  ],
  workspaceId: "1",
  menus,
};
