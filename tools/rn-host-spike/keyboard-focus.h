// Keyboard focus beyond text fields, as a Mac's Full Keyboard Access and Android's keyboard
// navigation give it (docs/adr/0039-keyboard-access.mdx): Tab and Shift+Tab move it through the
// focusable controls of the surface that has the keyboard, in reading order, round the ends; Space
// and Return activate the focused control, the arrows step an adjustable one; a ring is drawn
// around it while the keyboard moved it (renderer.h, TextInputEdit::focusRing), and a press of the
// pointer takes it away, as Android's touch mode hides its focus highlight.
//
// A view is focusable as React Native's Android view is (`focusable`, which Pressable and the
// touchables set) when it is also an accessible element, enabled, shown, and not hidden from
// assistive technologies; an editable text input is too. What it traverses is the surface's
// content: the window's, or the topmost modal's while one is shown, or a view marked modal for
// assistive technologies (`accessibilityViewIsModal`, a Bloom overlay's root), so a modal sheet
// holds the keyboard as it holds VoiceOver's cursor.
#pragma once

#include "renderer.h"

#include <map>
#include <optional>
#include <vector>

namespace oxyos {

// The view a surface's keyboard navigation goes through: the topmost view under `root` marked
// modal for assistive technologies (`accessibilityViewIsModal`, shown), else `root`.
facebook::react::Tag navigationRoot(const ViewTree& tree, facebook::react::Tag root);

// The focusable views under `root`, in reading order (each parent's children in order).
std::vector<facebook::react::Tag> focusables(const ViewTree& tree, facebook::react::Tag root);

// Whether a view is focusable as `focusables` counts it (wherever it is).
bool focusable(const ViewTree& tree, facebook::react::Tag tag);

// Whether a view is an editable text input (focused through the text editing, textinput.h).
bool editableInput(const ViewTree& tree, facebook::react::Tag tag);

class KeyboardFocus {
 public:
  explicit KeyboardFocus(const ViewTree& tree) : tree_(tree) {}

  // The focused view, -1 for none.
  facebook::react::Tag focused() const { return focused_; }
  // Whether its ring shows: the keyboard moved it, and no press came since.
  bool ringShown() const { return ring_ && focused_ >= 0; }
  // The next (or the previous) focusable view in `root` after the focused one, round the ends; the
  // first (or the last) when the focused view is not in it. -1 if `root` has none.
  facebook::react::Tag next(facebook::react::Tag root, bool backwards) const;
  // Focus moved to `tag` (-1 for none) by the keyboard (`ring`) or otherwise.
  void set(facebook::react::Tag tag, bool ring) {
    focused_ = tag;
    ring_ = ring;
  }
  // A press of the pointer: the ring goes, as Android's touch mode hides its focus highlight.
  void pressed() { ring_ = false; }
  // Before a modal or a view marked modal takes the keyboard: where focus was, to give it back to
  // when it goes, as a Mac's sheet gives the keyboard back to its window's control.
  void remember(facebook::react::Tag root, facebook::react::Tag tag) { before_[root] = tag; }
  // What `remember` kept for `root`, forgotten; -1 for none.
  facebook::react::Tag recall(facebook::react::Tag root);
  // Views no longer in the tree lose focus and what was remembered of them.
  void prune();

 private:
  const ViewTree& tree_;
  facebook::react::Tag focused_ = -1;
  bool ring_ = false;
  std::map<facebook::react::Tag, facebook::react::Tag> before_;
};

}  // namespace oxyos
