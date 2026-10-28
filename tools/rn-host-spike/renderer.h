// Retained view tree built from Fabric's mounting transactions, rasterized into a 32-bit buffer.
#pragma once

#include <react/renderer/mounting/ShadowViewMutation.h>

#include <cstdint>
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
};

// Pixels are 0xAARRGGBB, opaque, row-major, `width * scale` by `height * scale`.
struct Canvas {
  int width = 0, height = 0;
  std::vector<uint32_t> pixels;
};

RenderStats draw(const ViewTree& tree, const Display& display, Canvas& canvas);
// Draws, writes a PNG and returns the stats.
RenderStats render(const ViewTree& tree, const Display& display, const std::string& pngPath);
void writePng(const Canvas& canvas, const std::string& path);

}  // namespace oxy
