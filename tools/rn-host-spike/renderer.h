// Retained view tree built from Fabric's mounting transactions, rasterized into a 32-bit buffer.
#pragma once

#include <react/renderer/mounting/ShadowViewMutation.h>

#include <cstdint>
#include <map>
#include <memory>
#include <optional>
#include <utility>
#include <string>
#include <unordered_map>
#include <vector>

namespace facebook::react {
class AndroidTextInputProps;
}

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
  // How far each input's text is scrolled within its content box: across for a single-line input,
  // down for a multiline one. An input keeps it when it loses focus, as an EditText does.
  std::map<facebook::react::Tag, facebook::react::Point> scrolls;
  bool operator==(const TextInputEdit&) const = default;
};

// A text input's caret geometry, from its value laid out as it is drawn (wrapped when multiline,
// centred vertically when single-line). Offsets are UTF-8 byte offsets into the value; positions
// are relative to the input's content box. A secure field's geometry is its bullets'.
class TextInputGeometry {
 public:
  TextInputGeometry(const facebook::react::ShadowView& view, const std::string& value);

  struct Caret {
    size_t line = 0;
    float x = 0, top = 0, height = 0;
  };
  // Where the caret at `offset` is drawn. An offset where a wrapped line ends is drawn at the start
  // of the next line.
  Caret caret(size_t offset) const;
  // The offset nearest a point: its line by y, then the character boundary nearest x.
  size_t offsetAt(float x, float y) const;
  // The offset on `line` nearest x.
  size_t offsetOnLine(size_t line, float x) const;
  // The first and last offsets on `line` (Home and End). A wrapped line's last offset is before
  // the character at which it wrapped, since the offset after it is drawn on the next line.
  std::pair<size_t, size_t> lineEdges(size_t line) const;
  size_t lineCount() const { return lines_.size(); }
  // The text's extent: its widest line's right edge, and its last line's bottom.
  float width() const;
  float height() const;
  // `scroll` kept within what the text's extent allows in a content box of `size`, with room
  // for the caret at the end of a line.
  facebook::react::Point clamp(facebook::react::Point scroll, facebook::react::Size size) const;
  // Rectangles covering [from, to): one per line it touches; a hard break inside it shows as a
  // little width past its line's end.
  std::vector<facebook::react::Rect> span(size_t from, size_t to) const;

 private:
  struct Line {
    size_t start = 0, end = 0;  // offsets into what is shown; `end` excludes a hard break
    bool broken = false;        // ended by a hard break (not wrapped, not the last)
    float x = 0, top = 0, height = 0;
  };
  float xOn(const Line& line, size_t shownOffset) const;
  // lineEdges, in offsets into what is shown.
  std::pair<size_t, size_t> shownEdges(size_t line) const;
  size_t toShown(size_t offset) const;
  size_t toValue(size_t shownOffset) const;

  std::shared_ptr<const facebook::react::AndroidTextInputProps> props_;
  std::string value_, shown_;
  std::vector<Line> lines_;
};

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
