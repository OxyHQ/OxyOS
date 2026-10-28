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

// Component whose subtree is shown in a popup of its own (popup.h), never in the window.
inline constexpr const char* kPopupComponent = "OxyPopupView";

// Draws the window: the tree without popup subtrees. `caret` is the text input that has focus
// (-1 for none); its caret is drawn after its text.
RenderStats draw(const ViewTree& tree, const Display& display, Canvas& canvas, facebook::react::Tag caret = -1);
// Draws a popup view's children, relative to the popup, onto a transparent canvas of `display`'s
// size (the popup's configured size).
RenderStats drawPopup(const ViewTree& tree, facebook::react::Tag popup, const Display& display, Canvas& canvas,
                      facebook::react::Tag caret = -1);
// A view's frame in its root's coordinates (window-local logical pixels), as drawn: scroll
// offsets applied. Empty if the view is not in the tree.
std::optional<facebook::react::Rect> windowFrame(const ViewTree& tree, facebook::react::Tag tag);
// The views from the root to `tag`, inclusive; empty if `tag` is not in the tree.
std::vector<facebook::react::Tag> ancestry(const ViewTree& tree, facebook::react::Tag tag);
// Draws, writes a PNG and returns the stats; `written` reports whether the PNG was saved.
RenderStats render(const ViewTree& tree, const Display& display, const std::string& pngPath);
bool writePng(const Canvas& canvas, const std::string& path);

}  // namespace oxy
