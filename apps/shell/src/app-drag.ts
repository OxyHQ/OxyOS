/**
 * An application dragged within the shell or from an application (docs/adr/0034-the-dock.mdx,
 * decision 10): its registry id, offered as the platform's media type for an application, as
 * `application/x-oxyos-files` says a drag carries files (docs/adr/0035-drag-and-drop.mdx).
 */
export const APPLICATION_TYPE = "application/x-oxyos-application";

/** How far a mouse press moves on an icon before it drags it, as Dropsea's items and a Mac's. */
export const DRAG_SLOP = 4;

/**
 * The application the shell itself is dragging now, from the dock or Launchpad: a drag over the
 * dock offers only its types before the drop (the protocol's protected mode), and the shell, its
 * own source, knows what it drags. Null while it drags nothing.
 */
export const shellDrag: { current: string | null } = { current: null };
