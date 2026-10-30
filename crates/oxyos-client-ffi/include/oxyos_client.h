/* C ABI of the OxyOS display protocol client (crates/oxyos-client-ffi). The protocol, transport and
 * shared-memory buffers are implemented once, in Rust; this header only declares the calls a
 * native runtime written in C or C++ makes. Keep it in step with src/lib.rs (its tests check the
 * constants and layouts below).
 *
 * A client is used by one thread at a time. Calls returning int give 0 (or a documented positive
 * value) on success and OXYOS_ERROR on failure; oxyos_last_error() then describes the failure. */
#ifndef OXYOS_CLIENT_H
#define OXYOS_CLIENT_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct OxyOSClient OxyOSClient;

/* Event kinds (the protocol's event opcodes). */
#define OXYOS_EVENT_CONFIGURE 2
#define OXYOS_EVENT_POPUP_DONE 3
#define OXYOS_EVENT_POINTER 4
#define OXYOS_EVENT_KEY 5
#define OXYOS_EVENT_FOCUS 6
#define OXYOS_EVENT_TOUCH 7
#define OXYOS_EVENT_FEEDBACK 8
#define OXYOS_EVENT_RELEASED 9
#define OXYOS_EVENT_SCROLL 11
#define OXYOS_EVENT_TEXT 12
#define OXYOS_EVENT_PREEDIT 13
#define OXYOS_EVENT_ACCESSIBILITY_ACTION 14
#define OXYOS_EVENT_CLIPBOARD_TYPES 20
#define OXYOS_EVENT_CLIPBOARD_TYPE 21
#define OXYOS_EVENT_CLIPBOARD_EMPTY 22
#define OXYOS_EVENT_TEXT_INPUT 29
#define OXYOS_EVENT_WINDOW_STATE 37
#define OXYOS_EVENT_WINDOW_TITLE 38
#define OXYOS_EVENT_WINDOW_APP_ID 39
#define OXYOS_EVENT_WINDOW_CLOSED 40
#define OXYOS_EVENT_WORKSPACES 41
#define OXYOS_EVENT_CLOSE_REQUESTED 42
#define OXYOS_EVENT_PINCH 43
#define OXYOS_EVENT_LOCK_STATE 44
#define OXYOS_EVENT_AUTHENTICATION_RESULT 45
#define OXYOS_EVENT_CAPS_LOCK 46
#define OXYOS_EVENT_LAUNCH_RESULT 49
/* What came of a lock surface's password (OxyOSEvent.detail of an AUTHENTICATION_RESULT). */
#define OXYOS_AUTH_ACCEPTED 1
#define OXYOS_AUTH_REJECTED 2
#define OXYOS_AUTH_BUSY 3
#define OXYOS_AUTH_THROTTLED 4
/* A window's state bits (OxyOSEvent.flag of a WINDOW_STATE event). */
#define OXYOS_WINDOW_MINIMIZED 1
#define OXYOS_WINDOW_FOCUSED 2
/* The session started the window's process: its application id is the registry's, not its claim
 * (docs/adr/0013-launching-applications.mdx). */
#define OXYOS_WINDOW_LAUNCHED 4

/* Pointer kinds (OxyOSEvent.detail of a pointer event). */
#define OXYOS_POINTER_ENTER 1
#define OXYOS_POINTER_LEAVE 2
#define OXYOS_POINTER_MOTION 3
#define OXYOS_POINTER_PRESS 4
#define OXYOS_POINTER_RELEASE 5
#define OXYOS_POINTER_CANCEL 6

/* A pinch's phases (OxyOSEvent.detail of a pinch event). */
#define OXYOS_PINCH_BEGIN 1
#define OXYOS_PINCH_UPDATE 2
#define OXYOS_PINCH_END 3
#define OXYOS_PINCH_CANCEL 4

/* Feedback outcomes (OxyOSEvent.detail of a feedback event). */
#define OXYOS_OUTCOME_PRESENTED 1
#define OXYOS_OUTCOME_DISCARDED 2

/* Pixel formats (DRM fourcc codes). ARGB8888 is premultiplied. */
#define OXYOS_FORMAT_XRGB8888 0x34325258
#define OXYOS_FORMAT_ARGB8888 0x34325241

/* oxyos_client_poll results. */
#define OXYOS_POLL_EVENT 1
#define OXYOS_POLL_EMPTY 0
#define OXYOS_POLL_CLOSED (-1)
#define OXYOS_ERROR (-2)

/* One compositor event. Fields by kind:
 *   CONFIGURE   serial; x, y, width, height: the rect (logical; a popup's is relative to its
 *               parent); scale_120: scale * 120; flag: fullscreen.
 *   POPUP_DONE  detail: dismiss reason.
 *   POINTER     serial; x, y: surface-local position in 1/256 logical pixels; detail: pointer
 *               kind; code: button (Linux input code, 272 = left).
 *   KEY         serial; code: key (Linux input code); flag: pressed.
 *   FOCUS       flag: focused.
 *   TOUCH       serial; x, y as POINTER; detail: touch kind; code: touch id.
 *   FEEDBACK    serial: frame sequence; detail: outcome.
 *   RELEASED    serial: buffer id (the client reuses it; informational).
 *   SCROLL      serial; x, y: horizontal and vertical scroll distance in 1/256 logical pixels
 *               (positive moves the content's view right and down).
 *   TEXT        serial; text: committed UTF-8 (whole characters), NUL-terminated.
 *   PREEDIT     serial; text: the composition (empty ends it), NUL-terminated; detail: the
 *               composing cursor, a byte offset into text.
 *   ACCESSIBILITY_ACTION  serial; code: the node's index in the surface's snapshot; detail:
 *               OXYOS_ACTION_*; width: a CUSTOM action's index among the node's; x, y: a
 *               SET_SELECTION's range, in bytes of the node's value. An assistive technology asks
 *               the application to perform it; any action's serial but a focus move's may be
 *               cited by a popup's grab, as a key press's may.
 *   CLIPBOARD_TYPES  serial: the content's selection (for oxyos_client_paste); detail: how many
 *               types follow. Sent to the client with keyboard focus when the clipboard changes
 *               or focus reaches it.
 *   CLIPBOARD_TYPE   serial: the selection; text: one of its media types, NUL-terminated.
 *   CLIPBOARD_EMPTY  the clipboard holds nothing.
 *   TEXT_INPUT  serial: the input's (a user input when it commits text, as a key press);
 *               detail: the field's commit serial it was made for. A transaction for a field with
 *               text input enabled is complete: oxyos_client_text_transaction gives it.
 *   To the session's shell (ADR 0009):
 *   WINDOW_STATE  code: the window's number; detail: its workspace; flag: OXYOS_WINDOW_* bits.
 *   WINDOW_TITLE, WINDOW_APP_ID  code: the window; detail: the chunk's index (0 begins it);
 *               flag: whether it is the last; text: the chunk.
 *   WINDOW_CLOSED  code: the window.
 *   WORKSPACES  detail: how many; code: the active one (from 0).
 *   CLOSE_REQUESTED  the shell asks this application to close the window `surface`.
 *   PINCH       serial; x, y as POINTER: the pointer, which a pinch does not move; detail:
 *               OXYOS_PINCH_*; scale_65536: the fingers' spread over their spread when the
 *               pinch began (65536 = as they started); rotation_256: how far they have turned
 *               since, in 1/256 degrees, clockwise. A pinch stays with the surface under the
 *               pointer when it began until it ends or is cancelled.
 *   CAPS_LOCK   flag: whether Caps Lock is on; to the surface with keyboard focus when it changes,
 *               and after its FOCUS when it is on (a surface gains focus with it off unless told).
 *   To a client with the `lock` grant (ADR 0010):
 *   LOCK_STATE  flag: whether the session is locked. Locked, the client makes a lock surface per
 *               output; unlocked, it destroys them.
 *   AUTHENTICATION_RESULT  detail: OXYOS_AUTH_*, what came of the password `surface` submitted;
 *               code: the seconds until another may be tried (0: at once). Only the session
 *               unlocks, and LOCK_STATE then says so. */
typedef struct OxyOSEvent {
  uint32_t kind;
  uint32_t surface;
  uint64_t serial;
  int32_t x;
  int32_t y;
  uint32_t width;
  uint32_t height;
  uint32_t scale_120;
  uint32_t detail;
  uint32_t code;
  uint32_t scale_65536;
  int32_t rotation_256;
  uint8_t flag;
  char text[33];
} OxyOSEvent;


/* Accessibility (ADR 0003). Roles of an accessible node. */
#define OXYOS_ROLE_GENERIC 0
#define OXYOS_ROLE_BUTTON 1
#define OXYOS_ROLE_LINK 2
#define OXYOS_ROLE_TEXT 3
#define OXYOS_ROLE_HEADING 4
#define OXYOS_ROLE_IMAGE 5
#define OXYOS_ROLE_TEXT_INPUT 6
#define OXYOS_ROLE_MENU 7
#define OXYOS_ROLE_MENU_ITEM 8
#define OXYOS_ROLE_CHECK_BOX 9
#define OXYOS_ROLE_RADIO 10
#define OXYOS_ROLE_SWITCH 11
#define OXYOS_ROLE_TAB 12
#define OXYOS_ROLE_TAB_LIST 13
#define OXYOS_ROLE_LIST 14
#define OXYOS_ROLE_LIST_ITEM 15
#define OXYOS_ROLE_DIALOG 16
#define OXYOS_ROLE_ALERT 17
#define OXYOS_ROLE_PROGRESS_BAR 18
#define OXYOS_ROLE_SLIDER 19
#define OXYOS_ROLE_SEARCH 20
#define OXYOS_ROLE_TOOLBAR 21
#define OXYOS_ROLE_SCROLL_VIEW 22
#define OXYOS_ROLE_GROUP 23
#define OXYOS_ROLE_COMBO_BOX 24
#define OXYOS_ROLE_SUMMARY 25
#define OXYOS_ROLE_GRID 26
/* States of an accessible node (bits). */
#define OXYOS_STATE_DISABLED 1
#define OXYOS_STATE_SELECTED 2
#define OXYOS_STATE_CHECKED 4
#define OXYOS_STATE_MIXED 8
#define OXYOS_STATE_EXPANDED 16
#define OXYOS_STATE_COLLAPSED 32
#define OXYOS_STATE_BUSY 64
#define OXYOS_STATE_FOCUSED 128
/* A live region: what changes in it is announced, after what is being said or at once. */
#define OXYOS_STATE_LIVE_POLITE 256
#define OXYOS_STATE_LIVE_ASSERTIVE 512
/* Actions an assistive technology asks for (OxyOSEvent.detail of an ACCESSIBILITY_ACTION). */
#define OXYOS_ACTION_ACTIVATE 1
#define OXYOS_ACTION_FOCUS 2
#define OXYOS_ACTION_INCREMENT 3
#define OXYOS_ACTION_DECREMENT 4
#define OXYOS_ACTION_SCROLL_FORWARD 5
#define OXYOS_ACTION_SCROLL_BACKWARD 6
#define OXYOS_ACTION_DISMISS 7
#define OXYOS_ACTION_CUSTOM 8
#define OXYOS_ACTION_SET_SELECTION 9
/* What a node offers beyond activate and focus (OxyOSAccessibleNode.actions, bits). */
#define OXYOS_OFFERS_INCREMENT 1
#define OXYOS_OFFERS_DECREMENT 2
#define OXYOS_OFFERS_SCROLL_FORWARD 4
#define OXYOS_OFFERS_SCROLL_BACKWARD 8
#define OXYOS_OFFERS_DISMISS 16
#define OXYOS_OFFERS_SET_SELECTION 32
/* How an announcement interrupts (oxyos_client_announce). */
#define OXYOS_POLITE 1
#define OXYOS_ASSERTIVE 2
/* The parent of a root node. */
#define OXYOS_NO_ACCESSIBLE_PARENT 0xFFFFFFFF

/* A popup's placement: anchor rectangle in parent-local logical coordinates, the popup's logical
 * size, anchor and gravity (0 center, 1 top, 2 bottom, 3 left, 4 right, 5 top-left,
 * 6 bottom-left, 7 top-right, 8 bottom-right), adjustment flags (1 slide x, 2 slide y, 4 flip x,
 * 8 flip y, 16 resize x, 32 resize y, 64 right-to-left) and an offset. */
typedef struct OxyOSPositioner {
  int32_t anchor_x;
  int32_t anchor_y;
  uint32_t anchor_width;
  uint32_t anchor_height;
  uint32_t width;
  uint32_t height;
  uint8_t anchor;
  uint8_t gravity;
  uint8_t adjust;
  int32_t offset_x;
  int32_t offset_y;
} OxyOSPositioner;

/* One node of a surface's accessibility snapshot: its parent's index (OXYOS_NO_ACCESSIBLE_PARENT
 * for a root), role (OXYOS_ROLE_*), states (OXYOS_STATE_* bits), surface-local logical rectangle,
 * and name and value (UTF-8, NUL-terminated; NULL for none); what it offers (OXYOS_OFFERS_* bits)
 * and its custom actions' labels, one per line (NULL for none); a text field's selection, in bytes
 * of its value, when `has_selection`. A secure field's value and selection are never published. */
typedef struct OxyOSAccessibleNode {
  uint32_t parent;
  uint8_t role;
  uint16_t state;
  int32_t x;
  int32_t y;
  uint32_t width;
  uint32_t height;
  const char* name;
  const char* value;
  uint16_t actions;
  const char* custom_actions;
  uint8_t has_selection;
  uint32_t selection_start;
  uint32_t selection_end;
} OxyOSAccessibleNode;

const char* oxyos_last_error(void);

OxyOSClient* oxyos_client_connect(const char* path);
void oxyos_client_close(OxyOSClient* client);
int oxyos_client_fd(OxyOSClient* client);

int oxyos_client_create_toplevel(OxyOSClient* client, uint32_t surface, uint32_t width, uint32_t height);

/* The session's shell (the `shell` grant). Shell surface kinds and panel edges. */
#define OXYOS_SHELL_BACKGROUND 1
#define OXYOS_SHELL_PANEL 2
#define OXYOS_SHELL_OVERLAY 3
#define OXYOS_SHELL_NOTIFICATION 4
#define OXYOS_EDGE_NONE 0
#define OXYOS_EDGE_TOP 1
#define OXYOS_EDGE_BOTTOM 2
#define OXYOS_EDGE_LEFT 3
#define OXYOS_EDGE_RIGHT 4
/* Requests a shell surface of `kind` at `edge` of `output`, of the given logical size (a panel's
 * thickness along its edge; a background takes the whole output), keeping `exclusive` logical
 * pixels of a panel's edge out of where windows are placed. The compositor answers with a
 * configure. */
int oxyos_client_create_shell_surface(OxyOSClient* client, uint32_t surface, uint8_t kind, uint8_t edge,
                                      uint32_t output, uint32_t width, uint32_t height, uint32_t exclusive);
/* Names a window: its title, or with a non-zero `app_id` its application id (`length` bytes of
 * UTF-8; at most 256 for a title, 128 for an id). */
int oxyos_client_name_window(OxyOSClient* client, uint32_t surface, uint8_t app_id, const char* text, size_t length);
/* What the session's shell (the `shell` grant) asks of windows and workspaces. */
#define OXYOS_MANAGE_FOCUS 1
#define OXYOS_MANAGE_CLOSE 2
#define OXYOS_MANAGE_MINIMIZE 3
#define OXYOS_MANAGE_MOVE 4
#define OXYOS_MANAGE_ACTIVATE_WORKSPACE 5
#define OXYOS_MANAGE_ADD_WORKSPACE 6
#define OXYOS_MANAGE_REMOVE_WORKSPACE 7
/* The lock (ADR 0010). The lock surface of `output` (the `lock` grant): it covers the output and is
 * all that shows while locked; a configure answers. Locking the session (the `shell` grant). The
 * password typed in a lock surface (`length` bytes of UTF-8, at most 512), submitted for the
 * session to check: the library keeps no copy, and the caller overwrites its own once this
 * returns. */
int oxyos_client_create_lock_surface(OxyOSClient* client, uint32_t surface, uint32_t output);
int oxyos_client_lock_session(OxyOSClient* client);
/* The session's shell asks for the application `id` (1 to 128 bytes of UTF-8) of the session's
 * registry (docs/adr/0013-launching-applications.mdx). A LAUNCH_RESULT answers: code, `serial`;
 * flag, whether it was started. */
int oxyos_client_launch(OxyOSClient* client, uint32_t serial, const char* id, size_t length);
int oxyos_client_authenticate(OxyOSClient* client, uint32_t surface, const char* secret, size_t length);
/* Focuses (and restores, showing its workspace), closes (the application is asked), minimizes or
 * moves (to `workspace`) the window numbered `window`; activates, adds or removes a workspace. */
int oxyos_client_manage(OxyOSClient* client, uint8_t action, uint32_t window, uint32_t workspace);

/* Limits where a surface takes pointer and touch input to a surface-local logical rectangle (an
 * empty one: nowhere); input elsewhere reaches what is below. */
/* Pointer shapes (the protocol's): the pointer's shape while it is over a surface. */
#define OXYOS_POINTER_ARROW 1
#define OXYOS_POINTER_TEXT 2
#define OXYOS_POINTER_HAND 3
#define OXYOS_POINTER_NOT_ALLOWED 4
#define OXYOS_POINTER_WAIT 5
#define OXYOS_POINTER_HELP 6
#define OXYOS_POINTER_CROSSHAIR 7
#define OXYOS_POINTER_MOVE 8
#define OXYOS_POINTER_GRAB 9
#define OXYOS_POINTER_GRABBING 10
#define OXYOS_POINTER_RESIZE_HORIZONTAL 11
#define OXYOS_POINTER_RESIZE_VERTICAL 12
#define OXYOS_POINTER_RESIZE_TOP_LEFT 13
#define OXYOS_POINTER_RESIZE_TOP_RIGHT 14
#define OXYOS_POINTER_ZOOM_IN 15
#define OXYOS_POINTER_ZOOM_OUT 16
/* The pointer's shape while it is over `surface`: one of OXYOS_POINTER_*. 0, or OXYOS_ERROR. */
int oxyos_client_set_pointer_shape(OxyOSClient* client, uint32_t surface, uint8_t shape);
int oxyos_client_set_input_region(OxyOSClient* client, uint32_t surface, int32_t x, int32_t y, uint32_t width,
                                  uint32_t height);
int oxyos_client_create_popup(OxyOSClient* client, uint32_t surface, uint32_t parent,
                            const OxyOSPositioner* positioner);
int oxyos_client_grab_popup(OxyOSClient* client, uint32_t surface, uint64_t serial);
/* Places an existing popup again (its anchor moved or its content resized). */
int oxyos_client_reposition(OxyOSClient* client, uint32_t surface, const OxyOSPositioner* positioner);
/* Publishes a surface's accessibility snapshot, replacing the previous one: `count` nodes in
 * reading order, each parent before its children. Names and values are UTF-8 (NULL for none);
 * text longer than the protocol allows is cut at a character boundary. 0 on success. */
int oxyos_client_publish_accessibility(OxyOSClient* client, uint32_t surface, const OxyOSAccessibleNode* nodes,
                                     size_t count);
/* Announces `length` bytes of UTF-8 to assistive technologies, as Android's
 * announceForAccessibility and live regions: OXYOS_POLITE after what they are saying,
 * OXYOS_ASSERTIVE at once. Longer text than the protocol takes (1024 bytes) is cut at a character
 * boundary. 0 on success. */
int oxyos_client_announce(OxyOSClient* client, uint32_t surface, uint8_t politeness, const char* text,
                          size_t length);
int oxyos_client_destroy_surface(OxyOSClient* client, uint32_t surface);

/* Presents pixels (rows of width * 4 bytes) on a surface in answer to the configure with serial
 * `configure`: OXYOS_FORMAT_XRGB8888 is opaque (little-endian B, G, R, X), OXYOS_FORMAT_ARGB8888 is
 * premultiplied (B, G, R, A). Returns the frame's sequence number, or -1. */
int64_t oxyos_client_present(OxyOSClient* client, uint32_t surface, uint64_t configure, uint32_t width,
                           uint32_t height, uint32_t format, const uint8_t* pixels, size_t length);

/* A rectangle of a buffer, in its pixels. */
typedef struct OxyOSRect {
  int32_t x;
  int32_t y;
  uint32_t width;
  uint32_t height;
} OxyOSRect;

/* oxyos_client_present for a frame of which only `count` rectangles at `damage` changed since the
 * surface's previous frame (0: everything). The compositor hears the damage, and a reused buffer
 * receives only what changed since it was last written. Returns the sequence number, or -1. */
int64_t oxyos_client_present_damaged(OxyOSClient* client, uint32_t surface, uint64_t configure, uint32_t width,
                                   uint32_t height, uint32_t format, const uint8_t* pixels, size_t length,
                                   const OxyOSRect* damage, size_t count);

/* Takes one queued event without blocking. */
int oxyos_client_poll(OxyOSClient* client, OxyOSEvent* out);

/* The clipboard (docs/adr/0007-clipboard.mdx). Copies `length` bytes of UTF-8 text as the
 * clipboard (text/plain;charset=utf-8) with the serial of the user input that asked for it; the
 * client library keeps it and answers pastes itself. The compositor refuses a copy whose serial
 * is not a recent input of this client, or made without keyboard focus. */
int oxyos_client_copy_text(OxyOSClient* client, uint64_t serial, const char* text, size_t length);
/* Empties the clipboard if this client's copy is what it holds. */
int oxyos_client_clear_clipboard(OxyOSClient* client, uint64_t serial);
/* Pastes `mime` of the content CLIPBOARD_TYPES named `selection`, with the serial of the user input
 * that asked for it: returns the read end of a pipe the content arrives through (read it to its
 * end and close it; it ends at once, empty, if the paste was refused), or OXYOS_ERROR. */
int oxyos_client_paste(OxyOSClient* client, uint32_t selection, const char* mime, size_t mime_length,
                       uint64_t serial);

/* Text input (docs/adr/0008-input-method.mdx). What a field is for. */
#define OXYOS_PURPOSE_NORMAL 0
#define OXYOS_PURPOSE_DIGITS 1
#define OXYOS_PURPOSE_NUMBER 2
#define OXYOS_PURPOSE_PHONE 3
#define OXYOS_PURPOSE_URL 4
#define OXYOS_PURPOSE_EMAIL 5
#define OXYOS_PURPOSE_NAME 6
#define OXYOS_PURPOSE_PASSWORD 7
#define OXYOS_PURPOSE_PIN 8
#define OXYOS_PURPOSE_DATE 9
#define OXYOS_PURPOSE_TIME 10
/* Its hints (bits). */
#define OXYOS_HINT_NO_PREDICTION 1
#define OXYOS_HINT_NO_CAPITALIZATION 2
#define OXYOS_HINT_SENSITIVE 4
#define OXYOS_HINT_MULTILINE 8
/* How a segment of a composition is drawn (OxyOSPreeditSegment.style). */
#define OXYOS_PREEDIT_COMPOSING 1
#define OXYOS_PREEDIT_CONVERTING 2
#define OXYOS_PREEDIT_ERROR 3

/* A focused text field: purpose, hints, its text (`length` bytes of UTF-8, the composition left
 * out) with the selection's byte offsets in it, and its caret's rectangle in surface-local logical
 * coordinates when `has_caret`. */
typedef struct OxyOSTextField {
  uint8_t purpose;
  uint32_t hints;
  const char* text;
  size_t length;
  uint32_t anchor;
  uint32_t cursor;
  uint8_t has_caret;
  int32_t caret_x;
  int32_t caret_y;
  uint32_t caret_width;
  uint32_t caret_height;
} OxyOSTextField;

/* Tells the compositor about the field focused in `surface`, or that none is (`field` NULL); only
 * what changed is sent. A non-zero `new_field` describes another field than the one told before.
 * Returns the commit serial transactions for this state carry (0 once disabled), or -1. */
int64_t oxyos_client_text_input(OxyOSClient* client, uint32_t surface, const OxyOSTextField* field,
                                uint8_t new_field);

/* One styled segment of a composition, in bytes of it. */
typedef struct OxyOSPreeditSegment {
  uint32_t begin;
  uint32_t end;
  uint8_t style;
} OxyOSPreeditSegment;

/* One change for a field, applied whole: delete `delete_before` bytes before the selection and
 * `delete_after` after it, insert `commit` at it, then show `preedit` as the composition (none if
 * empty) with [cursor_begin, cursor_end) as the selection within it, and its segments. `serial`
 * and `commit_serial` are the TEXT_INPUT event's. Strings are not NUL-terminated; they and the
 * segments stay valid until the next oxyos_client_poll. */
typedef struct OxyOSTextTransaction {
  uint32_t surface;
  uint32_t delete_before;
  uint32_t delete_after;
  uint32_t cursor_begin;
  uint32_t cursor_end;
  uint32_t commit_serial;
  uint64_t serial;
  const char* commit;
  size_t commit_length;
  const char* preedit;
  size_t preedit_length;
  const OxyOSPreeditSegment* segments;
  size_t segment_count;
} OxyOSTextTransaction;

/* The transaction the TEXT_INPUT event just returned completed: 1 with `*out` filled, or 0. */
int oxyos_client_text_transaction(OxyOSClient* client, OxyOSTextTransaction* out);

/* HTTP (crates/oxyos-net): the engine behind React Native's networking. Each request runs on a
 * thread of its own (TLS through rustls against the system's certificates, redirects, gzip and
 * brotli, one cookie jar for the network); the caller takes its answers from its own thread.
 * Strings are a pointer and a length, never NUL-terminated. A network is used by one thread at a
 * time. */
typedef struct OxyOSNet OxyOSNet;

/* Answer kinds (OxyOSNetAnswer.kind), in the order a request answers: a response, then chunks
 * (incremental requests) or the data, then its completion. A failure may complete it at any
 * point. */
#define OXYOS_NET_RESPONSE 1
#define OXYOS_NET_CHUNK 2
#define OXYOS_NET_DATA 3
#define OXYOS_NET_COMPLETE 4

/* Results of oxyos_net_answer. */
#define OXYOS_NET_ANSWER 1
#define OXYOS_NET_EMPTY 0

typedef struct OxyOSNetHeader {
  const char* name;
  size_t name_length;
  const char* value;
  size_t value_length;
} OxyOSNetHeader;

/* timeout_ms is the whole request's (0 for none); has_body says whether body is sent (a request
 * may send an empty body, or none); with_credentials, whether the jar's cookies go with it and
 * what it sets is kept; incremental, whether the body comes as chunks instead of whole. */
typedef struct OxyOSNetRequest {
  int64_t id;
  const char* method;
  size_t method_length;
  const char* url;
  size_t url_length;
  const OxyOSNetHeader* headers;
  size_t header_count;
  const uint8_t* body;
  size_t body_length;
  uint64_t timeout_ms;
  uint8_t has_body;
  uint8_t with_credentials;
  uint8_t incremental;
} OxyOSNetRequest;

/* RESPONSE: status, headers (duplicates joined with ", ") and the URL answered after redirects.
 * CHUNK: bytes/length, progress (bytes so far) and total (-1 if unknown).
 * DATA: bytes/length, and content_type (NULL if none).
 * COMPLETE: error (NULL if it succeeded) and timed_out.
 * Its pointers stay valid until the next oxyos_net_answer or oxyos_net_free. */
typedef struct OxyOSNetAnswer {
  uint32_t kind;
  uint32_t status;
  int64_t id;
  const OxyOSNetHeader* headers;
  size_t header_count;
  const char* url;
  size_t url_length;
  const uint8_t* bytes;
  size_t length;
  uint64_t progress;
  int64_t total;
  const char* content_type;
  size_t content_type_length;
  const char* error;
  size_t error_length;
  uint8_t timed_out;
} OxyOSNetAnswer;

/* A network with an empty cookie jar, or NULL. */
OxyOSNet* oxyos_net_new(void);
/* Frees the network; requests still running end unheard. NULL is ignored. */
void oxyos_net_free(OxyOSNet* net);
/* Readable while answers are queued (for poll/epoll). */
int oxyos_net_fd(OxyOSNet* net);
/* Starts a request; its answers follow, ending with a completion. On failure (a malformed
 * request) nothing is sent and no answer follows. */
int oxyos_net_send(OxyOSNet* net, const OxyOSNetRequest* request);
/* Delivers nothing more of request `id`, not even its completion. */
int oxyos_net_abort(OxyOSNet* net, int64_t id);
/* Forgets every cookie. */
int oxyos_net_clear_cookies(OxyOSNet* net);
/* Takes one answer without blocking: OXYOS_NET_ANSWER with *out filled, or OXYOS_NET_EMPTY. */
int oxyos_net_answer(OxyOSNet* net, OxyOSNetAnswer* out);

/* Images (crates/oxyos-image): PNG and JPEG decoded into sRGB (from an embedded ICC profile),
 * premultiplied RGBA, reduced to the size they are drawn at, each load on a thread of its own.
 * Paths are a pointer and a length. A loader is used by one thread at a time. */
typedef struct OxyOSImages OxyOSImages;

/* How oxyos_images_load fits an image to its box: within it (resizeMode contain), or covering it
 * (cover, stretch). */
#define OXYOS_IMAGE_FIT_WITHIN 0
#define OXYOS_IMAGE_FIT_COVERING 1

/* Results of oxyos_images_answer. */
#define OXYOS_IMAGE_ANSWER 1
#define OXYOS_IMAGE_EMPTY 0

/* `width` x `height` premultiplied RGBA pixels, reduced from the image's natural size; `pixels` is
 * NULL when the load failed, `error` NULL when it did not. Its pointers stay valid until the next
 * oxyos_images_answer or oxyos_images_free. */
typedef struct OxyOSImageAnswer {
  uint64_t id;
  uint32_t width;
  uint32_t height;
  uint32_t natural_width;
  uint32_t natural_height;
  const uint8_t* pixels;
  size_t length;
  const char* error;
  size_t error_length;
} OxyOSImageAnswer;

/* The size in pixels of the image at `path`, from its headers alone. */
int oxyos_image_size(const char* path, size_t path_length, uint32_t* width, uint32_t* height);
/* A loader, or NULL. */
OxyOSImages* oxyos_images_new(void);
/* Frees the loader; loads still running end unheard. NULL is ignored. */
void oxyos_images_free(OxyOSImages* images);
/* Readable while answers are queued (for poll/epoll). */
int oxyos_images_fd(OxyOSImages* images);
/* Starts decoding the image at `path`, reduced as far as `fit` allows for a box of width x height
 * pixels (0 for either keeps its size), never enlarged; its answer, with `id`, follows. */
int oxyos_images_load(OxyOSImages* images, uint64_t id, const char* path, size_t path_length, uint32_t width,
                    uint32_t height, uint32_t fit);
/* Takes one answer without blocking: OXYOS_IMAGE_ANSWER with *out filled, or OXYOS_IMAGE_EMPTY. */
int oxyos_images_answer(OxyOSImages* images, OxyOSImageAnswer* out);

/* Vector graphics (crates/oxyos-vector): what react-native-svg draws. Shape kinds; caps, joins and
 * fill rules are react-native-svg's own codes. */
#define OXYOS_SHAPE_PATH 0
#define OXYOS_SHAPE_RECT 1
#define OXYOS_SHAPE_ELLIPSE 2
#define OXYOS_SHAPE_LINE 3
#define OXYOS_CAP_BUTT 0
#define OXYOS_CAP_ROUND 1
#define OXYOS_CAP_SQUARE 2
#define OXYOS_JOIN_MITER 0
#define OXYOS_JOIN_ROUND 1
#define OXYOS_JOIN_BEVEL 2

/* One shape: path data (`data`, `data_length` bytes of UTF-8) for OXYOS_SHAPE_PATH, otherwise
 * `geometry` holds the rectangle's x, y, width, height, rx, ry, the ellipse's cx, cy, rx, ry or
 * the line's x1, y1, x2, y2; SVG's matrix (a, b, c, d, e, f) to the picture's pixels; its fill
 * and stroke (0xAARRGGBB, not premultiplied; zero `fill` or `stroke` for none). */
typedef struct OxyOSVectorShape {
  uint8_t kind;
  const char* data;
  size_t data_length;
  float geometry[6];
  float transform[6];
  uint8_t fill;
  uint8_t even_odd;
  uint32_t fill_color;
  uint8_t stroke;
  uint8_t cap;
  uint8_t join;
  uint32_t stroke_color;
  float stroke_width;
  float miter_limit;
  const float* dash;
  size_t dash_count;
  float dash_offset;
} OxyOSVectorShape;

/* Draws `count` shapes in order onto a transparent width × height picture written to `pixels`
 * (`length` bytes, at least width × height × 4) as premultiplied R, G, B, A bytes, rows top to
 * bottom, as decoded images are. Returns how many shapes were left out (path data SVG would
 * reject), or OXYOS_ERROR. */
int oxyos_vector_render(const OxyOSVectorShape* shapes, size_t count, uint32_t width, uint32_t height,
                        uint8_t* pixels, size_t length);

/* Gesture recognition (crates/oxyos-gestures): react-native-gesture-handler's Android
 * recognizers, for the host's RNGestureHandlerModule. Pointer actions are Android's. */
#define OXYOS_GESTURE_DOWN 0
#define OXYOS_GESTURE_POINTER_DOWN 1
#define OXYOS_GESTURE_MOVE 2
#define OXYOS_GESTURE_POINTER_UP 3
#define OXYOS_GESTURE_UP 4
#define OXYOS_GESTURE_CANCEL 5
/* Setting values' kinds. */
#define OXYOS_GESTURE_NULL 0
#define OXYOS_GESTURE_BOOL 1
#define OXYOS_GESTURE_NUMBER 2
#define OXYOS_GESTURE_TEXT 3
#define OXYOS_GESTURE_LIST 4
/* Outputs' kinds, and which data an output has. */
#define OXYOS_GESTURE_STATE_CHANGE 1
#define OXYOS_GESTURE_UPDATE 2
#define OXYOS_GESTURE_INTERCEPT 3
#define OXYOS_GESTURE_CANCEL_RESPONDER 4
#define OXYOS_GESTURE_TOUCH 5
/* Touch events' types (onTouchesDown and the rest). */
#define OXYOS_GESTURE_TOUCH_DOWN 1
#define OXYOS_GESTURE_TOUCH_MOVE 2
#define OXYOS_GESTURE_TOUCH_UP 3
#define OXYOS_GESTURE_TOUCH_CANCELLED 4
#define OXYOS_GESTURE_HAS_POSITION 1
#define OXYOS_GESTURE_HAS_TRANSLATION 2
#define OXYOS_GESTURE_HAS_VELOCITY 4
#define OXYOS_GESTURE_HAS_DURATION 8
#define OXYOS_GESTURE_HAS_POINTER_INSIDE 16
/* A pinch's scale, focal_x, focal_y and rate (its velocity, scale per second). */
#define OXYOS_GESTURE_HAS_SCALE 32
/* A rotation's rotation (radians, clockwise), anchor_x, anchor_y and rate (its velocity, radians
 * per millisecond, as Android's). */
#define OXYOS_GESTURE_HAS_ROTATION 64

typedef struct OxyOSGestures OxyOSGestures;

/* A setting: its key (`key_length` bytes of UTF-8) and value. A bool's or a number's is `number`;
 * a list's (a hit slop's: left, top, right, bottom, width, height) is `list`, entry i set when bit
 * i of `list_set` is; a text's content is not needed. */
typedef struct OxyOSGestureValue {
  const char* key;
  size_t key_length;
  uint8_t kind;
  uint8_t list_set;
  double number;
  double list[6];
} OxyOSGestureValue;

/* A pointer at a point of the root, in logical pixels. */
typedef struct OxyOSGesturePointer {
  uint32_t id;
  float x;
  float y;
} OxyOSGesturePointer;

/* A view's frame in the root. */
typedef struct OxyOSGestureFrame {
  float x;
  float y;
  float width;
  float height;
} OxyOSGestureFrame;

/* Writes view `view`'s frame to `out` and returns 1, or returns 0 for a view no longer there. */
typedef uint8_t (*OxyOSGestureFrameFn)(void* context, int32_t view, OxyOSGestureFrame* out);

/* A pointer a touch event tells of: in the handler's view, and in the root. */
typedef struct OxyOSGestureTouch {
  uint32_t id;
  float x;
  float y;
  float absolute_x;
  float absolute_y;
} OxyOSGestureTouch;

/* What to send JavaScript. A state change has `state` and `old_state`, an update `state` (ACTIVE);
 * both have the handler's data, `fields` saying which. A touch event has `state`, `event_type`,
 * `pointer_type` and its number of touches in `number_of_pointers`; its pointers come from
 * oxyos_gestures_touches. A cancel-responder output has `tag`; an intercept output nothing more. */
typedef struct OxyOSGestureOutput {
  uint8_t kind;
  uint8_t state;
  uint8_t old_state;
  uint8_t pointer_type;
  uint8_t fields;
  uint8_t pointer_inside;
  uint8_t event_type;
  int32_t tag;
  int32_t view;
  uint32_t number_of_pointers;
  float x;
  float y;
  float absolute_x;
  float absolute_y;
  float translation_x;
  float translation_y;
  float velocity_x;
  float velocity_y;
  uint64_t duration;
  float scale;
  float focal_x;
  float focal_y;
  float rotation;
  float anchor_x;
  float anchor_y;
  float rate;
} OxyOSGestureOutput;

OxyOSGestures* oxyos_gestures_new(void);
void oxyos_gestures_free(OxyOSGestures* gestures);
/* createGestureHandler: handler `name` (`name_length` bytes) with tag `tag`, set to `count`
 * settings; each entry of `refused` (`count` bytes, or NULL) is set to 1 for a setting OxyOS
 * cannot honour. 0, or OXYOS_ERROR (a handler not ported, a tag in use). */
int oxyos_gestures_create(OxyOSGestures* gestures, const char* name, size_t name_length, int32_t tag,
                          const OxyOSGestureValue* config, size_t count, uint8_t* refused);
/* setGestureHandlerConfig (`reset` 1) or updateGestureHandlerConfig (0). */
int oxyos_gestures_configure(OxyOSGestures* gestures, int32_t tag, const OxyOSGestureValue* config, size_t count,
                             uint8_t reset, uint8_t* refused);
int oxyos_gestures_attach(OxyOSGestures* gestures, int32_t tag, int32_t view);
int oxyos_gestures_relate(OxyOSGestures* gestures, int32_t tag, const int32_t* wait_for, size_t wait_for_count,
                          const int32_t* simultaneous, size_t simultaneous_count, const int32_t* blocks,
                          size_t blocks_count);
void oxyos_gestures_drop(OxyOSGestures* gestures, int32_t tag);
/* JavaScript sets a handler's state (_setGestureStateSync, _setGestureStateAsync). */
int oxyos_gestures_set_state(OxyOSGestures* gestures, int32_t tag, int32_t state);
/* One event of the pointer stream: `action` about pointer `index` of the `count` down, at `time`
 * milliseconds, of `pointer_type` (0 touch, 1 stylus, 2 mouse). For a pointer going down, `hits`
 * are the views under it, deepest first, then that view's ancestors to the root. */
int oxyos_gestures_touch(OxyOSGestures* gestures, uint8_t action, size_t index, const OxyOSGesturePointer* pointers,
                         size_t count, uint64_t time, uint8_t pointer_type, const int32_t* hits, size_t hit_count,
                         OxyOSGestureFrameFn frame, void* context);
/* Time passed: handlers' timers due by `now` fire. */
void oxyos_gestures_tick(OxyOSGestures* gestures, uint64_t now);
/* When the next timer fires, or -1 for none. */
int64_t oxyos_gestures_next_deadline(OxyOSGestures* gestures);
/* 1 if a handler of `view` (at `frame`) reaches the root point (x, y) with its hit slop. */
uint8_t oxyos_gestures_reaches(OxyOSGestures* gestures, int32_t view, OxyOSGestureFrame frame, float x, float y);
/* Takes the next output: 1 with *out filled, 0 when there is none. */
int oxyos_gestures_output(OxyOSGestures* gestures, OxyOSGestureOutput* out);
/* The pointers of the last touch event taken: those that changed, or (`all` 1) all the handler is
 * told of. Writes up to `capacity` to `out`; returns how many there are. */
size_t oxyos_gestures_touches(OxyOSGestures* gestures, uint8_t all, OxyOSGestureTouch* out, size_t capacity);

/* The session's applications (crates/oxyos-apps, docs/adr/0013-launching-applications.mdx), as JSON:
 * [{"id", "name", "icon" (a file, or null), "pinned" (its place in the dock from 1, or null)}], sorted
 * by name, in the session's language.
 * Written into `out` (`capacity` bytes, NUL-terminated when it fits); returns its length (larger than
 * `capacity`: call again with more room). */
int64_t oxyos_applications(char* out, size_t capacity);

/* The solar wallpaper (crates/oxyos-solar, docs/adr/0012-solar-wallpaper.mdx). A frame of a solar
 * wallpaper and the sun it shows (its manifest's solarAnchors): degrees above the horizon and
 * clockwise from north. */
typedef struct OxyOSSolarAnchor {
  uint32_t frame;
  double elevation;
  double azimuth;
} OxyOSSolarAnchor;
/* The frame of `anchors` showing the sun at `unix_seconds` where the system's time zone is, and when
 * (unix seconds) it next changes. 0 on success; 1 where the time zone has no place (UTC): there is no
 * sun, and the caller shows its own frame; OXYOS_ERROR for anchors that are none. */
int oxyos_solar_frame(const OxyOSSolarAnchor* anchors, size_t count, int64_t unix_seconds, uint32_t* frame,
                      int64_t* next_change);

/* Intl (crates/oxyos-intl, on ICU4X). Locales are BCP 47 tags; options are ECMA-402 options as
 * "name=value" pairs joined by ';' (only those the application gave; booleans as true/false).
 * Results are UTF-8 written into `out` (`capacity` bytes, NUL-terminated when it fits). Each returns
 * the result's length in bytes (larger than `capacity`: call again with more room), or -1 with
 * oxyos_last_error() giving a RangeError's message (an invalid tag or value, or an option OxyOS
 * does not support yet). Parts are "type\x1Fvalue" joined by '\x1E'. */
int64_t oxyos_intl_default_locale(char* out, size_t capacity);
int64_t oxyos_intl_canonicalize(const char* tag, size_t length, char* out, size_t capacity);
/* `requested`: tags joined by ','. */
int64_t oxyos_intl_resolve(const char* requested, size_t length, char* out, size_t capacity);
int64_t oxyos_intl_format_date(const char* locale, size_t locale_length, const char* options,
                               size_t options_length, double epoch_ms, char* out, size_t capacity);
/* resolvedOptions() as "name=value" pairs joined by ';'. */
int64_t oxyos_intl_resolve_date(const char* locale, size_t locale_length, const char* options,
                                size_t options_length, char* out, size_t capacity);
int64_t oxyos_intl_format_number(const char* locale, size_t locale_length, const char* options,
                                 size_t options_length, double value, char* out, size_t capacity);
int64_t oxyos_intl_resolve_number(const char* locale, size_t locale_length, const char* options,
                                  size_t options_length, char* out, size_t capacity);

#ifdef __cplusplus
}
#endif

#endif /* OXYOS_CLIENT_H */
