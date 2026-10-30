/**
 * An item's icon, as Finder draws one: a folder in the accent colour, a page with its corner
 * turned and its kind's glyph on it, a volume's drive, the Trash; or, for a picture the host can
 * decode cheaply, the picture itself, fitted into the icon's square (docs/adr/0027-files.mdx).
 * The artwork is the application's own, drawn at any size with react-native-svg; the glyphs are
 * Bloom's.
 */
import * as React from "react";
import { Image, View } from "react-native";
import Svg, { Defs, LinearGradient, Path, Rect, Stop } from "react-native-svg";
import {
  RiCodeSSlashLine,
  RiDiscLine,
  RiFileExcel2Line,
  RiFileImageLine,
  RiFileMusicLine,
  RiFilePdf2Line,
  RiFileTextLine,
  RiFileWord2Line,
  RiFileZipLine,
  RiFilmLine,
  RiGalleryView,
  RiHardDrive2Line,
} from "@oxy.so/bloom/icons";
import { useTheme } from "@oxy.so/bloom/theme";
import { hasThumbnail, kindOf, type FileEntry, type Kind } from "./model";

type Glyph = React.ComponentType<{ width?: number; height?: number; fill?: string }>;

const GLYPHS: Partial<Record<Kind, Glyph>> = {
  image: RiFileImageLine,
  audio: RiFileMusicLine,
  video: RiFilmLine,
  pdf: RiFilePdf2Line,
  text: RiFileTextLine,
  code: RiCodeSSlashLine,
  document: RiFileWord2Line,
  spreadsheet: RiFileExcel2Line,
  presentation: RiGalleryView,
  archive: RiFileZipLine,
  "disk-image": RiDiscLine,
};

/** The folder's two tones from the accent: its back and tab darker, its front lighter. */
function FolderArt({ size, colour }: { size: number; colour: string }) {
  return (
    <Svg width={size} height={size} viewBox="0 0 64 64">
      <Defs>
        <LinearGradient id="front" x1="0" y1="0" x2="0" y2="1">
          <Stop offset="0" stopColor={colour} stopOpacity={0.72} />
          <Stop offset="1" stopColor={colour} stopOpacity={0.92} />
        </LinearGradient>
      </Defs>
      <Path
        d="M6 14a4 4 0 0 1 4-4h14.3a4 4 0 0 1 2.9 1.2L30 14h24a4 4 0 0 1 4 4v32a4 4 0 0 1-4 4H10a4 4 0 0 1-4-4z"
        fill={colour}
      />
      <Rect x={6} y={20} width={52} height={34} rx={4} fill="url(#front)" />
      <Rect x={6} y={20} width={52} height={1.4} fill="#ffffff" opacity={0.35} />
    </Svg>
  );
}

/** A page with its top corner turned, its kind's glyph in the middle. */
function PageArt({
  size,
  kind,
  paper,
  edge,
  ink,
}: {
  size: number;
  kind: Kind;
  paper: string;
  edge: string;
  ink: string;
}) {
  const Glyph = GLYPHS[kind] ?? RiFileTextLine;
  const glyph = Math.round(size * 0.36);
  return (
    <View style={{ width: size, height: size }}>
      <Svg width={size} height={size} viewBox="0 0 64 64">
        <Path
          d="M14 4h26l14 14v40a2 2 0 0 1-2 2H14a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z"
          fill={paper}
          stroke={edge}
          strokeWidth={1.2}
        />
        <Path d="M40 4v12a2 2 0 0 0 2 2h12" fill="none" stroke={edge} strokeWidth={1.2} />
      </Svg>
      <View style={{ position: "absolute", left: 0, right: 0, top: size * 0.4, alignItems: "center" }}>
        <Glyph width={glyph} height={glyph} fill={ink} />
      </View>
    </View>
  );
}

/** A volume: a drive in grey with its light. */
function VolumeArt({ size, colour }: { size: number; colour: string }) {
  const glyph = Math.round(size * 0.8);
  return (
    <View style={{ width: size, height: size, alignItems: "center", justifyContent: "center" }}>
      <RiHardDrive2Line width={glyph} height={glyph} fill={colour} />
    </View>
  );
}

export interface FileIconProps {
  entry: Pick<FileEntry, "name" | "folder" | "path" | "size">;
  size: number;
  /** A mounted volume's root, drawn as a drive. */
  volume?: boolean;
  /** Draw the picture itself, when the host can decode it cheaply. */
  thumbnail?: boolean;
}

export function FileIcon({ entry, size, volume, thumbnail = true }: FileIconProps) {
  const theme = useTheme();
  const colours = theme.colors;
  if (volume) return <VolumeArt size={size} colour={colours.textSecondary} />;
  if (entry.folder) return <FolderArt size={size} colour={colours.primary} />;
  const kind = kindOf(entry);
  if (thumbnail && size >= 32 && hasThumbnail(entry as FileEntry)) {
    // The host decodes it on a thread of its own, at the size it is drawn (crates/oxyos-image).
    return (
      <View style={{ width: size, height: size, alignItems: "center", justifyContent: "center" }}>
        <Image
          source={{ uri: entry.path }}
          style={{ width: size, height: size, borderRadius: 4 }}
          resizeMode="contain"
          accessible={false}
        />
      </View>
    );
  }
  return (
    <PageArt
      size={size}
      kind={kind}
      paper={theme.isDark ? "#2c2c30" : "#ffffff"}
      edge={theme.isDark ? "#5a5a62" : "#c9c9cf"}
      ink={colours.textSecondary}
    />
  );
}
