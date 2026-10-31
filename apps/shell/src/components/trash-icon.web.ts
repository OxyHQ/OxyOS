import type { ImageSourcePropType } from "react-native";
import trash from "../../assets/trash.png";

/** The Trash at the dock's end (docs/adr/0034-the-dock.mdx): the bin the platform's author gave,
    the same whether it holds anything or not. */
export const TRASH_ICON: ImageSourcePropType = { uri: trash };
