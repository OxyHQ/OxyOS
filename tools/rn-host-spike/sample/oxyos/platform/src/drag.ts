// `drag` (docs/adr/0058-the-platform-api.mdx, decision 1; docs/adr/0035-drag-and-drop.mdx):
// dragging an application's content to another application, and following drags over its window.
// Every application may, citing a press: a drag starts from the press the host holds (a press moved
// past its slop, as a Mac's `beginDraggingSession` from `mouseDragged`), and the compositor owns it
// from then on. Dropping in is a view's (the host's drop target, Bloom's DropZone), not a call.
import { PlatformError } from './errors.ts';
import type { CallOptions } from './frameworks.ts';
import { hostCall } from './host.ts';
import type { Events } from './session.ts';

/** What a drop does with what is dragged. */
export type DragAction = 'copy' | 'move' | 'link';
/** Why a drag ended without a drop: the person pressed Escape, released it over nothing that takes
    it, or it was cancelled otherwise (its window went, the session locked). */
export type DragEndReason = 'escape' | 'notAccepted' | 'cancelled';

/** What is dragged, each kind offered as its media types (`files` as `text/uri-list`). */
export interface DragData {
  /** `file://` URIs. */
  files?: readonly string[];
  text?: string;
  url?: string;
  html?: string;
  /** Other media types and their content as text, offered after the kinds above. */
  other?: Readonly<Record<string, string>>;
}

export interface DragRequest {
  data: DragData;
  /** The actions it allows (at least one). */
  actions: readonly DragAction[];
  /** The dragged view, by its native tag (`findNodeHandle`): the drag's image is its rendered
      subtree, as it looks, the point pressed in it kept under the pointer. None: the platform's. */
  preview?: number | null;
  /** How many items are dragged, the preview one of them: above one, the image is stacked and
      badged with the count. Default 1. */
  count?: number;
  /** Dropped on nothing, the image vanishes where it is instead of sliding back (an item dragged
      out of the dock). Default false. */
  vanish?: boolean;
}

/** How a drag ended: the action its drop took, or none and why. */
export type DragOutcome = { action: DragAction } | { action: null; reason: DragEndReason };

/** A drag coming over the application's window, or leaving it: `dropped` once a drop on one of its
    surfaces was delivered. */
export type WindowDragEvent = { type: 'enter' } | { type: 'exit'; dropped: boolean };

/** The host's module (OxyOSDrag). `start` tells why a drag ended without a drop (`onCancel`) before
    its promise settles with null; it rejects where the application has no window. */
export interface DragModule {
  getConstants(): { available?: boolean };
  start(
    data: DragData,
    actions: DragAction[],
    preview: number,
    count: number,
    options: { vanish: boolean },
    onCancel: (reason: string) => void,
  ): Promise<string | null>;
}

const ACTIONS: readonly string[] = ['copy', 'move', 'link'];
const END_REASONS: Record<string, DragEndReason> = {
  escape: 'escape',
  'not-accepted': 'notAccepted',
  cancelled: 'cancelled',
};

export function createDrag(native: DragModule | null | undefined, events: Events) {
  const available = Boolean(native?.getConstants()?.available);
  return {
    available,
    /**
     * Starts a drag from the press the application is handling; resolves with how it ended. With
     * `move` of data the application deletes its original; files are moved by the target. Rejects
     * with `needsInput` where there was no press to start it from, `invalid` for a drag that offers
     * nothing or allows no action, `unavailable` where the application has no window. Aborting
     * rejects with `cancelled`; the drag itself is the person's and goes on.
     */
    async start(request: DragRequest, options?: CallOptions): Promise<DragOutcome> {
      if (!native || !available) throw new PlatformError('drag', 'unavailable');
      const actions = [...request.actions];
      const { files, text, url, html, other } = request.data;
      const offers = Boolean(files?.length || text !== undefined || url !== undefined || html !== undefined)
        || Object.keys(other ?? {}).length > 0;
      if (actions.length === 0 || actions.some((action) => !ACTIONS.includes(action)) || !offers) {
        throw new PlatformError('drag', 'invalid');
      }
      let ended: string | undefined;
      const action = await hostCall('drag', () =>
        native
          .start(request.data, actions, request.preview ?? -1, request.count ?? 1, { vanish: Boolean(request.vanish) }, (reason) => {
            ended = reason;
          })
          .catch(() => {
            // The host rejects only where it cannot drag at all (no window).
            throw new PlatformError('drag', 'unavailable');
          }), options);
      if (action !== null && ACTIONS.includes(action)) return { action: action as DragAction };
      if (ended === 'refused') throw new PlatformError('drag', 'needsInput');
      return { action: null, reason: END_REASONS[ended ?? ''] ?? 'cancelled' };
    },
    /** Follows drags over the application's window, from any application; the function returned
        stops following. */
    onWindowDrag(listener: (event: WindowDragEvent) => void): () => void {
      if (!available) return () => {};
      const subscription = events.addListener('OxyOSWindowDrag', (event: { type?: string; dropped?: boolean }) =>
        listener(event.type === 'exit' ? { type: 'exit', dropped: Boolean(event.dropped) } : { type: 'enter' }),
      );
      return () => subscription.remove();
    },
  };
}

export type Drag = ReturnType<typeof createDrag>;
