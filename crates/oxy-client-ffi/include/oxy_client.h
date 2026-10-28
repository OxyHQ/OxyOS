/* C ABI of the Oxy display protocol client (crates/oxy-client-ffi). The protocol, transport and
 * shared-memory buffers are implemented once, in Rust; this header only declares the calls a
 * native runtime written in C or C++ makes. Keep it in step with src/lib.rs (its tests check the
 * constants and layouts below).
 *
 * A client is used by one thread at a time. Calls returning int give 0 (or a documented positive
 * value) on success and OXY_ERROR on failure; oxy_last_error() then describes the failure. */
#ifndef OXY_CLIENT_H
#define OXY_CLIENT_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct OxyClient OxyClient;

/* Event kinds (the protocol's event opcodes). */
#define OXY_EVENT_CONFIGURE 2
#define OXY_EVENT_POPUP_DONE 3
#define OXY_EVENT_POINTER 4
#define OXY_EVENT_KEY 5
#define OXY_EVENT_FOCUS 6
#define OXY_EVENT_TOUCH 7
#define OXY_EVENT_FEEDBACK 8
#define OXY_EVENT_RELEASED 9
#define OXY_EVENT_SCROLL 11
#define OXY_EVENT_TEXT 12
#define OXY_EVENT_PREEDIT 13
#define OXY_EVENT_ACCESSIBILITY_ACTION 14

/* Pointer kinds (OxyEvent.detail of a pointer event). */
#define OXY_POINTER_ENTER 1
#define OXY_POINTER_LEAVE 2
#define OXY_POINTER_MOTION 3
#define OXY_POINTER_PRESS 4
#define OXY_POINTER_RELEASE 5
#define OXY_POINTER_CANCEL 6

/* Feedback outcomes (OxyEvent.detail of a feedback event). */
#define OXY_OUTCOME_PRESENTED 1
#define OXY_OUTCOME_DISCARDED 2

/* Pixel formats (DRM fourcc codes). ARGB8888 is premultiplied. */
#define OXY_FORMAT_XRGB8888 0x34325258
#define OXY_FORMAT_ARGB8888 0x34325241

/* oxy_client_poll results. */
#define OXY_POLL_EVENT 1
#define OXY_POLL_EMPTY 0
#define OXY_POLL_CLOSED (-1)
#define OXY_ERROR (-2)

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
 *               OXY_ACTION_*. An assistive technology asks the application to perform it; an
 *               activation's serial may be cited by a popup's grab, as a key press's may. */
typedef struct OxyEvent {
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
  uint8_t flag;
  char text[33];
} OxyEvent;


/* Accessibility (ADR 0003). Roles of an accessible node. */
#define OXY_ROLE_GENERIC 0
#define OXY_ROLE_BUTTON 1
#define OXY_ROLE_LINK 2
#define OXY_ROLE_TEXT 3
#define OXY_ROLE_HEADING 4
#define OXY_ROLE_IMAGE 5
#define OXY_ROLE_TEXT_INPUT 6
#define OXY_ROLE_MENU 7
#define OXY_ROLE_MENU_ITEM 8
#define OXY_ROLE_CHECK_BOX 9
#define OXY_ROLE_RADIO 10
#define OXY_ROLE_SWITCH 11
#define OXY_ROLE_TAB 12
#define OXY_ROLE_TAB_LIST 13
#define OXY_ROLE_LIST 14
#define OXY_ROLE_LIST_ITEM 15
#define OXY_ROLE_DIALOG 16
#define OXY_ROLE_ALERT 17
#define OXY_ROLE_PROGRESS_BAR 18
#define OXY_ROLE_SLIDER 19
#define OXY_ROLE_SEARCH 20
#define OXY_ROLE_TOOLBAR 21
#define OXY_ROLE_SCROLL_VIEW 22
#define OXY_ROLE_GROUP 23
#define OXY_ROLE_COMBO_BOX 24
#define OXY_ROLE_SUMMARY 25
#define OXY_ROLE_GRID 26
/* States of an accessible node (bits). */
#define OXY_STATE_DISABLED 1
#define OXY_STATE_SELECTED 2
#define OXY_STATE_CHECKED 4
#define OXY_STATE_MIXED 8
#define OXY_STATE_EXPANDED 16
#define OXY_STATE_COLLAPSED 32
#define OXY_STATE_BUSY 64
#define OXY_STATE_FOCUSED 128
/* Actions an assistive technology asks for (OxyEvent.detail of an ACCESSIBILITY_ACTION). */
#define OXY_ACTION_ACTIVATE 1
#define OXY_ACTION_FOCUS 2
/* The parent of a root node. */
#define OXY_NO_ACCESSIBLE_PARENT 0xFFFFFFFF

/* A popup's placement: anchor rectangle in parent-local logical coordinates, the popup's logical
 * size, anchor and gravity (0 center, 1 top, 2 bottom, 3 left, 4 right, 5 top-left,
 * 6 bottom-left, 7 top-right, 8 bottom-right), adjustment flags (1 slide x, 2 slide y, 4 flip x,
 * 8 flip y, 16 resize x, 32 resize y, 64 right-to-left) and an offset. */
typedef struct OxyPositioner {
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
} OxyPositioner;

/* One node of a surface's accessibility snapshot: its parent's index (OXY_NO_ACCESSIBLE_PARENT
 * for a root), role (OXY_ROLE_*), states (OXY_STATE_* bits), surface-local logical rectangle,
 * and name and value (UTF-8, NUL-terminated; NULL for none). A secure field's value is never
 * published. */
typedef struct OxyAccessibleNode {
  uint32_t parent;
  uint8_t role;
  uint16_t state;
  int32_t x;
  int32_t y;
  uint32_t width;
  uint32_t height;
  const char* name;
  const char* value;
} OxyAccessibleNode;

const char* oxy_last_error(void);

OxyClient* oxy_client_connect(const char* path);
void oxy_client_close(OxyClient* client);
int oxy_client_fd(OxyClient* client);

int oxy_client_create_toplevel(OxyClient* client, uint32_t surface, uint32_t width, uint32_t height);
int oxy_client_create_popup(OxyClient* client, uint32_t surface, uint32_t parent,
                            const OxyPositioner* positioner);
int oxy_client_grab_popup(OxyClient* client, uint32_t surface, uint64_t serial);
/* Places an existing popup again (its anchor moved or its content resized). */
int oxy_client_reposition(OxyClient* client, uint32_t surface, const OxyPositioner* positioner);
/* Publishes a surface's accessibility snapshot, replacing the previous one: `count` nodes in
 * reading order, each parent before its children. Names and values are UTF-8 (NULL for none);
 * text longer than the protocol allows is cut at a character boundary. 0 on success. */
int oxy_client_publish_accessibility(OxyClient* client, uint32_t surface, const OxyAccessibleNode* nodes,
                                     size_t count);
int oxy_client_destroy_surface(OxyClient* client, uint32_t surface);

/* Presents pixels (rows of width * 4 bytes) on a surface in answer to the configure with serial
 * `configure`: OXY_FORMAT_XRGB8888 is opaque (little-endian B, G, R, X), OXY_FORMAT_ARGB8888 is
 * premultiplied (B, G, R, A). Returns the frame's sequence number, or -1. */
int64_t oxy_client_present(OxyClient* client, uint32_t surface, uint64_t configure, uint32_t width,
                           uint32_t height, uint32_t format, const uint8_t* pixels, size_t length);

/* A rectangle of a buffer, in its pixels. */
typedef struct OxyRect {
  int32_t x;
  int32_t y;
  uint32_t width;
  uint32_t height;
} OxyRect;

/* oxy_client_present for a frame of which only `count` rectangles at `damage` changed since the
 * surface's previous frame (0: everything). The compositor hears the damage, and a reused buffer
 * receives only what changed since it was last written. Returns the sequence number, or -1. */
int64_t oxy_client_present_damaged(OxyClient* client, uint32_t surface, uint64_t configure, uint32_t width,
                                   uint32_t height, uint32_t format, const uint8_t* pixels, size_t length,
                                   const OxyRect* damage, size_t count);

/* Takes one queued event without blocking. */
int oxy_client_poll(OxyClient* client, OxyEvent* out);

#ifdef __cplusplus
}
#endif

#endif /* OXY_CLIENT_H */
