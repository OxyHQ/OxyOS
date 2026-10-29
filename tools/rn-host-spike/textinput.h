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

#include <folly/dynamic.h>

#include <functional>
#include <map>
#include <memory>
#include <optional>
#include <string>
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

// The editing a focused input receives, as Android's EditText does it: a caret and selection
// (arrows, by word with Ctrl, Up and Down across lines, Page Up and Page Down, Home and End,
// Shift to extend, Ctrl+A, the pointer), deletion by character or word, committed text that
// replaces the selection, an input method's composition (preedit), which is part of the text
// while it lasts, undo and redo (Ctrl+Z, Ctrl+Shift+Z), and text scrolled to keep the caret in
// view.
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
  };
  void useClipboard(ClipboardAccess clipboard) { clipboard_ = std::move(clipboard); }

  // `autoFocus`, as Android's ReactEditText takes it: an input inserted with it takes focus once it
  // is attached. `inserted` sees a transaction before it is applied; `attached` focuses after.
  void inserted(const facebook::react::ShadowViewMutationList& mutations);
  void attached();
  // A view command from JavaScript (`focus`, `blur`, `setTextAndSelection`).
  void command(const facebook::react::ShadowView& view, const std::string& name, const folly::dynamic& args);
  // Compositor input: committed text, preedit, editing keys and pointer presses and drags.
  void handle(const OxyOSEvent& event);
  // A gesture handler took the pointer's gesture (gesture-handler.h): a drag selecting text stops.
  void releaseDrag() { dragging_ = false; }
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

  // The input that has focus, -1 if none.
  facebook::react::Tag focused() const { return focused_; }
  // What the renderer draws for the focused input: its selection (or caret) and composition.
  TextInputEdit edit() const;

 private:
  void focus(facebook::react::Tag tag);
  void blur();
  void key(uint32_t code, uint64_t serial);
  // Ctrl+V's text arriving for the input `tag`: replaces its selection, as one edit.
  void pasted(facebook::react::Tag tag, std::string text);
  void pointer(const OxyOSEvent& event);
  // The byte offset under a surface-local x, for the focused input.
  std::optional<size_t> offsetAt(uint32_t surface, float x, float y, bool inside) const;
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
  std::string text_;
  size_t anchor_ = 0, caret_ = 0;  // byte offsets; the selection lies between them
  size_t composeStart_ = 0, composeEnd_ = 0;  // the composition, none if equal
  // Its styled segments (OXYOS_PREEDIT_*), as offsets into the text, from the transaction that
  // made it; none for a composition from Preedit.
  std::vector<TextInputEdit::Segment> segments_;
  bool shift_ = false, ctrl_ = false;
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
  // The latest primary press: a press focuses its input on release (JavaScript sends `focus`),
  // and the caret goes where it was pressed.
  std::optional<std::pair<uint32_t, facebook::react::Point>> press_;
  int eventCount_ = 0;
};

}  // namespace oxyos
