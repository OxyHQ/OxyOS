// Retained view tree built from Fabric's mounting transactions, rasterized into a 32-bit buffer.
#pragma once

#include <react/renderer/mounting/ShadowViewMutation.h>

#include <cstdint>
#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

namespace oxy {

struct Display {
  float width;
  float height;
  float scale;
};

struct View {
  facebook::react::ShadowView view;
  std::vector<facebook::react::Tag> children;
};

class ViewTree {
 public:
  void apply(const facebook::react::ShadowViewMutationList& mutations);
  const std::unordered_map<facebook::react::Tag, View>& views() const { return views_; }
  facebook::react::Tag root() const { return root_; }

 private:
  std::unordered_map<facebook::react::Tag, View> views_;
  facebook::react::Tag root_ = -1;
};

struct RenderStats {
  int views = 0;
  int texts = 0;
  int unimplemented = 0;
  std::vector<std::string> unimplementedNames;
  // The text drawn, in drawing order: each paragraph's string and each text input's value (its
  // placeholder when empty), unless clipped out entirely.
  std::vector<std::string> textContent;
  bool written = false;  // whether render() saved the PNG
};

// Pixels are 0xAARRGGBB, premultiplied, row-major, `width * scale` by `height * scale`. A window's
// canvas starts opaque white; a popup's starts transparent.
struct Canvas {
  int width = 0, height = 0;
  std::vector<uint32_t> pixels;
};

// The focused text input's editing state, drawn over its text (textinput.h owns it). Offsets
// are UTF-8 byte offsets into the input's text.
struct TextInputEdit {
  facebook::react::Tag tag = -1;
  size_t selectionStart = 0, selectionEnd = 0;  // the caret when equal
  size_t composeStart = 0, composeEnd = 0;      // the composition, underlined; empty if equal
  bool operator==(const TextInputEdit&) const = default;
};

// A text input's caret geometry: the content-local x of byte offset `offset` of `text`, and the
// offset nearest a content-local x. Single-line inputs (multiline ones use their last line).
float textInputCaretX(const facebook::react::ShadowView& view, const std::string& text, size_t offset);
size_t textInputOffsetAt(const facebook::react::ShadowView& view, const std::string& text, float x);

// Component whose subtree is shown in a popup of its own (popup.h), never in the window.
inline constexpr const char* kPopupComponent = "OxyPopupView";

// Draws the window: the tree without popup subtrees. `edit` is the focused text input's caret,
// selection and composition, drawn with its text.
RenderStats draw(const ViewTree& tree, const Display& display, Canvas& canvas, const TextInputEdit& edit = {});
// Whether two versions of a view are drawn the same (damage.h): the props `drawView` reads are
// equal (background, opacity, borders, clipping), whatever new props object a re-render made.
// A text input, whose drawing reads many of its props, is the same only with the same props.
bool drawsTheSame(const facebook::react::ShadowView& before, const facebook::react::ShadowView& after);
// Draws only the window's `damage` (window-local logical rectangles) onto `canvas`, which holds
// the previous frame at this size: each rectangle is drawn again as a whole frame draws it. The
// stats are the whole window's. A canvas of another size is drawn whole.
RenderStats drawDamaged(const ViewTree& tree, const Display& display, Canvas& canvas, const TextInputEdit& edit,
                        const std::vector<facebook::react::Rect>& damage);
// Draws a popup view's children, relative to the popup, onto a transparent canvas of `display`'s
// size (the popup's configured size).
RenderStats drawPopup(const ViewTree& tree, facebook::react::Tag popup, const Display& display, Canvas& canvas,
                      const TextInputEdit& edit = {});
// A view's frame in its root's coordinates (window-local logical pixels), as drawn: scroll
// offsets applied. Empty if the view is not in the tree.
std::optional<facebook::react::Rect> windowFrame(const ViewTree& tree, facebook::react::Tag tag);
// The views from the root to `tag`, inclusive; empty if `tag` is not in the tree.
std::vector<facebook::react::Tag> ancestry(const ViewTree& tree, facebook::react::Tag tag);
// Draws, writes a PNG and returns the stats; `written` reports whether the PNG was saved.
RenderStats render(const ViewTree& tree, const Display& display, const std::string& pngPath);
bool writePng(const Canvas& canvas, const std::string& path);

}  // namespace oxy
