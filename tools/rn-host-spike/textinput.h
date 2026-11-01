// Text input on OxyOS: React Native's Android text input component (its props, state and shadow
// node, built unchanged) with an OxyOS descriptor, and the native editing side that Android's
// EditText provides: focus from JavaScript commands, committed text and editing keys from the
// compositor, state updates and events back to JavaScript.
#pragma once

#include "renderer.h"

#include <oxyos_client.h>

#include <react/renderer/components/androidtextinput/AndroidTextInputShadowNode.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>
#include <react/renderer/textlayoutmanager/TextLayoutManager.h>
#include <react/timing/primitives.h>

#include <folly/dynamic.h>

#include <chrono>
#include <functional>
#include <map>
#include <memory>
#include <optional>
#include <string>
#include <tuple>
#include <vector>

namespace oxyos {

// Android's descriptor without its JNI theme lookup: OxyOS has no platform text-field theme, so a
// text input takes only the padding its props give it (Bloom styles its own fields).
class TextInputComponentDescriptor final
    : public facebook::react::ConcreteComponentDescriptor<facebook::react::AndroidTextInputShadowNode> {
 public:
  explicit TextInputComponentDescriptor(const facebook::react::ComponentDescriptorParameters& parameters);

  facebook::react::State::Shared createInitialState(
      const facebook::react::Props::Shared& props,
      const facebook::react::ShadowNodeFamily::Shared& family) const override;

 protected:
  void adopt(facebook::react::ShadowNode& shadowNode) const override;

 private:
  std::shared_ptr<const facebook::react::TextLayoutManager> textLayoutManager_;
};

// Where a view is drawn: its surface and its frame in that surface's logical coordinates.
struct Placement {
  uint32_t surface = 0;
  facebook::react::Rect frame{};
};

// The editing a focused input receives, as Android's EditText does it with a Mac's keys
// (docs/adr/0037-text-editing.mdx; Ctrl is the applications' ⌘ on OxyOS, ADR 0024, and Alt is ⌥):
// a caret and selection (arrows; by word with Alt; to the line's edge with Ctrl+Left and
// Ctrl+Right, Home and End; to the text's with Ctrl+Up and Ctrl+Down, Ctrl+Home and Ctrl+End; to
// the paragraph's with Alt+Up and Alt+Down; Up and Down across lines keeping their column, Page Up
// and Page Down; Shift to extend; Ctrl+A; the pointer: a press, a drag, Shift and a press, a
// double click selecting a word and a triple click a paragraph, either dragged on by words or by
// paragraphs), deletion by character, by word (Alt) or to the line's edge (Ctrl), committed text
// that replaces the selection, an input method's composition (preedit), which is part of the text
// while it lasts, undo and redo (Ctrl+Z, Ctrl+Shift+Z), copy, cut and paste, text scrolled to
// keep the caret in view, the caret's blinking, and dragging selected text out of an input and
// dropping text into one (the platform's drag and drop, docs/adr/0035-drag-and-drop.mdx).
class TextEditing {
 public:
  // `locate` places a view: text, keys and pointer input reach the focused input only on its
  // own surface (typing into an open menu must not edit the field behind it).
  TextEditing(const ViewTree& tree, std::function<std::optional<Placement>(facebook::react::Tag)> locate)
      : tree_(tree), locate_(std::move(locate)) {}

  // The clipboard (clipboard.h): copying text with the serial of the key that asked, and pasting
  // with it, the text arriving later (nothing if the clipboard holds none).
  struct ClipboardAccess {
    std::function<void(uint64_t serial, const std::string& text)> copy;
    std::function<void(uint64_t serial, std::function<void(std::string)> done)> paste;
    // Whether the clipboard holds text: Paste in the context menu is enabled only then.
    std::function<bool()> hasText;
  };
  void useClipboard(ClipboardAccess clipboard) { clipboard_ = std::move(clipboard); }
  // The serial of the person's latest input to the application (a key, a press, in the window or
  // a popup of it): what a command chosen in the input's context menu cites to copy and paste.
  void useInputSerial(std::function<uint64_t()> serial) { inputSerial_ = std::move(serial); }

  // The platform's drag (drag.h): a drag of `text` from the press held, allowing copy and move,
  // with `image` under the pointer; `done` receives the action the drop took, or nothing when it
  // was cancelled. False if it did not start (`done` is then called at once).
  struct DragAccess {
    std::function<bool(const std::string& text, std::optional<DragImage> image,
                       std::function<void(std::optional<std::string>)> done)>
        start;
  };
  void useDrag(DragAccess drag) { drag_ = std::move(drag); }

  // Text dropped into inputs, as an NSTextView takes it (docs/adr/0037-text-editing.mdx). Points
  // are logical and local to `surface`.
  // Whether `tag` is an input text can be dropped into: editable and not secure.
  bool takesDrop(facebook::react::Tag tag) const;
  // Text dragged over the input `tag`: its drop caret shown where the text would go. Nothing if a
  // drop there would do nothing (over the very text it drags); else whether it prefers a move:
  // its own selected text (`own`: the drag is this client's) moves within it, anything else copies.
  std::optional<bool> dragOver(facebook::react::Tag tag, uint32_t surface, float x, float y, bool own);
  // The drag left the inputs: no drop caret.
  void dragLeft() { dropTag_ = -1; }
  // The input's own dragged text dropped back into it at that point: moved there (copied with
  // `copy`) as one edit and selected. False if the input drags no text of its own.
  bool dropOwn(facebook::react::Tag tag, uint32_t surface, float x, float y, bool copy);
  // Text dropped into the input at that point: inserted there as one edit and selected, the input
  // taking focus; as a paste, no key press, and a single-line input's line breaks become spaces.
  // Only what `maxLength` leaves room for goes in, as Android's LengthFilter keeps it.
  void dropText(facebook::react::Tag tag, uint32_t surface, float x, float y, std::string text);

  // `autoFocus`, as Android's ReactEditText takes it: an input inserted with it takes focus once it
  // is attached. `inserted` sees a transaction before it is applied; `attached` focuses after.
  void inserted(const facebook::react::ShadowViewMutationList& mutations);
  void attached();
  // A view command from JavaScript (`focus`, `blur`, `setTextAndSelection`).
  void command(const facebook::react::ShadowView& view, const std::string& name, const folly::dynamic& args);
  // Compositor input: committed text, preedit, editing keys and pointer presses and drags.
  void handle(const OxyOSEvent& event);
  // A secondary click, or a Control-click (input.h), on the input `tag` at (x, y) of `surface`:
  // its context menu, as a Mac's text field opens one (docs/adr/0037-text-editing.mdx). The input
  // takes focus if it has not; unless the press is within the selection, the word under the
  // pointer is selected (off a word, or in a secure field, the caret goes there); then JavaScript
  // is asked to show the menu: `oxyosTextMenu`, a bubbling event of the input's
  // (sample/oxyos/text-menu.js draws it with Bloom's ContextMenu in a popup), with the point in
  // the input and which of Undo, Redo, Cut, Copy, Paste and Select All apply now. False, and no
  // menu, for an input that takes no focus (not editable).
  bool contextMenu(facebook::react::Tag tag, uint32_t surface, float x, float y);
  // A gesture handler took the pointer's gesture (gesture-handler.h): a drag selecting text stops.
  void releaseDrag() {
    dragging_ = false;
    pendingDrag_.reset();
  }
  // A change from the compositor's text input for the focused input (ADR 0008), applied as one
  // edit, as Android's input connection applies a batch of deleteSurroundingText, commitText and
  // setComposingText. `current`: made for the state last told; if not, its deletion is dropped,
  // since it could remove text the user did not see.
  void transaction(const OxyOSTextTransaction& change, bool current);

  // What the compositor is told about the focused input (ADR 0008): the surface it is drawn on,
  // what it is for (OXYOS_PURPOSE_*, OXYOS_HINT_*), its text without the composition, the
  // selection within that, and the caret's rectangle in the surface's logical coordinates. A
  // password's text and selection are never told.
  struct Field {
    facebook::react::Tag tag = -1;
    uint32_t surface = 0;
    uint8_t purpose = OXYOS_PURPOSE_NORMAL;
    uint32_t hints = 0;
    std::string text;
    size_t anchor = 0, cursor = 0;
    bool operator==(const Field&) const = default;
  };
  std::optional<Field> field() const;
  // Where the focused input's caret is drawn, in its surface's logical coordinates.
  std::optional<facebook::react::Rect> caretRect() const;
  // The wheel over an input, focused or not: scrolls its text by (dx, dy) as far as it can go.
  // True if it moved.
  bool scrollText(facebook::react::Tag tag, float dx, float dy);

  // Whether the focused input is secure (`secureTextEntry`): its text is never reported, only
  // as many bullets as it has characters (`logged`).
  bool secure() const;
  std::string logged(const std::string& text) const;

  // An assistive technology sets an input's selection (Android's ACTION_SET_SELECTION): the input
  // takes focus if it has not, then selects bytes [start, end) of its text, kept within it at
  // characters. False if it cannot take focus (not editable).
  bool setSelection(facebook::react::Tag tag, size_t start, size_t end);
  // The focused input's selection, in bytes of its text, for its accessibility node; none for an
  // unfocused or a secure input (whose caret would tell how long the password is).
  std::optional<std::pair<size_t, size_t>> selection(facebook::react::Tag tag) const;

  // The caret blinks as Android's Editor blinks it, lit and dark for kBlinkPeriod each, starting
  // lit at every change of the focused input's text, selection or composition, and rests lit once
  // the field has waited kBlinkTimeout (GTK's cursor-blink-timeout, which Chromium on Linux
  // follows: no wake-ups twice a second on an idle machine) or while the session's `caret_blink`
  // preference is off (ADR 0037).
  static constexpr std::chrono::milliseconds kBlinkPeriod{500};
  static constexpr std::chrono::milliseconds kBlinkTimeout{10'000};
  void setCaretBlinks(bool blinks);
  // Brings the blink to `now`.
  void tick(std::chrono::steady_clock::time_point now);
  // When the caret is next lit or darkened, if it blinks.
  std::optional<std::chrono::steady_clock::time_point> nextBlink() const;
  // Whether the focused input's window is the key window, the one the person's keys are for, and
  // whether the system's appearance is dark (ADR 0037). Out of the key window the caret is hidden
  // and does not blink, and a selection is drawn in the unemphasized colour of the appearance, as
  // NSTextView draws it; back in it the caret is lit anew. The input keeps its focus throughout,
  // as an NSTextView stays its window's first responder and Android's EditText keeps focus when
  // its window loses it: JavaScript hears nothing.
  void setWindow(bool key, bool dark);

  // The input that has focus, -1 if none.
  facebook::react::Tag focused() const { return focused_; }
  // What the renderer draws for the focused input: its selection (or caret) and composition.
  TextInputEdit edit() const;
  // Focus moved onto an input by the keyboard (Tab, keyboard-focus.h): its whole text selected, as
  // a Mac's field is when Tab reaches it.
  void focusFromKeyboard(facebook::react::Tag tag);
  // Focus moved off the focused input by the keyboard, onto a control that is no input.
  void unfocus() {
    if (focused_ >= 0) blur();
  }

 private:
  // Focuses `tag`; with `at`, its caret there, the selection told by the edit that follows.
  void focus(facebook::react::Tag tag, std::optional<size_t> at = std::nullopt);
  void blur();
  void key(uint32_t code, uint64_t serial);
  // Ctrl+V's text arriving for the input `tag`: replaces its selection, as one edit.
  void pasted(facebook::react::Tag tag, std::string text);
  // The focused input's editing commands, from their keys (Ctrl+C, X, V, A, Z, Shift+Z) or its
  // context menu (`oxyosEdit`): `copy`, `cut`, `paste`, `selectAll`, `undo`, `redo`. `serial` is
  // the input that asked, which the clipboard cites. One that does not apply does nothing.
  void perform(const std::string& command, uint64_t serial);
  // What the context menu's commands may do now, as its rows are enabled (NSTextView's
  // validateMenuItem:).
  struct Commands {
    bool undo = false, redo = false, cut = false, copy = false, paste = false, selectAll = false;
  };
  Commands commands() const;
  // What a context click at the surface point (x, y) selects in the focused input: the word whose
  // character is under it, as [start, end) in bytes; none off a word (a space, punctuation, past a
  // line's end) or in a secure field.
  std::optional<std::pair<size_t, size_t>> wordUnder(uint32_t surface, float x, float y) const;
  void pointer(const OxyOSEvent& event);
  // What a double click (a word) or a triple click (a paragraph) at `offset` selects, as Android's
  // Editor selects it (selectCurrentWord, selectCurrentParagraph): [start, end) in bytes.
  std::pair<size_t, size_t> unitAt(size_t offset) const;
  // A drag after a double or triple click, at `offset`: the selection grows from what the click
  // selected by whole words or paragraphs (Editor's updateWordBasedSelection and
  // updateParagraphBasedSelection).
  void extendByUnit(size_t offset);
  // Selects from `anchor` to `caret` at once (one selection change).
  void selectRange(size_t anchor, size_t caret);
  // The byte offset under a surface-local point, for the focused input; and for any input whose
  // text is `value`. With `inside`, none outside the input.
  std::optional<size_t> offsetAt(uint32_t surface, float x, float y, bool inside) const;
  std::optional<size_t> offsetIn(facebook::react::Tag tag, const std::string& value, uint32_t surface, float x,
                                 float y, bool inside) const;
  // An input's text, focused or not.
  std::string valueOf(facebook::react::Tag tag) const;
  // Whether a surface-local point is on the focused input's selection as drawn.
  bool onSelection(uint32_t surface, float x, float y) const;
  // A primary press held on the selection, as NSTextView takes it: moved kDragThreshold it drags
  // the selected text, released first it places the caret where it was pressed.
  struct PendingDrag {
    uint32_t surface = 0;
    facebook::react::Point at;
    size_t offset = 0;
  };
  static constexpr float kDragThreshold = 4;
  // Starts the drag of the focused input's selection from `pressed`; false if it did not start.
  bool startTextDrag(const PendingDrag& pressed);
  // What came of it: moved to another target, the dragged text is deleted (ADR 0035, decision 6).
  void textDragEnded(const std::optional<std::string>& action);
  // `text` made what an input takes: a single-line input's line breaks as spaces, and no more
  // than `maxLength` leaves room for beside `kept` UTF-16 units.
  std::string fitted(const facebook::react::AndroidTextInputProps& props, std::string text, int kept) const;
  // Replaces bytes [from, to) with `insert` as the user's edit, the caret after it: new state,
  // then events, the key press (if any) first, and the edit recorded for undo. `composing` keeps
  // the inserted text as the composition.
  void replace(size_t from, size_t to, const std::string& insert, const std::string& key, bool composing = false);
  // The text and selection the user's editing leaves: the input's state, then its change and
  // selection events, each only if it changed.
  void update(std::string text, size_t anchor, size_t caret);

  // What undo returns to and redo restores.
  struct Snapshot {
    std::string text;
    size_t anchor = 0, caret = 0;
  };
  enum class Edit { Insert, Delete, Replace };
  struct Step {
    Snapshot before, after;
    Edit kind;
  };
  // An input's edits, the first `done` of them applied; valid while its text is still `text`.
  struct History {
    std::vector<Step> entries;
    size_t done = 0;
    std::string text;
  };
  static constexpr size_t kHistoryLimit = 100;
  // Records the edit from `before` to now, merged into the last one when it continues it.
  void record(const Snapshot& before, Edit kind);
  // Ends a composition, keeping its text, as one edit.
  void endComposition();
  // Ctrl+Z, or Ctrl+Shift+Z with `redo`.
  void undo(bool redo);
  // Dispatches onKeyPress for `key`, none if empty.
  void keyPress(const std::string& key);
  // When the key or text being handled happened (eventTime, input.h): what keyPress carries.
  facebook::react::HighResTimeStamp eventTime_ = facebook::react::HighResTimeStamp::now();
  // Moves the selection; `extend` keeps the anchor.
  void select(size_t caret, bool extend);
  // Up or Down: the caret to the line above or below; with `page`, a content box's height away
  // (Page Up, Page Down).
  void vertical(bool up, bool page);
  // Scrolls the focused input's text so that its caret is inside the content box.
  void reveal();
  // Records that `view`'s text is now scrolled to `scroll` (none if zero), and tells JavaScript
  // (onScroll, as Android's ReactEditText) and the report.
  void scrolled(const facebook::react::ShadowView& view, facebook::react::Point scroll,
                const TextInputGeometry& geometry);
  void selectionChanged();
  void submit();

  const ViewTree& tree_;
  std::function<std::optional<Placement>(facebook::react::Tag)> locate_;
  facebook::react::Tag focused_ = -1;
  // Inputs inserted with `autoFocus`, focused once attached.
  std::vector<facebook::react::Tag> autoFocus_;
  // A selection JavaScript set on an input while it had no focus (setTextAndSelection), in UTF-16
  // offsets: Android's EditText keeps it, and focus from JavaScript (autoFocus, focus()) shows it.
  std::map<facebook::react::Tag, std::pair<int64_t, int64_t>> unfocusedSelections_;
  std::string text_;
  size_t anchor_ = 0, caret_ = 0;  // byte offsets; the selection lies between them
  size_t composeStart_ = 0, composeEnd_ = 0;  // the composition, none if equal
  // Its styled segments (OXYOS_PREEDIT_*), as offsets into the text, from the transaction that
  // made it; none for a composition from Preedit.
  std::vector<TextInputEdit::Segment> segments_;
  bool shift_ = false, ctrl_ = false, alt_ = false;
  bool dragging_ = false;
  // The x that consecutive Up and Down moves keep, from where they began.
  std::optional<float> column_;
  // Each input's undo history, kept across focus; whether the next edit may merge into the last
  // one; the text and selection before the composition in progress.
  std::map<facebook::react::Tag, History> histories_;
  bool merging_ = false;
  std::optional<Snapshot> composeBefore_;
  // How far each input's text is scrolled (TextInputEdit::scrolls); none if not at all.
  std::map<facebook::react::Tag, facebook::react::Point> scrolls_;
  ClipboardAccess clipboard_;
  std::function<uint64_t()> inputSerial_;
  // A context click focusing an input (contextMenu): where it was, so focus selects what the click
  // selects rather than placing the caret where the last primary press was.
  std::optional<std::pair<uint32_t, facebook::react::Point>> contextPress_;
  DragAccess drag_;
  std::optional<PendingDrag> pendingDrag_;
  // The text this client drags out of an input: the input, its text and the range dragged then,
  // and whether it was dropped back into the input, and so placed already.
  struct TextDrag {
    facebook::react::Tag tag = -1;
    std::string text;
    size_t from = 0, to = 0;
    bool placed = false;
  };
  std::optional<TextDrag> textDrag_;
  // The input text dragged over would drop into, and where: its drop caret.
  facebook::react::Tag dropTag_ = -1;
  size_t dropOffset_ = 0;
  // The latest primary press: a press focuses its input on release (JavaScript sends `focus`),
  // and the caret goes where it was pressed.
  std::optional<std::pair<uint32_t, facebook::react::Point>> press_;
  // Consecutive primary presses, as Android's Editor counts them (updateTapState): a press within
  // the double-tap timeout of the last release, and within the double-tap slop of the press before
  // it, is a double click; with a mouse, the one after that a triple click; the next a single
  // press again. `lastRelease_` is when the last primary button went up (ms, CLOCK_MONOTONIC).
  enum class Taps { Single, Double, Triple };
  Taps taps_ = Taps::Single;
  std::optional<double> lastRelease_;
  // What the double or triple click being dragged selected, [start, end) in bytes.
  std::pair<size_t, size_t> unit_{0, 0};
  int eventCount_ = 0;
  // The blink (tick): whether carets blink, when the caret was last lit anew, whether it is lit,
  // and what it was lit anew for (the input, its text, selection and composition).
  bool blinks_ = true;
  std::chrono::steady_clock::time_point blinkFrom_{};
  bool caretLit_ = true;
  // The window (setWindow): key, and its appearance dark.
  bool key_ = true, dark_ = false;
  std::tuple<facebook::react::Tag, std::string, size_t, size_t, size_t, size_t> blinkedFor_{-1, {}, 0, 0, 0, 0};
};

}  // namespace oxyos
