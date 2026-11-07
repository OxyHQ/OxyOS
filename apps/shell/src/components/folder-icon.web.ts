import type { ImageSourcePropType } from "react-native";
import folder from "../../assets/folder@2x.png";

/** A folder on the desktop (docs/adr/0047-finder-drags-and-the-desktop.mdx): Dropsea's folder, so a
    folder looks the same on the desktop as in the file manager. */
export const FOLDER_ICON: ImageSourcePropType = { uri: folder };
