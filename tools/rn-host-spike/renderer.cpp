#include "renderer.h"

#include "expo-blur.h"
#include "masked-view.h"

#include "svg.h"
#include "text.h"

#include <react/renderer/components/androidtextinput/AndroidTextInputProps.h>
#include <react/renderer/components/androidtextinput/AndroidTextInputState.h>
#include <react/renderer/components/image/ImageProps.h>
#include <react/renderer/components/scrollview/ScrollViewState.h>
#include <react/renderer/components/text/ParagraphState.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/components/FBReactNativeSpec/Props.h>
#include <react/renderer/core/ConcreteState.h>
#include <react/renderer/graphics/Color.h>

#include <algorithm>
#include <array>
#include <cmath>
#include <fstream>
#include <iostream>
#include <optional>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <atomic>
#include <map>
#include <memory>
#include <tuple>
#include <set>

using namespace facebook::react;

namespace oxyos {

// A canvas oxyos-paint holds, freed with the last Canvas sharing it.
struct GpuCanvas {
  OxyOSPaint* paint = nullptr;
  uint32_t id = 0;
  ~GpuCanvas() {
    if (paint && id) oxyos_paint_canvas_free(paint, id);
  }
};


void ViewTree::apply(const ShadowViewMutationList& mutations) {
  for (const auto& m : mutations) {
    switch (m.type) {
      case ShadowViewMutation::Create:
        views_[m.newChildShadowView.tag] = View{m.newChildShadowView, {}};
        break;
      case ShadowViewMutation::Delete:
        views_.erase(m.oldChildShadowView.tag);
        break;
      case ShadowViewMutation::Insert: {
        auto& parent = views_[m.parentTag];
        auto index = std::min<size_t>(static_cast<size_t>(m.index), parent.children.size());
        parent.children.insert(parent.children.begin() + static_cast<long>(index), m.newChildShadowView.tag);
        auto& child = views_[m.newChildShadowView.tag];
        child.view = m.newChildShadowView;
        child.parent = m.parentTag;
        break;
      }
      case ShadowViewMutation::Remove: {
        auto& children = views_[m.parentTag].children;
        children.erase(std::remove(children.begin(), children.end(), m.oldChildShadowView.tag), children.end());
        if (auto child = views_.find(m.oldChildShadowView.tag);
            child != views_.end() && child->second.parent == m.parentTag) {
          child->second.parent = -1;
        }
        break;
      }
      case ShadowViewMutation::Update:
        views_[m.newChildShadowView.tag].view = m.newChildShadowView;
        if (m.parentTag == 0 || root_ < 0) {
          if (std::string(m.newChildShadowView.componentName) == "RootView") root_ = m.newChildShadowView.tag;
        }
        break;
    }
  }
}

std::vector<Tag> ViewTree::path(Tag tag) const {
  std::vector<Tag> path;
  if (root_ < 0) return path;
  // A tree has no cycles; the bound keeps a malformed transaction from looping.
  for (Tag at = tag; path.size() <= views_.size(); ) {
    auto it = views_.find(at);
    if (it == views_.end()) return {};
    path.push_back(at);
    if (at == root_) {
      std::reverse(path.begin(), path.end());
      return path;
    }
    at = it->second.parent;
    if (at < 0) return {};
  }
  return {};
}

namespace {

struct Rgba {
  float r, g, b, a;
};

Rgba rgba(const SharedColor& color) {
  if (!color) return {0, 0, 0, 0};
  auto c = colorComponentsFromColor(color);
  return {c.red, c.green, c.blue, c.alpha};
}

struct Clip {
  float x0, y0, x1, y1;
  Clip intersect(const Clip& o) const {
    return {std::max(x0, o.x0), std::max(y0, o.y0), std::min(x1, o.x1), std::min(y1, o.y1)};
  }
  bool empty() const { return x1 <= x0 || y1 <= y0; }
};

// Premultiplied pixels kept at full precision, each channel a plane of levels (0 to 255): alpha,
// red, green, blue. A wide blur's result, so its smooth gradients are never quantized before they
// are stretched over the screen (Painter::smooth).
struct LevelImage {
  int width = 0, height = 0;
  std::vector<float> planes[4];
};

// An 8 × 8 ordered (Bayer) threshold in [-0.5, 0.5): added to a level before it is rounded to 8
// bits, so a gradient shallower than a level per pixel turns into a fine even pattern rather
// than bands a level apart.
inline float ditherAt(int x, int y) {
  static constexpr uint8_t bayer[8][8] = {
      {0, 32, 8, 40, 2, 34, 10, 42},  {48, 16, 56, 24, 50, 18, 58, 26}, {12, 44, 4, 36, 14, 46, 6, 38},
      {60, 28, 52, 20, 62, 30, 54, 22}, {3, 35, 11, 43, 1, 33, 9, 41},  {51, 19, 59, 27, 49, 17, 57, 25},
      {15, 47, 7, 39, 13, 45, 5, 37},  {63, 31, 55, 23, 61, 29, 53, 21}};
  return (bayer[y & 7][x & 7] + 0.5f) / 64.0f - 0.5f;
}

// What the view tree is drawn with (docs/adr/0031-gpu-buffers.mdx, decision 12): the CPU painter
// below, or the GPU painter, which records the same operations into oxyos-paint, where each one is
// worked out per pixel with the CPU painter's arithmetic. The traversal (drawView and its helpers)
// is one; it asks the painter it is given for canvases of its own kind.
class Painter {
 public:
  explicit Painter(float scale) : scale_(scale) {}
  virtual ~Painter() = default;

  // Paints only the device pixels of [x0, x1) × [y0, y1) (a damaged region); what is drawn there
  // is what a whole frame draws, since only the pixels painted are chosen, never the geometry.
  void limit(int x0, int y0, int x1, int y1) { region_ = {x0, y0, x1, y1}; }

  // Whether a logical rectangle has any pixel the painter paints.
  float scale() const { return scale_; }
  bool touches(float x, float y, float w, float h) const {
    int x0 = static_cast<int>(std::floor(x * scale_)), x1 = static_cast<int>(std::ceil((x + w) * scale_));
    int y0 = static_cast<int>(std::floor(y * scale_)), y1 = static_cast<int>(std::ceil((y + h) * scale_));
    return x1 > region_[0] && x0 < region_[2] && y1 > region_[1] && y0 < region_[3];
  }

  // The device pixels painted, as limit() set them.
  const std::array<int, 4>& region() const { return region_; }
  // Whether a window shown live is placed from what it paints (docs/adr/0022-window-views.mdx):
  // the surface's canvas, or a layer of a view only moved and scaled (a dragged card); not a mask,
  // a blur, a modal animating or a view turned or skewed. `toSurface` maps its logical pixels
  // to the surface's, and `surfaceClip` is what clips the layer there.
  bool placesWindows = true;
  Affine toSurface;
  Clip surfaceClip{-1e9f, -1e9f, 1e9f, 1e9f};

  // The canvas: `width` × `height` pixels, every one `value`.
  virtual void allocate(int width, int height, uint32_t value) = 0;
  // The canvas at another size: its pixels kept where both have them (top left), the rest `value`.
  virtual void resize(int width, int height, uint32_t value) = 0;
  // [x0, x1) × [y0, y1) of the canvas set to `value`.
  virtual void clear(int x0, int y0, int x1, int y1, uint32_t value) = 0;
  // A painter of the same kind on another canvas.
  virtual std::unique_ptr<Painter> child(Canvas& canvas, float scale) = 0;

  virtual void fill(float x, float y, float w, float h, const BorderRadii& radii, Rgba color, const Clip& clip,
                    const BorderRadii* holeRadii = nullptr, float hx = 0, float hy = 0, float hw = 0,
                    float hh = 0) = 0;
  virtual void image(const Picture& picture, float dx, float dy, float dw, float dh, const Clip& clip, float bx,
                     float by, float bw, float bh, const BorderRadii& radii, float opacity,
                     const std::optional<Rgba>& tint) = 0;
  virtual void layer(const Canvas& layer, float layerScale, const Rect& area, const Affine& map, const Clip& clip,
                     float opacity = 1) = 0;
  virtual void glyphs(const ParagraphLayout& layout, float ox, float oy, const Clip& clip, float opacity) = 0;
  // A canvas drawn as a picture is (`image`, untinted).
  virtual void imageOf(const Canvas& source, float dx, float dy, float dw, float dh, const Clip& clip, float bx,
                       float by, float bw, float bh, const BorderRadii& radii, float opacity) = 0;
  // A canvas's levels blurred by `sigma` of its pixels, drawn at full precision, dithered (`smooth`).
  virtual void smoothOf(const Canvas& source, float sigma, float dx, float dy, float dw, float dh,
                        const Clip& clip, float bx, float by, float bw, float bh, const BorderRadii& radii,
                        float opacity) = 0;
  // The canvas blurred in place by `sigma` of its pixels (gaussianBlur).
  virtual void blur(float sigma) = 0;
  // The canvas kept by the alpha of `mask`, a canvas of its size.
  virtual void keepByAlphaOf(const Canvas& mask) = 0;
  // The box of the canvas's pixels with any alpha ([x0, y0, x1, y1]), where the painter can tell
  // it without reading its pixels back; none on the GPU.
  virtual std::optional<std::array<int, 4>> opaqueBounds() const = 0;

 protected:
  float scale_;
  std::array<int, 4> region_{std::numeric_limits<int>::min(), std::numeric_limits<int>::min(),
                             std::numeric_limits<int>::max(), std::numeric_limits<int>::max()};
};

std::unique_ptr<Painter> makePainter(Canvas& canvas, float scale);

// The CPU painter: every operation worked out pixel by pixel on the canvas's pixels.
class CpuPainter final : public Painter {
 public:
  CpuPainter(Canvas& canvas, float scale) : Painter(scale), canvas_(canvas) {}

  void blend(int x, int y, Rgba color, float coverage) {
    if (x < 0 || y < 0 || x >= canvas_.width || y >= canvas_.height) return;
    if (x < region_[0] || y < region_[1] || x >= region_[2] || y >= region_[3]) return;
    float a = std::clamp(color.a * coverage, 0.0f, 1.0f);
    if (a <= 0) return;
    // Premultiplied source-over: an opaque canvas stays opaque, a transparent one gains alpha.
    uint32_t& dst = canvas_.pixels[static_cast<size_t>(y) * canvas_.width + x];
    if (a >= 1) {
      // The formula below with a = 1: the destination does not contribute.
      auto c = [](float v) { return static_cast<uint32_t>(std::lround(v * 255)); };
      dst = 0xff000000u | c(color.r) << 16 | c(color.g) << 8 | c(color.b);
      return;
    }
    auto channel = [&](int shift, float src) {
      float d = ((dst >> shift) & 0xff) / 255.0f;
      return static_cast<uint32_t>(std::lround((src * a + d * (1 - a)) * 255)) << shift;
    };
    dst = channel(24, 1.0f) | channel(16, color.r) | channel(8, color.g) | channel(0, color.b);
  }

  // A colour as a fully covered pixel takes it: premultiplied 8-bit channels, packed when opaque.
  struct Solid {
    uint32_t a, r, g, b;
    bool opaque;
    uint32_t packed;
  };
  static Solid solid(Rgba color) {
    auto c = [](float v) { return static_cast<uint32_t>(std::lround(std::clamp(v, 0.0f, 1.0f) * 255)); };
    float a = std::clamp(color.a, 0.0f, 1.0f);
    Solid s{c(a), c(color.r * a), c(color.g * a), c(color.b * a), a >= 1, 0};
    s.packed = 0xff000000u | c(color.r) << 16 | c(color.g) << 8 | c(color.b);
    return s;
  }
  // Source-over of a fully covered pixel, in integers: the inside of a fill, where most of a
  // frame's pixels are, costs one store when opaque.
  static void blendSolid(uint32_t& dst, const Solid& s) {
    if (s.opaque) {
      dst = s.packed;
      return;
    }
    uint32_t keep = 255 - s.a;
    auto channel = [&](int shift, uint32_t src) {
      return (src + (((dst >> shift) & 0xff) * keep + 127) / 255) << shift;
    };
    dst = channel(24, s.a) | channel(16, s.r) | channel(8, s.g) | channel(0, s.b);
  }

  // Whether a point lies in a rectangle with circular corner radii (logical units).
  static bool insideRounded(float px, float py, float x, float y, float w, float h, const BorderRadii& r) {
    if (px < x || py < y || px > x + w || py > y + h) return false;
    float limit = std::min(w, h) / 2;
    auto corner = [&](float cx, float cy, float radius) {
      float dx = px - cx, dy = py - cy;
      return dx * dx + dy * dy <= radius * radius;
    };
    float tl = std::min(r.topLeft.horizontal, limit), tr = std::min(r.topRight.horizontal, limit);
    float bl = std::min(r.bottomLeft.horizontal, limit), br = std::min(r.bottomRight.horizontal, limit);
    if (px < x + tl && py < y + tl) return corner(x + tl, y + tl, tl);
    if (px > x + w - tr && py < y + tr) return corner(x + w - tr, y + tr, tr);
    if (px < x + bl && py > y + h - bl) return corner(x + bl, y + h - bl, bl);
    if (px > x + w - br && py > y + h - br) return corner(x + w - br, y + h - br, br);
    return true;
  }

  // Fills a rounded rectangle, optionally minus an inner one (a border ring).
  void fill(float x, float y, float w, float h, const BorderRadii& radii, Rgba color, const Clip& clip,
            const BorderRadii* holeRadii = nullptr, float hx = 0, float hy = 0, float hw = 0, float hh = 0) {
    if (color.a <= 0 || w <= 0 || h <= 0) return;
    Clip box = Clip{x, y, x + w, y + h}.intersect(clip);
    if (box.empty()) return;
    int x0 = static_cast<int>(std::floor(box.x0 * scale_)), x1 = static_cast<int>(std::ceil(box.x1 * scale_));
    int y0 = static_cast<int>(std::floor(box.y0 * scale_)), y1 = static_cast<int>(std::ceil(box.y1 * scale_));
    // Only the pixels painted are visited; each one's coverage is the whole frame's.
    x0 = std::max({x0, region_[0], 0}), y0 = std::max({y0, region_[1], 0});
    x1 = std::min({x1, region_[2], canvas_.width}), y1 = std::min({y1, region_[3], canvas_.height});
    Solid full = solid(color);
    bool rounded = radii.topLeft.horizontal > 0 || radii.topRight.horizontal > 0 ||
                   radii.bottomLeft.horizontal > 0 || radii.bottomRight.horizontal > 0 || holeRadii;
    const float s = 1 / scale_;
    for (int py = y0; py < y1; ++py) {
      const float ly = py / scale_;
      // The run of the row's pixels a rounded fill covers whole, inside the clip, without
      // sampling them one by one: each takes the colour; a ring (a border) paints nothing where
      // its hole covers a pixel whole, so the hole's run is skipped. Every pixel outside the runs
      // is sampled as before, and the runs are the pixels roundedCoverage's shortcut finds.
      auto [solidFrom, solidTo] = rounded ? clearRun(ly, s, x, y, w, h, radii, clip) : std::pair{0, 0};
      auto [holeFrom, holeTo] = holeRadii ? clearRun(ly, s, hx, hy, hw, hh, *holeRadii, clip) : std::pair{0, 0};
      if (holeRadii) solidFrom = solidTo = 0;
      for (int px = x0; px < x1; ++px) {
        if (px >= holeFrom && px < holeTo) {
          px = holeTo - 1;
          continue;
        }
        if (px >= solidFrom && px < solidTo) {
          uint32_t* row = canvas_.pixels.data() + static_cast<size_t>(py) * canvas_.width;
          for (int end = std::min(solidTo, x1); px < end; ++px) blendSolid(row[px], full);
          --px;
          continue;
        }
        float lx = px / scale_;
        if (lx + 1 / scale_ <= clip.x0 || lx >= clip.x1 || ly + 1 / scale_ <= clip.y0 || ly >= clip.y1) continue;
        float coverage;
        if (rounded) {
          // Subsample in logical space at device-pixel granularity.
          coverage = roundedCoverage(lx, ly, s, x, y, w, h, radii);
          if (holeRadii && coverage > 0) coverage -= roundedCoverage(lx, ly, s, hx, hy, hw, hh, *holeRadii);
        } else {
          float cx = std::min(lx + 1 / scale_, box.x1) - std::max(lx, box.x0);
          float cy = std::min(ly + 1 / scale_, box.y1) - std::max(ly, box.y0);
          coverage = std::clamp(cx * scale_, 0.0f, 1.0f) * std::clamp(cy * scale_, 0.0f, 1.0f);
        }
        if (coverage >= 1) {
          blendSolid(canvas_.pixels[static_cast<size_t>(py) * canvas_.width + px], full);
        } else {
          blend(px, py, color, std::clamp(coverage, 0.0f, 1.0f));
        }
      }
    }
  }

  // The 4x4-sampled coverage below, without sampling where every sample agrees: a pixel square
  // outside the rectangle covers nothing, and one inside it that meets no corner square covers
  // all of it (the same clamped radii as insideRounded).
  static float roundedCoverage(float lx, float ly, float s, float x, float y, float w, float h,
                               const BorderRadii& r) {
    if (lx + s <= x || ly + s <= y || lx >= x + w || ly >= y + h) return 0;
    if (coveredWhole(lx, ly, s, x, y, w, h, r)) return 1;
    return roundedCoverageScaled(lx, ly, s, x, y, w, h, r);
  }

  // Whether the pixel square at (lx, ly) of logical size `s` lies inside the rounded rectangle
  // and meets none of its corner squares: it is covered whole.
  static bool coveredWhole(float lx, float ly, float s, float x, float y, float w, float h, const BorderRadii& r) {
    if (lx < x || ly < y || lx + s > x + w || ly + s > y + h) return false;
    float limit = std::min(w, h) / 2;
    float tl = std::min(r.topLeft.horizontal, limit), tr = std::min(r.topRight.horizontal, limit);
    float bl = std::min(r.bottomLeft.horizontal, limit), br = std::min(r.bottomRight.horizontal, limit);
    return (lx >= x + tl || ly >= y + tl) && (lx + s <= x + w - tr || ly >= y + tr) &&
           (lx >= x + bl || ly + s <= y + h - bl) && (lx + s <= x + w - br || ly + s <= y + h - br);
  }

  // The device pixels [from, to) of the row at `ly` that the rounded rectangle covers whole
  // (coveredWhole) and the clip holds whole; empty when none. On a row the pixels covered whole
  // are one run (every condition bounds the square's left or right edge), so the run is found
  // from its bounds and its ends checked with coveredWhole itself.
  std::pair<int, int> clearRun(float ly, float s, float x, float y, float w, float h, const BorderRadii& r,
                               const Clip& clip) const {
    if (ly < y || ly + s > y + h || ly < clip.y0 || ly + s > clip.y1) return {0, 0};
    float limit = std::min(w, h) / 2;
    float tl = std::min(r.topLeft.horizontal, limit), tr = std::min(r.topRight.horizontal, limit);
    float bl = std::min(r.bottomLeft.horizontal, limit), br = std::min(r.bottomRight.horizontal, limit);
    float left = std::max(x, clip.x0), right = std::min(x + w, clip.x1);
    if (ly < y + tl) left = std::max(left, x + tl);
    if (ly + s > y + h - bl) left = std::max(left, x + bl);
    if (ly < y + tr) right = std::min(right, x + w - tr);
    if (ly + s > y + h - br) right = std::min(right, x + w - br);
    int from = static_cast<int>(std::ceil(left * scale_)), to = static_cast<int>(std::floor(right * scale_));
    auto whole = [&](int px) {
      float lx = px / scale_;
      return coveredWhole(lx, ly, s, x, y, w, h, r) && lx >= clip.x0 && lx + s <= clip.x1;
    };
    while (from < to && !whole(from)) ++from;
    while (to > from && !whole(to - 1)) --to;
    return {from, to};
  }

  // 4x4-sampled coverage of a device pixel whose logical size is `s`.
  static float roundedCoverageScaled(float lx, float ly, float s, float x, float y, float w, float h,
                                     const BorderRadii& r) {
    int hits = 0;
    for (int sy = 0; sy < 4; ++sy) {
      for (int sx = 0; sx < 4; ++sx) {
        hits += insideRounded(lx + (sx + 0.5f) * s / 4, ly + (sy + 0.5f) * s / 4, x, y, w, h, r);
      }
    }
    return hits / 16.0f;
  }

  // Draws `picture` stretched over the logical rectangle (dx, dy, dw, dh), sampled bilinearly,
  // within `clip`, rounded by `radii` over the box (bx, by, bw, bh). With `tint`, each pixel takes
  // the tint's colour and keeps its own alpha (Android's SRC_IN colour filter).
  void image(const Picture& picture, float dx, float dy, float dw, float dh, const Clip& clip, float bx, float by,
             float bw, float bh, const BorderRadii& radii, float opacity, const std::optional<Rgba>& tint) {
    if (picture.width <= 0 || picture.height <= 0 || dw <= 0 || dh <= 0 || opacity <= 0) return;
    Clip box = Clip{dx, dy, dx + dw, dy + dh}.intersect(clip);
    if (box.empty()) return;
    int x0 = std::max(static_cast<int>(std::floor(box.x0 * scale_)), region_[0]);
    int y0 = std::max(static_cast<int>(std::floor(box.y0 * scale_)), region_[1]);
    int x1 = std::min(static_cast<int>(std::ceil(box.x1 * scale_)), region_[2]);
    int y1 = std::min(static_cast<int>(std::ceil(box.y1 * scale_)), region_[3]);
    x0 = std::max(x0, 0), y0 = std::max(y0, 0), x1 = std::min(x1, canvas_.width), y1 = std::min(y1, canvas_.height);
    bool rounded = radii.topLeft.horizontal > 0 || radii.topRight.horizontal > 0 ||
                   radii.bottomLeft.horizontal > 0 || radii.bottomRight.horizontal > 0;
    const float s = 1 / scale_;
    auto texel = [&](int tx, int ty, int c) {
      tx = std::clamp(tx, 0, picture.width - 1), ty = std::clamp(ty, 0, picture.height - 1);
      return picture.pixels[(static_cast<size_t>(ty) * picture.width + tx) * 4 + c] / 255.0f;
    };
    // A picture whose texels fall on the device pixels one to one (as Images decodes one at the
    // size it is drawn, and as a view's children drawn on their own are), whole pixels inside the
    // box, untinted and opaque: each pixel is its texel, the bilinear sample at a texel's centre,
    // and an opaque one is copied. A translucent texel is blended as below, and so is a pixel a
    // rounded box does not cover whole (a corner's), at the coverage below.
    auto whole = [](float value) { return std::abs(value - std::round(value)) < 1e-3f; };
    const float left = dx * scale_, top = dy * scale_;
    if (!tint && opacity >= 1 && whole(left) && whole(top) &&
        std::abs(dw * scale_ - picture.width) < 1e-3f && std::abs(dh * scale_ - picture.height) < 1e-3f &&
        whole(box.x0 * scale_) && whole(box.y0 * scale_) && whole(box.x1 * scale_) && whole(box.y1 * scale_)) {
      // The picture's edges and the box's are whole only to within a rounding error (at 1.25,
      // 24.8 * 1.25 is 30.999998): each is rounded to the pixel it is, never floored or ceiled as
      // above, which took a row or column outside the picture's pixels. What is drawn is the
      // picture's pixels within the box, the region and the canvas.
      const int ox = static_cast<int>(std::round(left)), oy = static_cast<int>(std::round(top));
      const int bx0 = std::max({static_cast<int>(std::round(box.x0 * scale_)), region_[0], 0, ox});
      const int by0 = std::max({static_cast<int>(std::round(box.y0 * scale_)), region_[1], 0, oy});
      const int bx1 = std::min({static_cast<int>(std::round(box.x1 * scale_)), region_[2], canvas_.width,
                                ox + picture.width});
      const int by1 = std::min({static_cast<int>(std::round(box.y1 * scale_)), region_[3], canvas_.height,
                                oy + picture.height});
      for (int py = by0; py < by1; ++py) {
        const uint8_t* row = picture.pixels.data() + static_cast<size_t>(py - oy) * picture.width * 4;
        uint32_t* out = canvas_.pixels.data() + static_cast<size_t>(py) * canvas_.width;
        const float ly = py * s;
        // The row's pixels the rounded box covers whole, which need no coverage of their own.
        auto [solidFrom, solidTo] =
            rounded ? clearRun(ly, s, bx, by, bw, bh, radii, box) : std::pair{bx0, bx1};
        for (int px = bx0; px < bx1; ++px) {
          const uint8_t* t = row + static_cast<size_t>(px - ox) * 4;
          if (t[3] == 0) continue;  // as below: nothing where the sample has no alpha
          float coverage = 1;
          if (px < solidFrom || px >= solidTo) {
            coverage = roundedCoverage(px * s, ly, s, bx, by, bw, bh, radii);
            if (coverage <= 0) continue;
          }
          if (t[3] == 255 && coverage >= 1) {
            out[px] = 0xff000000u | (uint32_t{t[0]} << 16) | (uint32_t{t[1]} << 8) | t[2];
            continue;
          }
          const float alpha = t[3] / 255.0f * coverage;
          uint32_t& dst = out[px];
          auto channel = [&](int shift, float src) {
            float d = ((dst >> shift) & 0xff) / 255.0f;
            return static_cast<uint32_t>(std::lround(std::clamp(src + d * (1 - alpha), 0.0f, 1.0f) * 255)) << shift;
          };
          dst = channel(24, alpha) | channel(16, t[0] / 255.0f * coverage) | channel(8, t[1] / 255.0f * coverage) |
                channel(0, t[2] / 255.0f * coverage);
        }
      }
      return;
    }
    for (int py = y0; py < y1; ++py) {
      for (int px = x0; px < x1; ++px) {
        float lx = px * s, ly = py * s;
        // The share of this device pixel inside the destination and the clip.
        float cx = std::min(lx + s, box.x1) - std::max(lx, box.x0);
        float cy = std::min(ly + s, box.y1) - std::max(ly, box.y0);
        float coverage = std::clamp(cx * scale_, 0.0f, 1.0f) * std::clamp(cy * scale_, 0.0f, 1.0f);
        if (rounded && coverage > 0) coverage *= roundedCoverage(lx, ly, s, bx, by, bw, bh, radii);
        if (coverage <= 0) continue;
        // The picture's point under this pixel's centre, between texel centres.
        float u = ((lx + s / 2 - dx) / dw) * picture.width - 0.5f;
        float v = ((ly + s / 2 - dy) / dh) * picture.height - 0.5f;
        int u0 = static_cast<int>(std::floor(u)), v0 = static_cast<int>(std::floor(v));
        float fu = u - u0, fv = v - v0;
        float sample[4];
        for (int c = 0; c < 4; ++c) {
          float top = texel(u0, v0, c) * (1 - fu) + texel(u0 + 1, v0, c) * fu;
          float bottom = texel(u0, v0 + 1, c) * (1 - fu) + texel(u0 + 1, v0 + 1, c) * fu;
          sample[c] = top * (1 - fv) + bottom * fv;
        }
        float alpha = sample[3] * opacity * coverage;
        if (alpha <= 0) continue;
        float r, g, b;  // premultiplied by `alpha`
        if (tint) {
          r = tint->r * alpha * tint->a, g = tint->g * alpha * tint->a, b = tint->b * alpha * tint->a;
          alpha *= tint->a;
        } else {
          float k = opacity * coverage;
          r = sample[0] * k, g = sample[1] * k, b = sample[2] * k;
        }
        uint32_t& dst = canvas_.pixels[static_cast<size_t>(py) * canvas_.width + px];
        auto channel = [&](int shift, float src) {
          float d = ((dst >> shift) & 0xff) / 255.0f;
          return static_cast<uint32_t>(std::lround(std::clamp(src + d * (1 - alpha), 0.0f, 1.0f) * 255)) << shift;
        };
        dst = channel(24, alpha) | channel(16, r) | channel(8, g) | channel(0, b);
      }
    }
  }

  // Draws `layer`, a subtree drawn on its own at `layerScale` device pixels per logical pixel over
  // `area` (in the subtree's own coordinates), mapped onto the canvas through `map`, within `clip`:
  // each device pixel takes the layer's bilinear sample under its centre, transparent outside it, so
  // a turned edge is smoothed as the layer's pixels are. The layer is drawn at `opacity`.
  void layer(const Canvas& layer, float layerScale, const Rect& area, const Affine& map, const Clip& clip,
             float opacity = 1) {
    if (layer.width <= 0 || layer.height <= 0 || std::abs(map.determinant()) < 1e-6f || opacity <= 0) return;
    // A layer drawn on the GPU (a modal's last frame, drawn before the GPU path stopped, or drawn
    // on the CPU again by the render check): its pixels read back first.
    if (layer.gpu) {
      Canvas read{layer.width, layer.height, canvasPixels(layer), nullptr};
      if (!read.pixels.empty()) this->layer(read, layerScale, area, map, clip, opacity);
      return;
    }
    if (layer.pixels.size() < static_cast<size_t>(layer.width) * layer.height) return;
    Rect drawn = map.bounds(area);
    Clip box = Clip{drawn.origin.x, drawn.origin.y, drawn.getMaxX(), drawn.getMaxY()}.intersect(clip);
    if (box.empty()) return;
    int x0 = std::max({static_cast<int>(std::floor(box.x0 * scale_)), region_[0], 0});
    int y0 = std::max({static_cast<int>(std::floor(box.y0 * scale_)), region_[1], 0});
    int x1 = std::min({static_cast<int>(std::ceil(box.x1 * scale_)), region_[2], canvas_.width});
    int y1 = std::min({static_cast<int>(std::ceil(box.y1 * scale_)), region_[3], canvas_.height});
    Affine back = map.inverse();
    const float s = 1 / scale_;
    auto texel = [&](int tx, int ty) -> uint32_t {
      if (tx < 0 || ty < 0 || tx >= layer.width || ty >= layer.height) return 0;
      return layer.pixels[static_cast<size_t>(ty) * layer.width + tx];
    };
    for (int py = y0; py < y1; ++py) {
      for (int px = x0; px < x1; ++px) {
        float lx = (px + 0.5f) * s, ly = (py + 0.5f) * s;
        if (lx < clip.x0 || lx >= clip.x1 || ly < clip.y0 || ly >= clip.y1) continue;
        Point local = back.apply(Point{lx, ly});
        float u = (local.x - area.origin.x) * layerScale - 0.5f, v = (local.y - area.origin.y) * layerScale - 0.5f;
        int u0 = static_cast<int>(std::floor(u)), v0 = static_cast<int>(std::floor(v));
        if (u0 < -1 || v0 < -1 || u0 >= layer.width || v0 >= layer.height) continue;
        float fu = u - u0, fv = v - v0;
        uint32_t t00 = texel(u0, v0), t10 = texel(u0 + 1, v0), t01 = texel(u0, v0 + 1), t11 = texel(u0 + 1, v0 + 1);
        if ((t00 | t10 | t01 | t11) == 0) continue;
        // Premultiplied channels, interpolated as they are.
        auto sample = [&](int shift) {
          auto c = [&](uint32_t t) { return ((t >> shift) & 0xff) / 255.0f; };
          return ((c(t00) * (1 - fu) + c(t10) * fu) * (1 - fv) + (c(t01) * (1 - fu) + c(t11) * fu) * fv) * opacity;
        };
        float alpha = sample(24);
        if (alpha <= 0) continue;
        uint32_t& dst = canvas_.pixels[static_cast<size_t>(py) * canvas_.width + px];
        auto channel = [&](int shift, float src) {
          float d = ((dst >> shift) & 0xff) / 255.0f;
          return static_cast<uint32_t>(std::lround(std::clamp(src + d * (1 - alpha), 0.0f, 1.0f) * 255)) << shift;
        };
        dst = channel(24, alpha) | channel(16, sample(16)) | channel(8, sample(8)) | channel(0, sample(0));
      }
    }
  }

  void glyphs(const ParagraphLayout& layout, float ox, float oy, const Clip& clip, float opacity) {
    auto& engine = TextEngine::shared();
    for (const auto& run : layout.runs) {
      Rgba color{((run.color >> 16) & 0xff) / 255.0f, ((run.color >> 8) & 0xff) / 255.0f,
                 (run.color & 0xff) / 255.0f, ((run.color >> 24) & 0xff) / 255.0f * opacity};
      for (const auto& glyph : run.glyphs) {
        const auto& bitmap = engine.rasterize(glyph.face, glyph.id, scale_);
        int gx = static_cast<int>(std::lround((ox + glyph.x) * scale_)) + bitmap.left;
        int gy = static_cast<int>(std::lround((oy + run.baseline) * scale_)) - bitmap.top;
        if (gx + bitmap.width <= region_[0] || gx >= region_[2] || gy + bitmap.height <= region_[1] ||
            gy >= region_[3]) {
          continue;
        }
        for (int y = 0; y < bitmap.height; ++y) {
          for (int x = 0; x < bitmap.width; ++x) {
            float lx = (gx + x) / scale_, ly = (gy + y) / scale_;
            if (lx < clip.x0 || lx >= clip.x1 || ly < clip.y0 || ly >= clip.y1) continue;
            uint8_t value = bitmap.pixels[static_cast<size_t>(y) * bitmap.width + x];
            if (value) blend(gx + x, gy + y, color, value / 255.0f);
          }
        }
      }
    }
  }

  // Draws `image` stretched over the logical rectangle (dx, dy, dw, dh), sampled bilinearly at
  // full precision and dithered as it is written in 8 bits (ditherAt), within `clip`, rounded by
  // `radii` over the box (bx, by, bw, bh), at `opacity`: a wide blur's result, whose gradients
  // would otherwise band, the steps a level apart following the reduced pixels' grid.
  void smooth(const LevelImage& image, float dx, float dy, float dw, float dh, const Clip& clip, float bx, float by,
              float bw, float bh, const BorderRadii& radii, float opacity) {
    if (image.width <= 0 || image.height <= 0 || dw <= 0 || dh <= 0 || opacity <= 0) return;
    Clip box = Clip{dx, dy, dx + dw, dy + dh}.intersect(clip);
    if (box.empty()) return;
    int x0 = std::max({static_cast<int>(std::floor(box.x0 * scale_)), region_[0], 0});
    int y0 = std::max({static_cast<int>(std::floor(box.y0 * scale_)), region_[1], 0});
    int x1 = std::min({static_cast<int>(std::ceil(box.x1 * scale_)), region_[2], canvas_.width});
    int y1 = std::min({static_cast<int>(std::ceil(box.y1 * scale_)), region_[3], canvas_.height});
    if (x0 >= x1 || y0 >= y1) return;
    bool rounded = radii.topLeft.horizontal > 0 || radii.topRight.horizontal > 0 ||
                   radii.bottomLeft.horizontal > 0 || radii.bottomRight.horizontal > 0;
    const float s = 1 / scale_;
    // Each column's share inside the box, its two samples and the weight of the second.
    struct Column {
      float cover, weight;
      int left, right;
    };
    std::vector<Column> columns(static_cast<size_t>(x1 - x0));
    for (int px = x0; px < x1; ++px) {
      float lx = px * s;
      float u = ((lx + s / 2 - dx) / dw) * image.width - 0.5f;
      int u0 = static_cast<int>(std::floor(u));
      columns[px - x0] = Column{std::clamp((std::min(lx + s, box.x1) - std::max(lx, box.x0)) * scale_, 0.0f, 1.0f),
                                u - u0, std::clamp(u0, 0, image.width - 1), std::clamp(u0 + 1, 0, image.width - 1)};
    }
    // A source row stretched along the columns, kept while the device rows between it and the next
    // are drawn (a reduced row serves k device rows): each pixel is then one step between two.
    const size_t span = columns.size();
    std::vector<float> stretched[2] = {std::vector<float>(span * 4), std::vector<float>(span * 4)};
    int held[2] = {-1, -1};
    auto stretch = [&](int sourceRow, int slot) {
      if (held[slot] == sourceRow) return;
      if (held[1 - slot] == sourceRow) {
        stretched[slot] = stretched[1 - slot];
      } else {
        size_t base = static_cast<size_t>(sourceRow) * image.width;
        for (int c = 0; c < 4; ++c) {
          const float* plane = image.planes[c].data() + base;
          for (size_t i = 0; i < span; ++i) {
            const Column& column = columns[i];
            stretched[slot][i * 4 + c] =
                plane[column.left] + (plane[column.right] - plane[column.left]) * column.weight;
          }
        }
      }
      held[slot] = sourceRow;
    };
    for (int py = y0; py < y1; ++py) {
      float ly = py * s;
      float rowCover = std::clamp((std::min(ly + s, box.y1) - std::max(ly, box.y0)) * scale_, 0.0f, 1.0f);
      float v = ((ly + s / 2 - dy) / dh) * image.height - 0.5f;
      int v0 = static_cast<int>(std::floor(v));
      float fv = v - v0;
      stretch(std::clamp(v0, 0, image.height - 1), 0);
      stretch(std::clamp(v0 + 1, 0, image.height - 1), 1);
      uint32_t* row = canvas_.pixels.data() + static_cast<size_t>(py) * canvas_.width;
      for (int px = x0; px < x1; ++px) {
        const size_t i = static_cast<size_t>(px - x0);
        float coverage = columns[i].cover * rowCover;
        if (rounded && coverage > 0) coverage *= roundedCoverage(px * s, ly, s, bx, by, bw, bh, radii);
        if (coverage <= 0) continue;
        const float* top = &stretched[0][i * 4];
        const float* bottom = &stretched[1][i * 4];
        const float k = opacity * coverage;
        float level[4];
        for (int c = 0; c < 4; ++c) level[c] = (top[c] + (bottom[c] - top[c]) * fv) * k;
        const float dither = ditherAt(px, py);
        // Rounded to the nearest level (the sum is never below -0.5, so adding a half and
        // truncating is rounding).
        auto quantize = [dither](float value) {
          return static_cast<uint32_t>(std::clamp(static_cast<int>(value + dither + 0.5f), 0, 255));
        };
        uint32_t& out = row[px];
        const float alpha = level[0] / 255.0f;
        if (alpha <= 0) continue;
        if (level[0] >= 254.5f) {
          out = 0xff000000u | (quantize(level[1]) << 16) | (quantize(level[2]) << 8) | quantize(level[3]);
          continue;
        }
        auto over = [&](int shift, float source) {
          return quantize(source + ((out >> shift) & 0xff) * (1 - alpha)) << shift;
        };
        out = over(24, level[0]) | over(16, level[1]) | over(8, level[2]) | over(0, level[3]);
      }
    }
  }

  void allocate(int width, int height, uint32_t value);
  void resize(int width, int height, uint32_t value);
  void clear(int x0, int y0, int x1, int y1, uint32_t value);
  std::unique_ptr<Painter> child(Canvas& canvas, float scale);
  void imageOf(const Canvas& source, float dx, float dy, float dw, float dh, const Clip& clip, float bx, float by,
               float bw, float bh, const BorderRadii& radii, float opacity);
  void smoothOf(const Canvas& source, float sigma, float dx, float dy, float dw, float dh, const Clip& clip,
                float bx, float by, float bw, float bh, const BorderRadii& radii, float opacity);
  void blur(float sigma);
  void keepByAlphaOf(const Canvas& mask);
  std::optional<std::array<int, 4>> opaqueBounds() const;

 private:
  Canvas& canvas_;
};

const std::set<std::string> kImplemented = {"RootView", "View", "Paragraph", "Text", "RawText", "ScrollView",
                                            "AndroidTextInput", "AndroidHorizontalScrollContentView",
                                            "Image", "RNSVGSvgView", "RNSVGSvgViewAndroid",
                                            "ViewManagerAdapter_ExpoBlur_ExpoBlurView",
                                            "ViewManagerAdapter_ExpoBlur_ExpoBlurTargetView", "RNCMaskedView",
                                            kWindowViewComponent};

// The input's text laid out as it is drawn: single-line inputs do not wrap, and none is cut at a
// number of lines or ellipsized (an EditText scrolls its text; `numberOfLines` is its height).
ParagraphLayout layoutInput(const AndroidTextInputProps& props, const AttributedString& text, float width) {
  float maxWidth = props.multiline ? width : std::numeric_limits<float>::infinity();
  auto paragraph = props.paragraphAttributes;
  paragraph.maximumNumberOfLines = 0;
  return TextEngine::shared().layout(text, paragraph, maxWidth);
}

// A secure field (`secureTextEntry`) shows one bullet per character, as Android's password
// fields do; its text is never drawn or reported.
constexpr const char* kSecureBullet = "\u2022";
constexpr float kCaretWidth = 1.5f;

bool isContinuation(const std::string& text, size_t i) {
  return (static_cast<unsigned char>(text[i]) & 0xc0) == 0x80;
}

std::string shownText(const AndroidTextInputProps& props, const std::string& value) {
  if (!props.secureTextEntry) return value;
  std::string shown;
  for (size_t i = 0; i < value.size(); ++i) {
    if (!isContinuation(value, i)) shown += kSecureBullet;
  }
  return shown;
}

// A byte offset into the value, as a byte offset into what is shown.
size_t shownOffset(const AndroidTextInputProps& props, const std::string& value, size_t offset) {
  if (!props.secureTextEntry) return offset;
  size_t characters = 0;
  for (size_t i = 0; i < std::min(offset, value.size()); ++i) {
    if (!isContinuation(value, i)) ++characters;
  }
  return characters * std::char_traits<char>::length(kSecureBullet);
}

AttributedString inputText(const AndroidTextInputProps& props, const std::string& text) {
  AttributedString string;
  string.setBaseTextAttributes(props.getEffectiveTextAttributes(1));
  if (!text.empty()) {
    string.appendFragment({.string = text, .textAttributes = string.getBaseTextAttributes(), .parentShadowView = {}});
  }
  return string;
}

// A text input's text, or its placeholder when empty. When it has focus (`edit`): the selection
// behind the text, the composition underlined, and the caret. A single-line input centres its
// line vertically, as Android's EditText does.
void drawTextInput(const ShadowView& view, float x, float y, float w, float h, Clip clip, float opacity,
                   const TextInputEdit& editing, Painter& painter) {
  const TextInputEdit* edit = view.tag == editing.tag ? &editing : nullptr;
  const auto& data = static_cast<const ConcreteState<AndroidTextInputState>&>(*view.state).getData();
  auto props = std::static_pointer_cast<const AndroidTextInputProps>(view.props);
  AttributedString text = data.attributedStringBox.getValue();
  std::string value = text.getString();
  if (props->secureTextEntry) text = inputText(*props, shownText(*props, value));
  if (text.isEmpty() && !props->placeholder.empty()) {
    auto attributes = props->getEffectiveTextAttributes(1);
    attributes.foregroundColor = props->placeholderTextColor ? props->placeholderTextColor
                                                             : colorFromComponents({0.5f, 0.5f, 0.5f, 1});
    text.appendFragment({.string = props->placeholder, .textAttributes = attributes, .parentShadowView = {}});
  }
  auto content = view.layoutMetrics.getContentFrame();
  auto layout = layoutInput(*props, text, content.size.width);
  // Text scrolled within the input is cut at its content box on the axis it scrolls, as Android's
  // TextView clips to its padding.
  Point scroll{};
  if (auto it = editing.scrolls.find(view.tag); it != editing.scrolls.end()) {
    scroll = TextInputGeometry(view, value).clamp(it->second, content.size);
  }
  float top = y + content.origin.y - scroll.y;
  if (!props->multiline) top += (content.size.height - layout.height) / 2;
  float left = x + content.origin.x - scroll.x;
  Clip inside = clip.intersect(Clip{x, y, x + w, y + h});
  if (scroll.x != 0 || scroll.y != 0 || props->multiline) {
    float boxLeft = x + content.origin.x, boxTop = y + content.origin.y;
    inside = props->multiline ? inside.intersect(Clip{x, boxTop, x + w, boxTop + content.size.height})
                              : inside.intersect(Clip{boxLeft, y, boxLeft + content.size.width, y + h});
  }
  SharedColor textColor = props->getEffectiveTextAttributes(1).foregroundColor;
  std::optional<TextInputGeometry> geometry;
  if (edit) geometry.emplace(view, value);
  // The geometry's positions are relative to the content box, before scrolling.
  float boxTop = y + content.origin.y - scroll.y;

  if (edit && edit->selectionStart != edit->selectionEnd) {
    Rgba selection = props->selectionColor ? rgba(props->selectionColor) : Rgba{0.25f, 0.45f, 1.0f, 0.3f};
    selection.a *= opacity;
    for (const auto& rect : geometry->span(edit->selectionStart, edit->selectionEnd)) {
      painter.fill(left + rect.origin.x, boxTop + rect.origin.y, rect.size.width, rect.size.height, BorderRadii{},
                   selection, inside);
    }
  }
  painter.glyphs(layout, left, top, inside, opacity);
  if (!edit) return;
  Rgba ink = textColor ? rgba(textColor) : Rgba{0, 0, 0, 1};
  ink.a *= opacity;
  if (edit->composeStart != edit->composeEnd) {
    // Underlined under each line it covers, just below the line's text.
    for (const auto& rect : geometry->span(edit->composeStart, edit->composeEnd)) {
      painter.fill(left + rect.origin.x, boxTop + rect.origin.y + rect.size.height - 1, rect.size.width, 1,
                   BorderRadii{}, ink, inside);
    }
    // The segment being converted: a heavier underline on a tint of the text's colour, drawn
    // over the text as Android's composing background is under it; an error, underlined in red.
    for (const auto& segment : edit->segments) {
      if (segment.style == OXYOS_PREEDIT_COMPOSING) continue;
      bool converting = segment.style == OXYOS_PREEDIT_CONVERTING;
      Rgba line = converting ? ink : Rgba{0.86f, 0.15f, 0.15f, opacity};
      Rgba tint{ink.r, ink.g, ink.b, ink.a * 0.15f};
      for (const auto& rect : geometry->span(segment.start, segment.end)) {
        if (converting) {
          painter.fill(left + rect.origin.x, boxTop + rect.origin.y, rect.size.width, rect.size.height, BorderRadii{},
                       tint, inside);
        }
        float thickness = converting ? 2 : 1;
        painter.fill(left + rect.origin.x, boxTop + rect.origin.y + rect.size.height - thickness, rect.size.width,
                     thickness, BorderRadii{}, line, inside);
      }
    }
  }
  if (edit->selectionStart == edit->selectionEnd) {
    auto caret = geometry->caret(edit->selectionEnd);
    Rgba color = props->cursorColor ? rgba(props->cursorColor) : ink;
    color.a *= opacity;
    painter.fill(left + caret.x, boxTop + caret.top, kCaretWidth, caret.height, BorderRadii{}, color, inside);
  }
}

// An Image view's picture within its content box, as React Native's resizeMode places it, rounded
// with the view's corners and tinted with its tintColor.
void drawImage(const ShadowView& view, const Picture& picture, float x, float y, float w, float h, Clip clip,
               float opacity, Painter& painter) {
  auto props = std::static_pointer_cast<const ImageProps>(view.props);
  if (props->blurRadius > 0) {
    static bool reported = false;
    if (!reported) std::cerr << "[host] unsupported on OxyOS: Image blurRadius (drawn unblurred)\n";
    reported = true;
  }
  auto content = view.layoutMetrics.getContentFrame();
  float cx = x + content.origin.x, cy = y + content.origin.y, cw = content.size.width, ch = content.size.height;
  if (cw <= 0 || ch <= 0) return;
  // The picture's own proportions, and its size at its scale (center, repeat, none).
  float aspect = static_cast<float>(picture.width) / std::max(1, picture.height);
  float nw = picture.naturalWidth > 0 ? picture.naturalWidth : picture.width;
  float nh = picture.naturalHeight > 0 ? picture.naturalHeight : picture.height;
  auto within = [&](float w0, float h0) {
    float shrink = std::min(1.0f, std::min(cw / w0, ch / h0));
    return std::pair{w0 * shrink, h0 * shrink};
  };
  Clip inside = clip.intersect(Clip{cx, cy, cx + cw, cy + ch});
  auto border = props->resolveBorderMetrics(view.layoutMetrics);
  std::optional<Rgba> tint;
  if (props->tintColor && *props->tintColor) tint = rgba(*props->tintColor);
  auto draw = [&](float dx, float dy, float dw, float dh) {
    painter.image(picture, dx, dy, dw, dh, inside, x, y, w, h, border.borderRadii, opacity, tint);
  };
  switch (props->resizeMode) {
    case ImageResizeMode::Stretch:
      return draw(cx, cy, cw, ch);
    case ImageResizeMode::Cover:
    case ImageResizeMode::Contain: {
      bool cover = props->resizeMode == ImageResizeMode::Cover;
      float dw = cw, dh = cw / aspect;
      if (cover ? dh < ch : dh > ch) dh = ch, dw = ch * aspect;
      return draw(cx + (cw - dw) / 2, cy + (ch - dh) / 2, dw, dh);
    }
    case ImageResizeMode::Center: {
      auto [dw, dh] = within(nw, nh);
      return draw(cx + (cw - dw) / 2, cy + (ch - dh) / 2, dw, dh);
    }
    case ImageResizeMode::None:
      return draw(cx, cy, nw, nh);
    case ImageResizeMode::Repeat: {
      auto [dw, dh] = within(nw, nh);
      for (float ty = cy; ty < cy + ch; ty += dh) {
        for (float tx = cx; tx < cx + cw; tx += dw) draw(tx, ty, dw, dh);
      }
      return;
    }
  }
}

// The extent of a view's drawing in its own coordinates: its frame, and its children's drawings
// unless it clips them.
Rect localBounds(const ViewTree& tree, Tag tag) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return {};
  const ShadowView& view = it->second.view;
  Rect area{{0, 0}, view.layoutMetrics.frame.size};
  std::string name = view.componentName ? view.componentName : "";
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  if (name == "ScrollView" || isSvgView(name) || (props && props->getClipsContentToBounds())) return area;
  for (Tag child : it->second.children) {
    auto found = tree.views().find(child);
    if (found == tree.views().end()) continue;
    std::string childName = found->second.view.componentName ? found->second.view.componentName : "";
    if (ownSurface(childName) || isModal(childName)) continue;
    area.unionInPlace(viewTransform(found->second.view).bounds(localBounds(tree, child)));
  }
  return area;
}

void drawView(const ViewTree& tree, Tag tag, float ox, float oy, Clip clip, float opacity, Painter& painter,
              RenderStats& stats, const TextInputEdit& edit, bool untransformed = false);

// A view whose transform scales, turns or skews it, as Android draws it through its matrix: its
// subtree drawn on its own, untransformed, at the resolution its largest scale needs (up to four
// times the display's), then mapped onto the canvas. Only the layer's pixels under the painter's
// region are drawn, with a margin for the sampling.
void drawTransformed(const ViewTree& tree, Tag tag, const Affine& map, Clip clip, float opacity, Painter& painter,
                     RenderStats& stats, const TextInputEdit& edit) {
  const ShadowView& view = tree.views().at(tag).view;
  Rect area = localBounds(tree, tag);
  float largest = std::max(std::hypot(map.a, map.b), std::hypot(map.c, map.d));
  float layerScale = painter.scale() * std::clamp(largest, 1.0f, 4.0f);
  while (area.size.width * layerScale * area.size.height * layerScale > 16e6f && layerScale > painter.scale()) {
    layerScale = std::max(painter.scale(), layerScale / 2);
  }
  Canvas layer;
  auto insidePainter = painter.child(layer, layerScale);
  Painter& inside = *insidePainter;
  // Moved and scaled, not turned or skewed: the windows in it are placed where it is drawn.
  inside.placesWindows = painter.placesWindows && map.b == 0 && map.c == 0 && map.a > 0 && map.d > 0;
  if (inside.placesWindows) {
    Rect bounds = painter.toSurface.bounds(Rect{{clip.x0, clip.y0}, {clip.x1 - clip.x0, clip.y1 - clip.y0}});
    inside.surfaceClip = painter.surfaceClip.intersect(
        Clip{bounds.origin.x, bounds.origin.y, bounds.getMaxX(), bounds.getMaxY()});
    inside.toSurface = painter.toSurface.after(map.after(Affine::translation(area.origin.x, area.origin.y)));
  }
  bool invertible = std::abs(map.determinant()) >= 1e-6f && area.size.width > 0 && area.size.height > 0;
  const auto& region = painter.region();
  // The canvas's painted region, in logical pixels, as the layer's own coordinates see it.
  Rect painted{{std::max(region[0], 0) / painter.scale(), std::max(region[1], 0) / painter.scale()}, {}};
  painted.size = {std::min(region[2], 1 << 24) / painter.scale() - painted.origin.x,
                  std::min(region[3], 1 << 24) / painter.scale() - painted.origin.y};
  Rect needed = invertible ? Rect::intersect(map.bounds(area), painted) : Rect{};
  if (needed.size.width > 0 && needed.size.height > 0) {
    inside.allocate(static_cast<int>(std::ceil(area.size.width * layerScale)),
                    static_cast<int>(std::ceil(area.size.height * layerScale)), 0);
    Rect source = map.inverse().bounds(needed);
    inside.limit(static_cast<int>(std::floor((source.origin.x - area.origin.x) * layerScale)) - 2,
                 static_cast<int>(std::floor((source.origin.y - area.origin.y) * layerScale)) - 2,
                 static_cast<int>(std::ceil((source.getMaxX() - area.origin.x) * layerScale)) + 2,
                 static_cast<int>(std::ceil((source.getMaxY() - area.origin.y) * layerScale)) + 2);
  } else {
    // Nothing of it is painted (or nothing of it shows): its subtree is still walked, for the stats.
    inside.limit(0, 0, 0, 0);
  }
  const auto& frame = view.layoutMetrics.frame;
  drawView(tree, tag, -frame.origin.x - area.origin.x, -frame.origin.y - area.origin.y,
           Clip{0, 0, area.size.width, area.size.height}, opacity, inside, stats, edit, true);
  if (layer.width > 0 && layer.height > 0) {
    painter.layer(layer, layerScale, area, map, clip);
  }
}

// The device pixels of a logical rectangle within `clip` that `painter` paints: [x0, x1) × [y0, y1).
struct PixelBox {
  int x0 = 0, y0 = 0, x1 = 0, y1 = 0;
  bool empty() const { return x0 >= x1 || y0 >= y1; }
};
PixelBox paintedPixels(const Painter& painter, float x, float y, float w, float h, const Clip& clip) {
  Clip visible = clip.intersect(Clip{x, y, x + w, y + h});
  if (visible.empty()) return {};
  const auto& region = painter.region();
  float scale = painter.scale();
  return PixelBox{std::max(static_cast<int>(std::floor(visible.x0 * scale)), region[0]),
                  std::max(static_cast<int>(std::floor(visible.y0 * scale)), region[1]),
                  std::min(static_cast<int>(std::ceil(visible.x1 * scale)), region[2]),
                  std::min(static_cast<int>(std::ceil(visible.y1 * scale)), region[3])};
}

// A canvas's premultiplied pixels as a picture (premultiplied R, G, B, A bytes), to draw with
// Painter::image.
Picture asPicture(const Canvas& canvas) {
  Picture picture;
  picture.width = canvas.width;
  picture.height = canvas.height;
  picture.pixels.resize(canvas.pixels.size() * 4);
  for (size_t i = 0; i < canvas.pixels.size(); ++i) {
    uint32_t pixel = canvas.pixels[i];
    picture.pixels[i * 4] = (pixel >> 16) & 0xff;
    picture.pixels[i * 4 + 1] = (pixel >> 8) & 0xff;
    picture.pixels[i * 4 + 2] = pixel & 0xff;
    picture.pixels[i * 4 + 3] = pixel >> 24;
  }
  return picture;
}

// Blurs `w` × `h` planes of levels with a Gaussian of standard deviation `sigma` pixels, as three
// box blurs of the sizes that approximate it (each pass a running sum, so the cost does not grow
// with the radius), along rows then columns, the edges extended.
void blurPlanes(std::vector<float> (&planes)[4], int w, int h, float sigma) {
  if (w <= 0 || h <= 0 || sigma < 0.5f) return;
  // The widths of three boxes whose succession has this standard deviation.
  float ideal = std::sqrt(12 * sigma * sigma / 3 + 1);
  int lower = static_cast<int>(std::floor(ideal));
  if (lower % 2 == 0) --lower;
  int upper = lower + 2;
  int lowerCount = static_cast<int>(
      std::round((12 * sigma * sigma - 3.0f * lower * lower - 12.0f * lower - 9) / (-4.0f * lower - 4)));
  std::vector<float> scratch(static_cast<size_t>(w) * h);
  // One box of half-width `r` along `count` lines of `length` samples `step` apart, the lines
  // `stride` apart: from `in` to `out`.
  auto box = [](const std::vector<float>& in, std::vector<float>& out, int r, int lines, int length, int stride,
                int step) {
    const float scale = 1.0f / (2 * r + 1);
    for (int line = 0; line < lines; ++line) {
      const size_t base = static_cast<size_t>(line) * stride;
      auto at = [&](int i) { return in[base + static_cast<size_t>(std::clamp(i, 0, length - 1)) * step]; };
      float sum = 0;
      for (int i = -r; i <= r; ++i) sum += at(i);
      for (int i = 0; i < length; ++i) {
        out[base + static_cast<size_t>(i) * step] = sum * scale;
        sum += at(i + r + 1) - at(i - r);
      }
    }
  };
  for (auto& plane : planes) {
    for (int pass = 0; pass < 3; ++pass) {
      int r = ((pass < lowerCount ? lower : upper) - 1) / 2;
      box(plane, scratch, r, h, w, w, 1);
      box(scratch, plane, r, w, h, 1, w);
    }
  }
}

// A canvas's premultiplied pixels as planes of levels (alpha, red, green, blue).
LevelImage levelsOf(const Canvas& canvas) {
  LevelImage image;
  image.width = canvas.width;
  image.height = canvas.height;
  for (int c = 0; c < 4; ++c) {
    image.planes[c].resize(canvas.pixels.size());
    for (size_t i = 0; i < canvas.pixels.size(); ++i) image.planes[c][i] = (canvas.pixels[i] >> (24 - 8 * c)) & 0xff;
  }
  return image;
}

// Blurs a canvas's premultiplied pixels with a Gaussian of standard deviation `sigma` device
// pixels (blurPlanes), rounded back to 8 bits.
void gaussianBlur(Canvas& canvas, float sigma) {
  if (canvas.width <= 0 || canvas.height <= 0 || sigma < 0.5f) return;
  LevelImage image = levelsOf(canvas);
  auto& planes = image.planes;
  blurPlanes(planes, canvas.width, canvas.height, sigma);
  for (size_t i = 0; i < canvas.pixels.size(); ++i) {
    uint32_t pixel = 0;
    for (int c = 0; c < 4; ++c) {
      pixel |= static_cast<uint32_t>(std::lround(std::clamp(planes[c][i], 0.0f, 255.0f))) << (24 - 8 * c);
    }
    canvas.pixels[i] = pixel;
  }
}

// ---- the CPU painter's canvases and composite operations ----------------------------------------

void CpuPainter::allocate(int width, int height, uint32_t value) {
  canvas_.width = width;
  canvas_.height = height;
  canvas_.gpu.reset();
  canvas_.pixels.assign(static_cast<size_t>(std::max(width, 0)) * std::max(height, 0), value);
}

void CpuPainter::resize(int width, int height, uint32_t value) {
  std::vector<uint32_t> pixels(static_cast<size_t>(width) * height, value);
  int rows = std::min(height, canvas_.height), columns = std::min(width, canvas_.width);
  for (int y = 0; y < rows; ++y) {
    std::copy_n(canvas_.pixels.begin() + static_cast<long>(y) * canvas_.width, columns,
                pixels.begin() + static_cast<long>(y) * width);
  }
  canvas_.pixels = std::move(pixels);
  canvas_.width = width;
  canvas_.height = height;
}

void CpuPainter::clear(int x0, int y0, int x1, int y1, uint32_t value) {
  for (int y = y0; y < y1; ++y) {
    std::fill_n(canvas_.pixels.begin() + static_cast<long>(y) * canvas_.width + x0, x1 - x0, value);
  }
}

std::unique_ptr<Painter> CpuPainter::child(Canvas& canvas, float scale) {
  return std::make_unique<CpuPainter>(canvas, scale);
}

void CpuPainter::imageOf(const Canvas& source, float dx, float dy, float dw, float dh, const Clip& clip, float bx,
                         float by, float bw, float bh, const BorderRadii& radii, float opacity) {
  image(asPicture(source), dx, dy, dw, dh, clip, bx, by, bw, bh, radii, opacity, std::nullopt);
}

void CpuPainter::smoothOf(const Canvas& source, float sigma, float dx, float dy, float dw, float dh,
                          const Clip& clip, float bx, float by, float bw, float bh, const BorderRadii& radii,
                          float opacity) {
  LevelImage levels = levelsOf(source);
  blurPlanes(levels.planes, source.width, source.height, sigma);
  smooth(levels, dx, dy, dw, dh, clip, bx, by, bw, bh, radii, opacity);
}

void CpuPainter::blur(float sigma) { gaussianBlur(canvas_, sigma); }

void CpuPainter::keepByAlphaOf(const Canvas& mask) {
  for (size_t i = 0; i < canvas_.pixels.size() && i < mask.pixels.size(); ++i) {
    uint32_t keep = mask.pixels[i] >> 24, pixel = canvas_.pixels[i];
    if (keep == 255) continue;
    uint32_t kept = 0;
    for (int shift = 0; shift < 32; shift += 8) {
      kept |= ((((pixel >> shift) & 0xff) * keep + 127) / 255) << shift;
    }
    canvas_.pixels[i] = kept;
  }
}

std::optional<std::array<int, 4>> CpuPainter::opaqueBounds() const {
  int width = canvas_.width, height = canvas_.height;
  int bx0 = width, by0 = height, bx1 = 0, by1 = 0;
  for (int py = 0; py < height; ++py) {
    const uint32_t* row = canvas_.pixels.data() + static_cast<size_t>(py) * width;
    for (int px = 0; px < width; ++px) {
      if (row[px] >> 24) {
        bx0 = std::min(bx0, px), bx1 = std::max(bx1, px + 1);
        by0 = std::min(by0, py), by1 = std::max(by1, py + 1);
      }
    }
  }
  return std::array<int, 4>{bx0, by0, bx1, by1};
}

// ---- the GPU painter ----------------------------------------------------------------------------

// The device pixels of a logical box within the painter's region and its canvas, as the CPU
// painter visits them.
struct DeviceBox {
  int x0, y0, x1, y1;
};

// Records the CPU painter's operations into oxyos-paint, each with the device pixels the CPU
// painter would visit for it; oxyos-paint works each pixel out with the CPU painter's arithmetic.
class GpuPainter final : public Painter {
 public:
  GpuPainter(Canvas& canvas, float scale, OxyOSPaint* paint) : Painter(scale), canvas_(canvas), paint_(paint) {}

  void allocate(int width, int height, uint32_t value);
  void resize(int width, int height, uint32_t value);
  void clear(int x0, int y0, int x1, int y1, uint32_t value);
  std::unique_ptr<Painter> child(Canvas& canvas, float scale) {
    return std::make_unique<GpuPainter>(canvas, scale, paint_);
  }

  void fill(float x, float y, float w, float h, const BorderRadii& radii, Rgba color, const Clip& clip,
            const BorderRadii* holeRadii = nullptr, float hx = 0, float hy = 0, float hw = 0, float hh = 0);
  void image(const Picture& picture, float dx, float dy, float dw, float dh, const Clip& clip, float bx, float by,
             float bw, float bh, const BorderRadii& radii, float opacity, const std::optional<Rgba>& tint);
  void layer(const Canvas& layer, float layerScale, const Rect& area, const Affine& map, const Clip& clip,
             float opacity = 1);
  void glyphs(const ParagraphLayout& layout, float ox, float oy, const Clip& clip, float opacity);
  void imageOf(const Canvas& source, float dx, float dy, float dw, float dh, const Clip& clip, float bx, float by,
               float bw, float bh, const BorderRadii& radii, float opacity);
  void smoothOf(const Canvas& source, float sigma, float dx, float dy, float dw, float dh, const Clip& clip,
                float bx, float by, float bw, float bh, const BorderRadii& radii, float opacity);
  void blur(float sigma) {
    if (uint32_t id = gpuCanvasId(canvas_)) oxyos_paint_blur(paint_, id, sigma);
  }
  void keepByAlphaOf(const Canvas& mask) {
    uint32_t content = gpuCanvasId(canvas_), kept = gpuCanvasId(mask);
    if (content && kept) oxyos_paint_mask(paint_, content, kept);
  }
  std::optional<std::array<int, 4>> opaqueBounds() const { return std::nullopt; }

 private:
  // The pixels of a logical box (within `clip`) the painter paints, clamped to the canvas.
  DeviceBox pixelsOf(const Clip& box) const {
    return DeviceBox{std::max({static_cast<int>(std::floor(box.x0 * scale_)), region_[0], 0}),
                     std::max({static_cast<int>(std::floor(box.y0 * scale_)), region_[1], 0}),
                     std::min({static_cast<int>(std::ceil(box.x1 * scale_)), region_[2], canvas_.width}),
                     std::min({static_cast<int>(std::ceil(box.y1 * scale_)), region_[3], canvas_.height})};
  }
  OxyOSPaintCommon common(const DeviceBox& box, const Clip& clip, Rgba color, float opacity) const {
    OxyOSPaintCommon common{};
    common.x0 = box.x0, common.y0 = box.y0, common.x1 = box.x1, common.y1 = box.y1;
    common.scale = scale_;
    common.clip[0] = clip.x0, common.clip[1] = clip.y0, common.clip[2] = clip.x1, common.clip[3] = clip.y1;
    common.color[0] = color.r, common.color[1] = color.g, common.color[2] = color.b, common.color[3] = color.a;
    common.opacity = opacity;
    return common;
  }
  static void radiiInto(const BorderRadii& radii, float out[4]) {
    out[0] = radii.topLeft.horizontal, out[1] = radii.topRight.horizontal;
    out[2] = radii.bottomLeft.horizontal, out[3] = radii.bottomRight.horizontal;
  }

  Canvas& canvas_;
  OxyOSPaint* paint_;
};

void GpuPainter::allocate(int width, int height, uint32_t value) {
  canvas_.width = width;
  canvas_.height = height;
  canvas_.pixels.clear();
  if (canvas_.gpu && canvas_.gpu->paint == paint_ && width > 0 && height > 0 &&
      oxyos_paint_canvas_resize(paint_, canvas_.gpu->id, width, height, value) == 0) {
    oxyos_paint_clear(paint_, canvas_.gpu->id, 0, 0, width, height, value);
    return;
  }
  canvas_.gpu.reset();
  if (width <= 0 || height <= 0) return;
  auto made = std::make_shared<GpuCanvas>();
  made->paint = paint_;
  made->id = oxyos_paint_canvas(paint_, static_cast<uint32_t>(width), static_cast<uint32_t>(height), value);
  if (made->id) canvas_.gpu = std::move(made);
}

void GpuPainter::resize(int width, int height, uint32_t value) {
  if (!canvas_.gpu) return allocate(width, height, value);
  oxyos_paint_canvas_resize(paint_, canvas_.gpu->id, width, height, value);
  canvas_.width = width;
  canvas_.height = height;
}

void GpuPainter::clear(int x0, int y0, int x1, int y1, uint32_t value) {
  if (uint32_t id = gpuCanvasId(canvas_)) oxyos_paint_clear(paint_, id, x0, y0, x1, y1, value);
}

void GpuPainter::fill(float x, float y, float w, float h, const BorderRadii& radii, Rgba color, const Clip& clip,
                      const BorderRadii* holeRadii, float hx, float hy, float hw, float hh) {
  uint32_t id = gpuCanvasId(canvas_);
  if (!id || color.a <= 0 || w <= 0 || h <= 0) return;
  Clip box = Clip{x, y, x + w, y + h}.intersect(clip);
  if (box.empty()) return;
  DeviceBox pixels = pixelsOf(box);
  if (pixels.x0 >= pixels.x1 || pixels.y0 >= pixels.y1) return;
  OxyOSPaintCommon given = common(pixels, clip, color, 1);
  OxyOSPaintFill fill{};
  fill.rect[0] = x, fill.rect[1] = y, fill.rect[2] = w, fill.rect[3] = h;
  radiiInto(radii, fill.radii);
  if (holeRadii) {
    fill.hole = true;
    fill.hole_rect[0] = hx, fill.hole_rect[1] = hy, fill.hole_rect[2] = hw, fill.hole_rect[3] = hh;
    radiiInto(*holeRadii, fill.hole_radii);
  }
  oxyos_paint_fill(paint_, id, &given, &fill);
}

void GpuPainter::image(const Picture& picture, float dx, float dy, float dw, float dh, const Clip& clip, float bx,
                       float by, float bw, float bh, const BorderRadii& radii, float opacity,
                       const std::optional<Rgba>& tint) {
  uint32_t id = gpuCanvasId(canvas_);
  if (!id || picture.width <= 0 || picture.height <= 0 || dw <= 0 || dh <= 0 || opacity <= 0) return;
  Clip box = Clip{dx, dy, dx + dw, dy + dh}.intersect(clip);
  if (box.empty()) return;
  DeviceBox pixels = pixelsOf(box);
  if (pixels.x0 >= pixels.x1 || pixels.y0 >= pixels.y1) return;
  if (oxyos_paint_picture(paint_, picture.id, static_cast<uint32_t>(picture.width),
                          static_cast<uint32_t>(picture.height), picture.pixels.data()) < 0) {
    return;
  }
  OxyOSPaintCommon given = common(pixels, clip, tint.value_or(Rgba{0, 0, 0, 0}), opacity);
  OxyOSPaintImage image{};
  image.picture = picture.id;
  image.rect[0] = dx, image.rect[1] = dy, image.rect[2] = dw, image.rect[3] = dh;
  image.bounds[0] = bx, image.bounds[1] = by, image.bounds[2] = bw, image.bounds[3] = bh;
  radiiInto(radii, image.radii);
  image.tinted = tint.has_value();
  oxyos_paint_image(paint_, id, &given, &image);
}

void GpuPainter::imageOf(const Canvas& source, float dx, float dy, float dw, float dh, const Clip& clip, float bx,
                         float by, float bw, float bh, const BorderRadii& radii, float opacity) {
  uint32_t id = gpuCanvasId(canvas_), from = gpuCanvasId(source);
  if (!id || !from || source.width <= 0 || source.height <= 0 || dw <= 0 || dh <= 0 || opacity <= 0) return;
  Clip box = Clip{dx, dy, dx + dw, dy + dh}.intersect(clip);
  if (box.empty()) return;
  DeviceBox pixels = pixelsOf(box);
  if (pixels.x0 >= pixels.x1 || pixels.y0 >= pixels.y1) return;
  OxyOSPaintCommon given = common(pixels, clip, Rgba{0, 0, 0, 0}, opacity);
  OxyOSPaintImage image{};
  image.canvas = from;
  image.rect[0] = dx, image.rect[1] = dy, image.rect[2] = dw, image.rect[3] = dh;
  image.bounds[0] = bx, image.bounds[1] = by, image.bounds[2] = bw, image.bounds[3] = bh;
  radiiInto(radii, image.radii);
  oxyos_paint_image(paint_, id, &given, &image);
}

void GpuPainter::smoothOf(const Canvas& source, float sigma, float dx, float dy, float dw, float dh,
                          const Clip& clip, float bx, float by, float bw, float bh, const BorderRadii& radii,
                          float opacity) {
  uint32_t id = gpuCanvasId(canvas_), from = gpuCanvasId(source);
  if (!id || !from || source.width <= 0 || source.height <= 0 || dw <= 0 || dh <= 0 || opacity <= 0) return;
  Clip box = Clip{dx, dy, dx + dw, dy + dh}.intersect(clip);
  if (box.empty()) return;
  DeviceBox pixels = pixelsOf(box);
  if (pixels.x0 >= pixels.x1 || pixels.y0 >= pixels.y1) return;
  OxyOSPaintCommon given = common(pixels, clip, Rgba{0, 0, 0, 0}, opacity);
  OxyOSPaintSmooth smooth{};
  smooth.source = from;
  smooth.sigma = sigma;
  smooth.rect[0] = dx, smooth.rect[1] = dy, smooth.rect[2] = dw, smooth.rect[3] = dh;
  smooth.bounds[0] = bx, smooth.bounds[1] = by, smooth.bounds[2] = bw, smooth.bounds[3] = bh;
  radiiInto(radii, smooth.radii);
  oxyos_paint_smooth(paint_, id, &given, &smooth);
}

void GpuPainter::layer(const Canvas& layer, float layerScale, const Rect& area, const Affine& map, const Clip& clip,
                       float opacity) {
  uint32_t id = gpuCanvasId(canvas_), from = gpuCanvasId(layer);
  if (!id || !from || layer.width <= 0 || layer.height <= 0 || std::abs(map.determinant()) < 1e-6f ||
      opacity <= 0) {
    return;
  }
  Rect drawn = map.bounds(area);
  Clip box = Clip{drawn.origin.x, drawn.origin.y, drawn.getMaxX(), drawn.getMaxY()}.intersect(clip);
  if (box.empty()) return;
  DeviceBox pixels = pixelsOf(box);
  if (pixels.x0 >= pixels.x1 || pixels.y0 >= pixels.y1) return;
  OxyOSPaintCommon given = common(pixels, clip, Rgba{0, 0, 0, 0}, opacity);
  Affine back = map.inverse();
  OxyOSPaintLayer drawnLayer{};
  drawnLayer.canvas = from;
  drawnLayer.layer_scale = layerScale;
  drawnLayer.origin[0] = area.origin.x, drawnLayer.origin[1] = area.origin.y;
  drawnLayer.inverse[0] = back.a, drawnLayer.inverse[1] = back.b, drawnLayer.inverse[2] = back.c;
  drawnLayer.inverse[3] = back.d, drawnLayer.inverse[4] = back.tx, drawnLayer.inverse[5] = back.ty;
  oxyos_paint_layer(paint_, id, &given, &drawnLayer);
}

void GpuPainter::glyphs(const ParagraphLayout& layout, float ox, float oy, const Clip& clip, float opacity) {
  uint32_t id = gpuCanvasId(canvas_);
  if (!id) return;
  // Each glyph's mask by its face, glyph and scale: uploaded once, under a number of its own.
  static std::map<std::tuple<void*, uint32_t, float>, uint64_t> keys;
  auto& engine = TextEngine::shared();
  std::vector<OxyOSPaintGlyph> run;
  for (const auto& laid : layout.runs) {
    Rgba color{((laid.color >> 16) & 0xff) / 255.0f, ((laid.color >> 8) & 0xff) / 255.0f, (laid.color & 0xff) / 255.0f,
               ((laid.color >> 24) & 0xff) / 255.0f * opacity};
    run.clear();
    int bx0 = std::numeric_limits<int>::max(), by0 = bx0, bx1 = std::numeric_limits<int>::min(), by1 = bx1;
    for (const auto& glyph : laid.glyphs) {
      const auto& bitmap = engine.rasterize(glyph.face, glyph.id, scale_);
      if (bitmap.width <= 0 || bitmap.height <= 0) continue;
      int gx = static_cast<int>(std::lround((ox + glyph.x) * scale_)) + bitmap.left;
      int gy = static_cast<int>(std::lround((oy + laid.baseline) * scale_)) - bitmap.top;
      if (gx + bitmap.width <= region_[0] || gx >= region_[2] || gy + bitmap.height <= region_[1] ||
          gy >= region_[3]) {
        continue;
      }
      auto [found, added] = keys.try_emplace({glyph.face, glyph.id, scale_}, keys.size() + 1);
      uint64_t key = found->second;
      if (!oxyos_paint_has_glyph(paint_, key)) {
        oxyos_paint_glyph(paint_, key, static_cast<uint32_t>(bitmap.width), static_cast<uint32_t>(bitmap.height),
                          bitmap.pixels.data());
      }
      run.push_back(OxyOSPaintGlyph{gx, gy, key});
      bx0 = std::min(bx0, gx), by0 = std::min(by0, gy);
      bx1 = std::max(bx1, gx + bitmap.width), by1 = std::max(by1, gy + bitmap.height);
    }
    if (run.empty() || color.a <= 0) continue;
    DeviceBox pixels{std::max({bx0, region_[0], 0}), std::max({by0, region_[1], 0}),
                     std::min({bx1, region_[2], canvas_.width}), std::min({by1, region_[3], canvas_.height})};
    if (pixels.x0 >= pixels.x1 || pixels.y0 >= pixels.y1) continue;
    OxyOSPaintCommon given = common(pixels, clip, color, 1);
    oxyos_paint_glyphs(paint_, id, &given, run.data(), run.size());
  }
}

// ---- which painter ------------------------------------------------------------------------------

OxyOSPaint* gpuPaintSet = nullptr;
thread_local int cpuOnly = 0;

OxyOSPaint* activePaint() { return cpuOnly ? nullptr : gpuPaintSet; }

// A canvas made ready for the painter about to draw it: one drawn on the GPU and now drawn on the
// CPU takes its pixels back (or is drawn whole again); one drawn on the CPU and now drawn on the
// GPU is drawn whole again.
void adopt(Canvas& canvas) {
  OxyOSPaint* paint = activePaint();
  if (paint && !canvas.gpu && canvas.width > 0) {
    canvas = Canvas{};
  } else if (!paint && canvas.gpu) {
    std::vector<uint32_t> pixels = canvasPixels(canvas);
    canvas.gpu.reset();
    if (pixels.size() == static_cast<size_t>(canvas.width) * canvas.height) {
      canvas.pixels = std::move(pixels);
    } else {
      canvas = Canvas{};
    }
  }
}

std::unique_ptr<Painter> makePainter(Canvas& canvas, float scale) {
  adopt(canvas);
  if (OxyOSPaint* paint = activePaint()) return std::make_unique<GpuPainter>(canvas, scale, paint);
  return std::make_unique<CpuPainter>(canvas, scale);
}

// Whether the blur view being drawn is drawing its target: a blur view inside that target draws
// its tint alone, as Dimezis' does, so a target never blurs into itself.
thread_local int drawingBlurTarget = 0;

// expo-blur's view with a blur target (expo-blur.h), as Dimezis' BlurView draws on Android: the
// target's content under the view, drawn afresh (never what the canvas holds, which outside the
// damage is the last frame, this view's blur included), blurred, then clipped to the view and its
// rounded corners. Its tint is drawn over it by the caller.
void drawBlurred(const ViewTree& tree, Tag blur, const ExpoBlurTarget& blurTarget, float x, float y, float w,
                 float h, const BorderRadii& radii, Clip clip, float opacity, Painter& painter,
                 const TextInputEdit& edit) {
  PixelBox painted = paintedPixels(painter, x, y, w, h, clip);
  if (painted.empty()) return;
  auto blurAt = windowFrame(tree, blur);
  auto targetAt = windowFrame(tree, blurTarget.target);
  auto target = tree.views().find(blurTarget.target);
  if (!blurAt || !targetAt || target == tree.views().end()) return;
  const float scale = painter.scale(), sigma = blurTarget.radius * scale;
  // A wide blur (16 device pixels of deviation or more: Launchpad's backdrop, 37 at 1.25) is
  // worked out on the target drawn reduced, as Dimezis' view draws it into a bitmap scaled down:
  // k times smaller (an eighth of the deviation, at most 8), blurred by what the Gaussian has left
  // to do at that size (drawing it reduced and stretching it back blur by about k²/4 of variance
  // between them), and stretched back over the view, bilinearly, at full precision and dithered
  // (Painter::smooth): stretched from 8 bits, its gradients would band. What a blur that wide keeps
  // is far coarser than k pixels, so it is within a level or two of the blur at full size, for a
  // k²th of the drawing and the blurring: Launchpad's backdrop at 1.25 took 230 ms, now 37 to 40,
  // most of it the stretch back.
  const int k = sigma >= 16 ? std::min(8, static_cast<int>(sigma / 8)) : 1;
  const float reducedScale = scale / k;
  const float reducedSigma = k > 1 ? std::sqrt(std::max(0.25f, sigma * sigma - k * k / 4.0f)) / k : sigma;
  // What the blur reads around a pixel: three standard deviations.
  const int margin = static_cast<int>(std::ceil(3 * reducedSigma));
  Canvas layer;
  auto insidePainter = painter.child(layer, reducedScale);
  Painter& inside = *insidePainter;
  inside.allocate((painted.x1 - painted.x0 + k - 1) / k + 2 * margin,
                  (painted.y1 - painted.y0 + k - 1) / k + 2 * margin, 0);
  const float left = static_cast<float>(painted.x0) / scale - margin / reducedScale;
  const float top = static_cast<float>(painted.y0) / scale - margin / reducedScale;
  // The target where it is relative to this view, in the layer's coordinates.
  Affine map = viewTransform(target->second.view);
  float targetX = x + (targetAt->origin.x - blurAt->origin.x) - map.tx - left;
  float targetY = y + (targetAt->origin.y - blurAt->origin.y) - map.ty - top;
  inside.placesWindows = false;
  RenderStats unseen;
  ++drawingBlurTarget;
  drawView(tree, blurTarget.target, targetX, targetY,
           Clip{-std::numeric_limits<float>::infinity(), -std::numeric_limits<float>::infinity(),
                std::numeric_limits<float>::infinity(), std::numeric_limits<float>::infinity()},
           1.0f, inside, unseen, edit);
  --drawingBlurTarget;
  if (k > 1) {
    // Kept in levels and stretched back bilinearly, dithered as it is written: quantized to 8 bits
    // at the reduced size, its bands a level apart would follow the reduced pixels' grid as steps.
    painter.smoothOf(layer, reducedSigma, left, top, layer.width / reducedScale, layer.height / reducedScale,
                     clip.intersect(Clip{x, y, x + w, y + h}), x, y, w, h, radii, opacity);
    return;
  }
  inside.blur(reducedSigma);
  painter.imageOf(layer, left, top, layer.width / reducedScale, layer.height / reducedScale,
                  clip.intersect(Clip{x, y, x + w, y + h}), x, y, w, h, radii, opacity);
}

// A MaskedView (masked-view.h): its children but the first drawn on their own, kept by the alpha
// of its first child drawn on its own, then drawn at the view's opacity.
void drawMasked(const ViewTree& tree, Tag tag, float x, float y, float w, float h, Clip clip, float opacity,
                Painter& painter, RenderStats& stats, const TextInputEdit& edit) {
  const auto& children = tree.views().at(tag).children;
  if (children.empty()) return;
  PixelBox painted = paintedPixels(painter, x, y, w, h, clip);
  const float scale = painter.scale();
  Canvas content, mask;
  int width = painted.empty() ? 0 : painted.x1 - painted.x0, height = painted.empty() ? 0 : painted.y1 - painted.y0;
  auto maskOwner = painter.child(mask, scale), contentOwner = painter.child(content, scale);
  Painter &maskPainter = *maskOwner, &contentPainter = *contentOwner;
  maskPainter.allocate(width, height, 0);
  contentPainter.allocate(width, height, 0);
  const float left = painted.empty() ? 0 : painted.x0 / scale, top = painted.empty() ? 0 : painted.y0 / scale;
  Clip inside{clip.x0 - left, clip.y0 - top, clip.x1 - left, clip.y1 - top};
  inside = inside.intersect(Clip{x - left, y - top, x - left + w, y - top + h});
  maskPainter.placesWindows = contentPainter.placesWindows = false;
  // Nothing of it painted: its subtree is still walked, for the stats.
  if (painted.empty()) {
    maskPainter.limit(0, 0, 0, 0);
    contentPainter.limit(0, 0, 0, 0);
  }
  drawView(tree, children.front(), x - left, y - top, inside, 1.0f, maskPainter, stats, edit);
  // Nothing of the children is kept where the mask is transparent: they are drawn within the
  // mask's opaque box alone (a blur under a clock's digits, not across the screen).
  // (On the GPU, where the mask's box is not read back, they are drawn whole and kept by it.)
  if (!painted.empty()) {
    if (auto box = maskPainter.opaqueBounds()) contentPainter.limit((*box)[0], (*box)[1], (*box)[2], (*box)[3]);
  }
  for (size_t i = 1; i < children.size(); ++i) {
    drawView(tree, children[i], x - left, y - top, inside, 1.0f, contentPainter, stats, edit);
  }
  if (painted.empty()) return;
  contentPainter.keepByAlphaOf(mask);
  painter.imageOf(content, left, top, width / scale, height / scale, clip, x, y, w, h, BorderRadii{}, opacity);
}

// What a view that clips its children (`overflow` other than visible, and a ScrollView) clips them
// to, as Android's BackgroundStyleApplicator.clipToPaddingBox: its padding box, inside its borders,
// with the inner edge of its rounded corners, which is where its border ring's hole is drawn.
struct ChildClip {
  float x, y, w, h;
  BorderRadii radii;
  bool rounded() const {
    return radii.topLeft.horizontal > 0 || radii.topRight.horizontal > 0 || radii.bottomLeft.horizontal > 0 ||
           radii.bottomRight.horizontal > 0;
  }
};

ChildClip paddingBox(const BorderMetrics& border, float x, float y, float w, float h) {
  const auto& widths = border.borderWidths;
  BorderRadii inner = border.borderRadii;
  inner.topLeft.horizontal = std::max(0.0f, inner.topLeft.horizontal - std::max(widths.left, widths.top));
  inner.topRight.horizontal = std::max(0.0f, inner.topRight.horizontal - std::max(widths.right, widths.top));
  inner.bottomLeft.horizontal = std::max(0.0f, inner.bottomLeft.horizontal - std::max(widths.left, widths.bottom));
  inner.bottomRight.horizontal = std::max(0.0f, inner.bottomRight.horizontal - std::max(widths.right, widths.bottom));
  return ChildClip{x + widths.left, y + widths.top, std::max(0.0f, w - widths.left - widths.right),
                   std::max(0.0f, h - widths.top - widths.bottom), inner};
}

// The device pixels of a logical rectangle within `clip`, whole pixels taken outward, before the
// painter's region limits them: [x0, x1) × [y0, y1).
PixelBox devicePixels(const Painter& painter, float x, float y, float w, float h, const Clip& clip) {
  Clip visible = clip.intersect(Clip{x, y, x + w, y + h});
  if (visible.empty()) return {};
  float scale = painter.scale();
  return PixelBox{static_cast<int>(std::floor(visible.x0 * scale)), static_cast<int>(std::floor(visible.y0 * scale)),
                  static_cast<int>(std::ceil(visible.x1 * scale)), static_cast<int>(std::ceil(visible.y1 * scale))};
}

// The device pixels of each corner's square of `box` (top left, top right, bottom left, bottom
// right) that the painter paints, whole pixels taken outward: the only pixels the rounded box does
// not cover whole (Painter::coveredWhole). Empty for a square corner, or where nothing is painted.
std::array<PixelBox, 4> cornerSquares(const ChildClip& box, const PixelBox& painted, float scale) {
  const float limit = std::min(box.w, box.h) / 2;
  const float tl = std::min(box.radii.topLeft.horizontal, limit), tr = std::min(box.radii.topRight.horizontal, limit);
  const float bl = std::min(box.radii.bottomLeft.horizontal, limit),
              br = std::min(box.radii.bottomRight.horizontal, limit);
  const float corners[4][3] = {{box.x, box.y, tl},
                               {box.x + box.w - tr, box.y, tr},
                               {box.x, box.y + box.h - bl, bl},
                               {box.x + box.w - br, box.y + box.h - br, br}};
  std::array<PixelBox, 4> squares{};
  for (int i = 0; i < 4; ++i) {
    const auto& [cx, cy, r] = corners[i];
    if (r <= 0) continue;
    PixelBox square{std::max(static_cast<int>(std::floor(cx * scale)), painted.x0),
                    std::max(static_cast<int>(std::floor(cy * scale)), painted.y0),
                    std::min(static_cast<int>(std::ceil((cx + r) * scale)), painted.x1),
                    std::min(static_cast<int>(std::ceil((cy + r) * scale)), painted.y1)};
    if (!square.empty()) squares[i] = square;
  }
  return squares;
}

// Whether some child drawn at (ox, oy) draws within the device pixels `square`.
bool childReaches(const ViewTree& tree, const std::vector<Tag>& children, float ox, float oy, const PixelBox& square,
                  float scale) {
  Rect area{{square.x0 / scale, square.y0 / scale},
            {(square.x1 - square.x0) / scale, (square.y1 - square.y0) / scale}};
  for (Tag child : children) {
    auto found = tree.views().find(child);
    if (found == tree.views().end()) continue;
    std::string name = found->second.view.componentName ? found->second.view.componentName : "";
    if (ownSurface(name) || isModal(name)) continue;
    Rect drawn = viewTransform(found->second.view).bounds(localBounds(tree, child));
    drawn.origin.x += ox, drawn.origin.y += oy;
    Rect common = Rect::intersect(drawn, area);
    if (common.size.width > 0 && common.size.height > 0) return true;
  }
  return false;
}

// The children drawn on their own over the device pixels `layerBox`, then drawn onto the canvas
// with the rounded box's coverage (Painter::image), within `clip`. The children are clipped in the
// layer to the box's whole pixels (`whole`), so the partial pixels at its edges hold them whole and
// are covered once, by the rounded box's coverage.
void drawThroughRoundedLayer(const ViewTree& tree, const std::vector<Tag>& children, float ox, float oy,
                             const Clip& clip, const ChildClip& box, const PixelBox& whole, const PixelBox& layerBox,
                             float opacity, Painter& painter, RenderStats& stats, const TextInputEdit& edit) {
  const float scale = painter.scale();
  // The layer is the painter's own kind (on the GPU, a device image), as a blur's or a transformed
  // view's is.
  Canvas content;
  auto insideOwner = painter.child(content, scale);
  Painter& inside = *insideOwner;
  inside.allocate(layerBox.x1 - layerBox.x0, layerBox.y1 - layerBox.y0, 0);
  const float left = layerBox.x0 / scale, top = layerBox.y0 / scale;
  // A window shown live inside is still placed, within the box's rectangle, with its own corners.
  inside.placesWindows = painter.placesWindows;
  inside.toSurface = painter.toSurface.after(Affine::translation(left, top));
  inside.surfaceClip = painter.surfaceClip;
  Clip pixels{whole.x0 / scale - left, whole.y0 / scale - top, whole.x1 / scale - left, whole.y1 / scale - top};
  for (Tag child : children) drawView(tree, child, ox - left, oy - top, pixels, opacity, inside, stats, edit);
  painter.imageOf(content, left, top, content.width / scale, content.height / scale, clip, box.x, box.y, box.w,
                  box.h, box.radii, 1.0f);
}

// The children of a view that clips them to a rounded box (`overflow: hidden` with a
// `borderRadius`), as Android clips them to its padding box's path, anti-aliased as a rounded
// background is (Painter::image's rounded coverage): Android's own anti-aliased clip, a layer kept
// by the path's mask (clipWithAntiAliasing). Pictures, transformed views and other rounded clips
// inside are drawn in the layer as anywhere else. Only the corners' squares need the layer: a box
// much larger than its corners (a window's content) is drawn directly, clipped to its rectangle,
// in the bands of rows and columns its corners leave, and through a layer in each corner a child
// reaches; a smaller one (a round picture) through one layer over all of it. A corner no child
// reaches needs nothing.
void drawRoundedClipped(const ViewTree& tree, const std::vector<Tag>& children, float ox, float oy, Clip clip,
                        const ChildClip& box, float opacity, Painter& painter, RenderStats& stats,
                        const TextInputEdit& edit) {
  Clip inner = clip.intersect(Clip{box.x, box.y, box.x + box.w, box.y + box.h});
  PixelBox whole = devicePixels(painter, box.x, box.y, box.w, box.h, clip);
  const std::array<int, 4> region = painter.region();
  PixelBox painted{std::max(whole.x0, region[0]), std::max(whole.y0, region[1]), std::min(whole.x1, region[2]),
                   std::min(whole.y1, region[3])};
  const float scale = painter.scale();
  auto direct = [&](RenderStats& into) {
    for (Tag child : children) drawView(tree, child, ox, oy, inner, opacity, painter, into, edit);
  };
  std::array<PixelBox, 4> squares = painted.empty() ? std::array<PixelBox, 4>{} : cornerSquares(box, painted, scale);
  std::array<bool, 4> cut{};
  long cornerArea = 0;
  bool overlapping = false;
  for (int i = 0; i < 4; ++i) {
    if (squares[i].empty()) continue;
    cut[i] = childReaches(tree, children, ox, oy, squares[i], scale);
    cornerArea += static_cast<long>(squares[i].x1 - squares[i].x0) * (squares[i].y1 - squares[i].y0);
    for (int j = 0; j < i; ++j) {
      const PixelBox& a = squares[i];
      const PixelBox& b = squares[j];
      if (!b.empty() && a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1) overlapping = true;
    }
  }
  if (std::none_of(cut.begin(), cut.end(), [](bool c) { return c; })) {
    direct(stats);
    return;
  }
  const long area = static_cast<long>(painted.x1 - painted.x0) * (painted.y1 - painted.y0);
  if (overlapping || area < 4 * cornerArea) {
    drawThroughRoundedLayer(tree, children, ox, oy, clip, box, whole, painted, opacity, painter, stats, edit);
    return;
  }
  // The bands: the painted rows cut where a corner's square begins or ends, each row band's
  // columns those the corners covering it leave. The views' texts and windows are counted once,
  // with the first band drawn (or a walk painting nothing, if none is).
  std::vector<int> breaks{painted.y0, painted.y1};
  for (const auto& square : squares) {
    if (!square.empty()) breaks.insert(breaks.end(), {square.y0, square.y1});
  }
  std::sort(breaks.begin(), breaks.end());
  breaks.erase(std::unique(breaks.begin(), breaks.end()), breaks.end());
  RenderStats again;
  bool counted = false;
  for (size_t b = 0; b + 1 < breaks.size(); ++b) {
    int y0 = breaks[b], y1 = breaks[b + 1], x0 = painted.x0, x1 = painted.x1;
    for (int i = 0; i < 4; ++i) {
      if (squares[i].empty() || squares[i].y0 > y0 || squares[i].y1 < y1) continue;
      if (i % 2 == 0) {
        x0 = std::max(x0, squares[i].x1);
      } else {
        x1 = std::min(x1, squares[i].x0);
      }
    }
    if (x0 >= x1) continue;
    painter.limit(x0, y0, x1, y1);
    direct(counted ? again : stats);
    counted = true;
  }
  if (!counted) {
    painter.limit(0, 0, 0, 0);
    direct(stats);
  }
  painter.limit(region[0], region[1], region[2], region[3]);
  for (int i = 0; i < 4; ++i) {
    if (!cut[i]) continue;
    drawThroughRoundedLayer(tree, children, ox, oy, clip, box, whole, squares[i], opacity, painter, again, edit);
  }
}

void drawView(const ViewTree& tree, Tag tag, float ox, float oy, Clip clip, float opacity, Painter& painter,
              RenderStats& stats, const TextInputEdit& edit, bool untransformed) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  const ShadowView& view = it->second.view;
  const auto& frame = view.layoutMetrics.frame;
  std::string name = view.componentName ? view.componentName : "";
  // A popup's or a shell region's subtree is drawn in its own surface, never in the window; a
  // modal's over everything else (drawModals).
  if (ownSurface(name) || isModal(name)) return;
  Affine map = untransformed ? Affine::translation(frame.origin.x, frame.origin.y) : viewTransform(view);
  if (!map.translates()) {
    drawTransformed(tree, tag, Affine::translation(ox, oy).after(map), clip, opacity, painter, stats, edit);
    return;
  }
  float x = ox + map.tx, y = oy + map.ty, w = frame.size.width, h = frame.size.height;
  ++stats.views;
  if (!kImplemented.count(name)) {
    ++stats.unimplemented;
    if (std::find(stats.unimplementedNames.begin(), stats.unimplementedNames.end(), name) ==
        stats.unimplementedNames.end()) {
      stats.unimplementedNames.push_back(name);
    }
  }

  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  // Debug aid: OXYOS_HOST_DUMP=1 prints every drawn view, indented by depth.
  static const bool dump = std::getenv("OXYOS_HOST_DUMP") != nullptr;
  static int depth = 0;
  struct Depth {
    bool entered;
    ~Depth() {
      if (entered) --depth;
    }
  } scope{dump};
  if (dump) {
    Rgba bg = props ? rgba(props->backgroundColor) : Rgba{0, 0, 0, 0};
    std::fprintf(stderr,
                 "[tree] %*s%s tag=%d frame=(%.0f,%.0f %.0fx%.0f) at=(%.0f,%.0f) bg=%.2f,%.2f,%.2f,%.2f opacity=%.2f "
                 "z=%d\n",
                 depth * 2, "", name.c_str(), tag, frame.origin.x, frame.origin.y, w, h, x, y, bg.r, bg.g, bg.b, bg.a,
                 props ? props->opacity : 1.0f, props && props->zIndex ? *props->zIndex : 0);
    ++depth;
  }
  Clip childClip = clip;
  std::optional<ChildClip> clipsTo;
  float childOx = x, childOy = y;
  if (props) {
    opacity *= props->opacity;
    if (opacity <= 0.001f) return;
    auto border = props->resolveBorderMetrics(view.layoutMetrics);
    Rgba background = rgba(props->backgroundColor);
    background.a *= opacity;
    painter.fill(x, y, w, h, border.borderRadii, background, clip);
    // expo-blur's view, as on Android: with a blur target, the target under it blurred; then its
    // tint (all there is without a target).
    if (auto blurTarget = expoBlurTarget(view); blurTarget && drawingBlurTarget == 0) {
      drawBlurred(tree, tag, *blurTarget, x, y, w, h, border.borderRadii, clip, opacity, painter, edit);
    }
    if (auto tint = expoBlurTint(view)) {
      Rgba color{((*tint >> 16) & 0xff) / 255.0f, ((*tint >> 8) & 0xff) / 255.0f, (*tint & 0xff) / 255.0f,
                 ((*tint >> 24) & 0xff) / 255.0f * opacity};
      painter.fill(x, y, w, h, border.borderRadii, color, clip);
    }
    const auto& widths = border.borderWidths;
    if (widths.left > 0 || widths.top > 0 || widths.right > 0 || widths.bottom > 0) {
      Rgba color = rgba(border.borderColors.top ? border.borderColors.top : border.borderColors.left);
      color.a *= opacity;
      BorderRadii inner = border.borderRadii;
      inner.topLeft.horizontal = std::max(0.0f, inner.topLeft.horizontal - std::max(widths.left, widths.top));
      inner.topRight.horizontal = std::max(0.0f, inner.topRight.horizontal - std::max(widths.right, widths.top));
      inner.bottomLeft.horizontal = std::max(0.0f, inner.bottomLeft.horizontal - std::max(widths.left, widths.bottom));
      inner.bottomRight.horizontal =
          std::max(0.0f, inner.bottomRight.horizontal - std::max(widths.right, widths.bottom));
      painter.fill(x, y, w, h, border.borderRadii, color, clip, &inner, x + widths.left, y + widths.top,
                   w - widths.left - widths.right, h - widths.top - widths.bottom);
    }
    if (props->getClipsContentToBounds() || name == "ScrollView") {
      clipsTo = paddingBox(border, x, y, w, h);
      childClip = clip.intersect(Clip{clipsTo->x, clipsTo->y, clipsTo->x + clipsTo->w, clipsTo->y + clipsTo->h});
    }
  }

  if (name == "ScrollView" && view.state) {
    const auto& state = static_cast<const ConcreteState<ScrollViewState>&>(*view.state);
    childOx -= state.getData().contentOffset.x;
    childOy -= state.getData().contentOffset.y;
  }

  bool visible = !clip.intersect(Clip{x, y, x + w, y + h}).empty() && opacity > 0.001f;
  if (name == "AndroidTextInput" && view.state) {
    if (painter.touches(x, y, w, h)) {
      drawTextInput(view, x, y, w, h, clip, opacity, edit, painter);
    }
    ++stats.texts;
    if (visible) {
      const auto& data = static_cast<const ConcreteState<AndroidTextInputState>&>(*view.state).getData();
      std::string value = data.attributedStringBox.getValue().getString();
      auto props = std::static_pointer_cast<const AndroidTextInputProps>(view.props);
      stats.textContent.push_back(value.empty() ? props->placeholder : shownText(*props, value));
    }
  }

  // A window shown live (docs/adr/0022-window-views.mdx): the compositor draws it here, over what
  // the surface shows, within what clips the view; the host draws nothing of it.
  if (name == kWindowViewComponent && painter.placesWindows && visible) {
    if (auto window = windowViewNumber(view)) {
      Clip inside = clip.intersect(Clip{x, y, x + w, y + h});
      auto border = props ? props->resolveBorderMetrics(view.layoutMetrics).borderRadii : BorderRadii{};
      float radius = std::max({border.topLeft.horizontal, border.topRight.horizontal, border.bottomLeft.horizontal,
                               border.bottomRight.horizontal});
      // In the surface's logical pixels.
      const Affine& map = painter.toSurface;
      Rect rect = map.bounds(Rect{{x, y}, {w, h}});
      Rect cut = map.bounds(Rect{{inside.x0, inside.y0}, {inside.x1 - inside.x0, inside.y1 - inside.y0}});
      Clip kept = painter.surfaceClip.intersect(Clip{cut.origin.x, cut.origin.y, cut.getMaxX(), cut.getMaxY()});
      if (!kept.empty()) {
        stats.windowViews.push_back(RenderStats::WindowView{
            *window, rect, Rect{{kept.x0, kept.y0}, {kept.x1 - kept.x0, kept.y1 - kept.y0}}, radius * map.a, opacity});
      }
    }
  }

  if (name == "Image" && it->second.picture && painter.touches(x, y, w, h)) {
    drawImage(view, *it->second.picture, x, y, w, h, clip, opacity, painter);
  }

  // An SVG view's elements are its picture (svg.h), drawn pixel for pixel over its content box.
  if (isSvgView(name)) {
    if (painter.touches(x, y, w, h)) {
      if (auto picture = svgPicture(tree, tag, painter.scale())) {
        auto content = view.layoutMetrics.getContentFrame();
        float pw = picture->width / painter.scale(), ph = picture->height / painter.scale();
        painter.image(*picture, x + content.origin.x, y + content.origin.y, pw, ph,
                      clip.intersect(Clip{x, y, x + w, y + h}), x, y, w, h, BorderRadii{}, opacity, std::nullopt);
      }
    }
    return;
  }

  if (name == "Paragraph" && view.state) {
    const auto& state = static_cast<const ConcreteState<ParagraphState>&>(*view.state).getData();
    // Glyphs stay within the paragraph's frame (a word wider than the paragraph is cut, not
    // painted over its neighbours), as a view's content is clipped to its bounds on Android: a
    // paragraph outside the damage is not laid out.
    if (painter.touches(x, y, w, h)) {
      auto content = view.layoutMetrics.getContentFrame();
      auto layout = TextEngine::shared().layout(state.attributedString, state.paragraphAttributes, content.size.width);
      painter.glyphs(layout, x + content.origin.x, y + content.origin.y, clip.intersect(Clip{x, y, x + w, y + h}),
                     opacity);
    }
    ++stats.texts;
    if (visible) stats.textContent.push_back(state.attributedString.getString());
  }

  if (isMaskedView(name)) {
    drawMasked(tree, tag, x, y, w, h, clip, opacity, painter, stats, edit);
    return;
  }

  if (clipsTo && clipsTo->rounded()) {
    drawRoundedClipped(tree, it->second.children, childOx, childOy, clip, *clipsTo, opacity, painter, stats, edit);
    return;
  }
  for (Tag child : it->second.children) {
    drawView(tree, child, childOx, childOy, childClip, opacity, painter, stats, edit);
  }
}

uint32_t crc32(const uint8_t* data, size_t size, uint32_t crc = 0xffffffff) {
  for (size_t i = 0; i < size; ++i) {
    crc ^= data[i];
    for (int k = 0; k < 8; ++k) crc = crc & 1 ? (crc >> 1) ^ 0xedb88320 : crc >> 1;
  }
  return crc;
}

void be32(std::vector<uint8_t>& out, uint32_t v) {
  for (int s = 24; s >= 0; s -= 8) out.push_back(static_cast<uint8_t>(v >> s));
}

void chunk(std::vector<uint8_t>& out, const char* kind, const std::vector<uint8_t>& data) {
  be32(out, static_cast<uint32_t>(data.size()));
  size_t start = out.size();
  out.insert(out.end(), kind, kind + 4);
  out.insert(out.end(), data.begin(), data.end());
  be32(out, crc32(out.data() + start, out.size() - start) ^ 0xffffffff);
}

}  // namespace

TextInputGeometry::TextInputGeometry(const ShadowView& view, const std::string& value)
    : props_(std::static_pointer_cast<const AndroidTextInputProps>(view.props)),
      value_(value),
      shown_(shownText(*props_, value)) {
  auto content = view.layoutMetrics.getContentFrame();
  // An empty input still has a line, as tall as its text would be, where its caret goes.
  auto layout = layoutInput(*props_, inputText(*props_, shown_.empty() ? " " : shown_), content.size.width);
  float top = props_->multiline ? 0 : (content.size.height - layout.height) / 2;
  for (size_t i = 0; i < layout.lines.size(); ++i) {
    const auto& line = layout.lines[i];
    Line geometry{.start = shown_.empty() ? 0 : line.start, .x = line.x, .top = top + line.top, .height = line.height};
    if (!shown_.empty()) geometry.stops = line.stops;
    if (shown_.empty()) {
      geometry.end = 0;
    } else if (i + 1 < layout.lines.size()) {
      size_t next = layout.lines[i + 1].start;
      geometry.broken = next > 0 && shown_[next - 1] == '\n';
      geometry.end = geometry.broken ? next - 1 : next;
    } else {
      geometry.end = std::min(line.start + line.text.size(), shown_.size());
    }
    lines_.push_back(geometry);
  }
  if (lines_.empty()) lines_.push_back(Line{.top = top});
}

size_t TextInputGeometry::toShown(size_t offset) const { return shownOffset(*props_, value_, offset); }

size_t TextInputGeometry::toValue(size_t shownOffset) const {
  if (!props_->secureTextEntry) return std::min(shownOffset, value_.size());
  size_t characters = shownOffset / std::char_traits<char>::length(kSecureBullet);
  size_t i = 0;
  for (; i < value_.size() && characters > 0; ++i) {
    if (!isContinuation(value_, i)) --characters;
    while (i + 1 < value_.size() && isContinuation(value_, i + 1)) ++i;
  }
  return i;
}

float TextInputGeometry::xOn(const Line& line, size_t shownOffset) const {
  // From the shaped line's caret stops; inside a cluster of several characters (a ligature), its
  // width is shared among them.
  if (shownOffset <= line.start || line.stops.empty()) return line.x;
  auto next = std::upper_bound(line.stops.begin(), line.stops.end(), shownOffset,
                               [](size_t at, const auto& stop) { return at < stop.first; });
  if (next == line.stops.begin()) return line.x;
  auto stop = std::prev(next);
  if (stop->first == shownOffset || next == line.stops.end()) return stop->second;
  auto characters = [&](size_t from, size_t to) {
    size_t count = 0;
    for (size_t i = from; i < to && i < shown_.size(); ++i) count += !isContinuation(shown_, i);
    return count;
  };
  float share = static_cast<float>(characters(stop->first, shownOffset)) /
                static_cast<float>(std::max<size_t>(1, characters(stop->first, next->first)));
  return stop->second + (next->second - stop->second) * share;
}

TextInputGeometry::Caret TextInputGeometry::caret(size_t offset) const {
  size_t shown = toShown(offset);
  size_t index = 0;
  while (index + 1 < lines_.size() && lines_[index + 1].start <= shown) ++index;
  const auto& line = lines_[index];
  return Caret{.line = index, .x = xOn(line, std::min(shown, std::max(line.end, line.start))), .top = line.top,
               .height = line.height};
}

std::pair<size_t, size_t> TextInputGeometry::shownEdges(size_t index) const {
  const auto& line = lines_[std::min(index, lines_.size() - 1)];
  bool wrapped = index + 1 < lines_.size() && !line.broken;
  size_t last = line.end;
  if (wrapped && last > line.start) {
    do --last;
    while (last > line.start && isContinuation(shown_, last));
  }
  return {line.start, last};
}

std::pair<size_t, size_t> TextInputGeometry::lineEdges(size_t index) const {
  auto [first, last] = shownEdges(index);
  return {toValue(first), toValue(last)};
}

size_t TextInputGeometry::offsetOnLine(size_t index, float x) const {
  const auto& line = lines_[std::min(index, lines_.size() - 1)];
  auto [first, last] = shownEdges(index);
  size_t best = first;
  float bestDistance = std::abs(x - xOn(line, first));
  for (size_t i = first + 1; i <= last; ++i) {
    if (i < shown_.size() && isContinuation(shown_, i)) continue;
    float distance = std::abs(x - xOn(line, i));
    if (distance < bestDistance) {
      best = i;
      bestDistance = distance;
    }
  }
  return toValue(best);
}

size_t TextInputGeometry::offsetAt(float x, float y) const {
  size_t index = 0;
  while (index + 1 < lines_.size() && y >= lines_[index + 1].top) ++index;
  return offsetOnLine(index, x);
}

float TextInputGeometry::width() const {
  float width = 0;
  for (const auto& line : lines_) width = std::max(width, xOn(line, line.end));
  return width;
}

float TextInputGeometry::height() const { return lines_.back().top + lines_.back().height; }

Point TextInputGeometry::clamp(Point scroll, Size size) const {
  bool multiline = props_->multiline;
  float x = multiline ? 0 : std::clamp(scroll.x, 0.0f, std::max(0.0f, width() + kCaretWidth - size.width));
  float y = multiline ? std::clamp(scroll.y, 0.0f, std::max(0.0f, height() - size.height)) : 0;
  return Point{x, y};
}

std::vector<Rect> TextInputGeometry::span(size_t from, size_t to) const {
  std::vector<Rect> rects;
  size_t start = toShown(from), end = toShown(to);
  for (size_t i = 0; i < lines_.size(); ++i) {
    const auto& line = lines_[i];
    size_t lineEnd = i + 1 < lines_.size() ? lines_[i + 1].start : shown_.size();
    if (end <= line.start || start >= lineEnd) continue;
    float left = xOn(line, std::max(start, line.start));
    float right = xOn(line, std::min(end, line.end));
    if (line.broken && end > line.end) right += line.height / 4;
    if (right > left) rects.push_back(Rect{Point{left, line.top}, Size{right - left, line.height}});
  }
  return rects;
}

namespace {

void collectModals(const ViewTree& tree, Tag tag, std::vector<Tag>& out) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  std::string name = it->second.view.componentName ? it->second.view.componentName : "";
  if (ownSurface(name)) return;
  if (isModal(name)) {
    auto props = std::dynamic_pointer_cast<const ModalHostViewProps>(it->second.view.props);
    if (props && props->visible) out.push_back(tag);
  }
  for (Tag child : it->second.children) collectModals(tree, child, out);
}

// The modals shown, over the window's content: each its children at the window's origin, an
// opaque one (not `transparent`) on white, as Android's dialog window for it.
void drawModalOnto(const ViewTree& tree, Tag modal, const Display& display, Painter& painter, RenderStats& stats,
                   const TextInputEdit& edit) {
  Clip window{0, 0, display.width, display.height};
  const auto& entry = tree.views().at(modal);
  auto props = std::static_pointer_cast<const ModalHostViewProps>(entry.view.props);
  if (!props->transparent) painter.fill(0, 0, display.width, display.height, {}, Rgba{1, 1, 1, 1}, window);
  for (Tag child : entry.children) drawView(tree, child, 0, 0, window, 1.0f, painter, stats, edit);
}

// The modals over the window's content; one animating in is drawn as its window is, moved and
// faded, and those animating out after them, from their last frames.
void drawModals(const ViewTree& tree, const Display& display, Painter& painter, RenderStats& stats,
                const TextInputEdit& edit) {
  Clip window{0, 0, display.width, display.height};
  Rect area{{0, 0}, {display.width, display.height}};
  for (Tag modal : modals(tree)) {
    ModalPresentation presented = tree.presentation(modal);
    if (presented == ModalPresentation{}) {
      drawModalOnto(tree, modal, display, painter, stats, edit);
      continue;
    }
    Canvas layer;
    auto insidePainter = painter.child(layer, display.scale);
    Painter& inside = *insidePainter;
    inside.allocate(display.pixelWidth(), display.pixelHeight(), 0);
    inside.placesWindows = false;
    drawModalOnto(tree, modal, display, inside, stats, edit);
    painter.layer(layer, display.scale, area, Affine::translation(0, presented.offset), window, presented.alpha);
  }
  for (const auto& leaving : tree.leaving()) {
    if (!leaving.frame || leaving.frame->width <= 0) continue;
    painter.layer(*leaving.frame, leaving.frame->width / display.width, area,
                  Affine::translation(0, leaving.presentation.offset), window, leaving.presentation.alpha);
  }
}

}  // namespace

std::vector<Tag> modals(const ViewTree& tree) {
  std::vector<Tag> out;
  if (tree.root() >= 0) collectModals(tree, tree.root(), out);
  return out;
}

RenderStats drawModal(const ViewTree& tree, Tag modal, const Display& display, Canvas& canvas,
                      const TextInputEdit& edit) {
  auto owner = makePainter(canvas, display.scale);
  Painter& painter = *owner;
  painter.allocate(display.pixelWidth(), display.pixelHeight(), 0);
  RenderStats stats;
  if (!tree.views().count(modal)) return stats;
  drawModalOnto(tree, modal, display, painter, stats, edit);
  return stats;
}

Rect Affine::bounds(const Rect& rect) const {
  return Rect::boundingRect(apply(rect.origin), apply(Point{rect.getMaxX(), rect.getMinY()}),
                            apply(Point{rect.getMinX(), rect.getMaxY()}), apply(Point{rect.getMaxX(), rect.getMaxY()}));
}

Affine viewTransform(const ShadowView& view) {
  const auto& origin = view.layoutMetrics.frame.origin;
  static const Transform identity = Transform::Identity();
  const auto* props = dynamic_cast<const ViewProps*>(view.props.get());
  if (!props || props->transform == identity) return Affine::translation(origin.x, origin.y);
  const auto& m = props->resolveTransform(view.layoutMetrics).matrix;
  // Column-major, as CSS: x' = m0 x + m4 y + m12, y' = m1 x + m5 y + m13, about the view's centre,
  // as Android's pivot is by default (resolveTransform puts a transform origin in the matrix, as an
  // offset from the centre).
  if (m[3] != 0 || m[7] != 0 || m[11] != 0) {
    static bool reported = false;
    if (!reported) {
      reported = true;
      std::cerr << "[host] transforms: perspective is not drawn on OxyOS yet (a 3D transform is drawn as its "
                   "projection onto the screen)\n";
    }
  }
  const auto& size = view.layoutMetrics.frame.size;
  return Affine::translation(origin.x + size.width / 2, origin.y + size.height / 2)
      .after(Affine{static_cast<float>(m[0]), static_cast<float>(m[1]), static_cast<float>(m[4]),
                    static_cast<float>(m[5]), static_cast<float>(m[12]), static_cast<float>(m[13])})
      .after(Affine::translation(-size.width / 2, -size.height / 2));
}

Point transformOffset(const ShadowView& view) {
  const auto& frame = view.layoutMetrics.frame;
  Point centre{frame.size.width / 2, frame.size.height / 2};
  Point moved = viewTransform(view).apply(centre);
  return Point{moved.x - frame.origin.x - centre.x, moved.y - frame.origin.y - centre.y};
}

RenderStats draw(const ViewTree& tree, const Display& display, Canvas& canvas, const TextInputEdit& edit) {
  auto owner = makePainter(canvas, display.scale);
  Painter& painter = *owner;
  painter.allocate(display.pixelWidth(), display.pixelHeight(), display.background);
  RenderStats stats;
  drawView(tree, tree.root(), 0, 0, Clip{0, 0, display.width, display.height}, 1.0f, painter, stats, edit);
  drawModals(tree, display, painter, stats, edit);
  return stats;
}

bool drawsTheSame(const ShadowView& before, const ShadowView& after) {
  if (before.props == after.props) return true;
  std::string name = after.componentName ? after.componentName : "";
  // A text input's drawing and a blur view's tint read props of their own.
  if ((before.componentName ? before.componentName : "") != name || name == "AndroidTextInput" || isExpoBlurView(name)) {
    return false;
  }
  auto a = std::dynamic_pointer_cast<const ViewProps>(before.props);
  auto b = std::dynamic_pointer_cast<const ViewProps>(after.props);
  if (!a || !b) return !a && !b;
  // What drawView reads from a view's props, each with its own layout.
  return a->opacity == b->opacity && a->backgroundColor == b->backgroundColor &&
         a->resolveBorderMetrics(before.layoutMetrics) == b->resolveBorderMetrics(after.layoutMetrics) &&
         a->getClipsContentToBounds() == b->getClipsContentToBounds() && a->transform == b->transform &&
         a->transformOrigin == b->transformOrigin;
}

bool drawsFlat(const ShadowView& view) {
  std::string name = view.componentName ? view.componentName : "";
  if (name != "View" && name != "RootView") return false;
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  if (!props) return true;
  if (!(props->transform == Transform::Identity())) return false;
  auto border = props->resolveBorderMetrics(view.layoutMetrics);
  const auto& r = border.borderRadii;
  const auto& w = border.borderWidths;
  for (const auto& corner : {r.topLeft, r.topRight, r.bottomLeft, r.bottomRight}) {
    if (corner.horizontal > 0 || corner.vertical > 0) return false;
  }
  return w.left <= 0 && w.top <= 0 && w.right <= 0 && w.bottom <= 0;
}

RenderStats drawDamaged(const ViewTree& tree, const Display& display, Canvas& canvas, const TextInputEdit& edit,
                        const std::vector<Rect>& damage) {
  int width = display.pixelWidth();
  int height = display.pixelHeight();
  adopt(canvas);
  if (canvas.width != width || canvas.height != height) {
    if (canvas.width <= 0 || canvas.height <= 0) return draw(tree, display, canvas, edit);
    // The window resized: its previous frame stays where the new one overlaps it, anchored at the
    // top left as the window's content is laid out; the rest is new, and in the damage.
    makePainter(canvas, display.scale)->resize(width, height, 0xffffffffu);
  }
  RenderStats stats;
  bool counted = false;
  Clip window{0, 0, display.width, display.height};
  for (const auto& rect : damage) {
    int x0 = std::max(0, static_cast<int>(std::floor(rect.origin.x * display.scale)));
    int y0 = std::max(0, static_cast<int>(std::floor(rect.origin.y * display.scale)));
    int x1 = std::min(width, static_cast<int>(std::ceil((rect.origin.x + rect.size.width) * display.scale)));
    int y1 = std::min(height, static_cast<int>(std::ceil((rect.origin.y + rect.size.height) * display.scale)));
    if (x1 <= x0 || y1 <= y0) continue;
    // The region starts as a new window does, then everything over it is drawn again.
    auto owner = makePainter(canvas, display.scale);
    Painter& painter = *owner;
    painter.clear(x0, y0, x1, y1, display.background);
    painter.limit(x0, y0, x1, y1);
    RenderStats drawn;
    drawView(tree, tree.root(), 0, 0, window, 1.0f, painter, drawn, edit);
    drawModals(tree, display, painter, drawn, edit);
    if (!counted) {
      stats = std::move(drawn);
      counted = true;
    }
  }
  if (!counted) {
    // Nothing to paint; the stats are still the window's.
    auto owner = makePainter(canvas, display.scale);
    Painter& painter = *owner;
    painter.limit(0, 0, 0, 0);
    drawView(tree, tree.root(), 0, 0, window, 1.0f, painter, stats, edit);
    drawModals(tree, display, painter, stats, edit);
  }
  return stats;
}

RenderStats drawPopup(const ViewTree& tree, Tag popup, const Display& display, Canvas& canvas,
                      const TextInputEdit& edit) {
  return drawRegion(tree, popup, Point{0, 0}, display, canvas, edit);
}

RenderStats drawRegion(const ViewTree& tree, Tag region, Point at, const Display& display, Canvas& canvas,
                       const TextInputEdit& edit) {
  auto owner = makePainter(canvas, display.scale);
  Painter& painter = *owner;
  painter.allocate(display.pixelWidth(), display.pixelHeight(), 0);
  RenderStats stats;
  auto it = tree.views().find(region);
  if (it == tree.views().end()) return stats;
  for (Tag child : it->second.children) {
    drawView(tree, child, at.x, at.y, Clip{0, 0, display.width, display.height}, 1.0f, painter, stats, edit);
  }
  return stats;
}

RenderStats drawRegionDamaged(const ViewTree& tree, Tag region, Point at, const Display& display, Canvas& canvas,
                              const TextInputEdit& edit, const std::vector<Rect>& damage) {
  int width = display.pixelWidth();
  int height = display.pixelHeight();
  adopt(canvas);
  if (canvas.width != width || canvas.height != height) return drawRegion(tree, region, at, display, canvas, edit);
  auto it = tree.views().find(region);
  if (it == tree.views().end()) return {};
  Clip surface{0, 0, display.width, display.height};
  auto drawChildren = [&](Painter& painter, RenderStats& stats) {
    for (Tag child : it->second.children) drawView(tree, child, at.x, at.y, surface, 1.0f, painter, stats, edit);
  };
  RenderStats stats;
  bool counted = false;
  for (const auto& rect : damage) {
    int x0 = std::max(0, static_cast<int>(std::floor(rect.origin.x * display.scale)));
    int y0 = std::max(0, static_cast<int>(std::floor(rect.origin.y * display.scale)));
    int x1 = std::min(width, static_cast<int>(std::ceil((rect.origin.x + rect.size.width) * display.scale)));
    int y1 = std::min(height, static_cast<int>(std::ceil((rect.origin.y + rect.size.height) * display.scale)));
    if (x1 <= x0 || y1 <= y0) continue;
    // The region starts transparent, as a new region does, then everything over it is drawn again.
    auto owner = makePainter(canvas, display.scale);
    Painter& painter = *owner;
    painter.clear(x0, y0, x1, y1, 0u);
    painter.limit(x0, y0, x1, y1);
    RenderStats drawn;
    drawChildren(painter, drawn);
    if (!counted) {
      stats = std::move(drawn);
      counted = true;
    }
  }
  if (!counted) {
    // Nothing to paint; the stats are still the region's.
    auto owner = makePainter(canvas, display.scale);
    Painter& painter = *owner;
    painter.limit(0, 0, 0, 0);
    drawChildren(painter, stats);
  }
  return stats;
}

std::vector<Tag> ancestry(const ViewTree& tree, Tag tag) { return tree.path(tag); }

std::optional<Rect> windowFrame(const ViewTree& tree, Tag tag) {
  auto path = tree.path(tag);
  if (path.empty()) return std::nullopt;
  // From the root down, as the views are drawn: each one where its parent puts its children.
  Point offset{0, 0};
  for (size_t i = 0;; ++i) {
    const auto& view = tree.views().at(path[i]).view;
    Point moved = transformOffset(view);
    Point position{offset.x + view.layoutMetrics.frame.origin.x + moved.x,
                   offset.y + view.layoutMetrics.frame.origin.y + moved.y};
    // A modal is at the window's origin, wherever it is in the tree.
    if (isModal(view.componentName ? view.componentName : "")) position = Point{0, 0};
    if (i + 1 == path.size()) return Rect{position, view.layoutMetrics.frame.size};
    offset = position;
    if (std::strcmp(view.componentName ? view.componentName : "", "ScrollView") == 0 && view.state) {
      const auto& state = static_cast<const ConcreteState<ScrollViewState>&>(*view.state).getData();
      offset.x -= state.contentOffset.x;
      offset.y -= state.contentOffset.y;
    }
  }
}

bool writePng(const Canvas& drawn, const std::string& path) {
  Canvas canvas{drawn.width, drawn.height, canvasPixels(drawn), nullptr};
  std::vector<uint8_t> raw;
  raw.reserve(static_cast<size_t>(canvas.width * 3 + 1) * canvas.height);
  for (int y = 0; y < canvas.height; ++y) {
    raw.push_back(0);
    for (int x = 0; x < canvas.width; ++x) {
      uint32_t p = canvas.pixels[static_cast<size_t>(y) * canvas.width + x];
      raw.push_back(static_cast<uint8_t>(p >> 16));
      raw.push_back(static_cast<uint8_t>(p >> 8));
      raw.push_back(static_cast<uint8_t>(p));
    }
  }
  std::vector<uint8_t> zlib = {0x78, 0x01};
  for (size_t offset = 0; offset < raw.size(); offset += 65535) {
    size_t n = std::min<size_t>(65535, raw.size() - offset);
    zlib.push_back(offset + n == raw.size());
    zlib.push_back(static_cast<uint8_t>(n));
    zlib.push_back(static_cast<uint8_t>(n >> 8));
    zlib.push_back(static_cast<uint8_t>(~n));
    zlib.push_back(static_cast<uint8_t>(~n >> 8));
    zlib.insert(zlib.end(), raw.begin() + static_cast<long>(offset), raw.begin() + static_cast<long>(offset + n));
  }
  uint32_t a = 1, b = 0;
  for (uint8_t byte : raw) {
    a = (a + byte) % 65521;
    b = (b + a) % 65521;
  }
  be32(zlib, b << 16 | a);
  std::vector<uint8_t> png = {0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n'};
  std::vector<uint8_t> header;
  be32(header, static_cast<uint32_t>(canvas.width));
  be32(header, static_cast<uint32_t>(canvas.height));
  header.insert(header.end(), {8, 2, 0, 0, 0});
  chunk(png, "IHDR", header);
  chunk(png, "IDAT", zlib);
  chunk(png, "IEND", {});
  std::ofstream file(path, std::ios::binary);
  file.write(reinterpret_cast<const char*>(png.data()), static_cast<long>(png.size()));
  file.close();
  return !file.fail();
}

RenderStats render(const ViewTree& tree, const Display& display, const std::string& pngPath) {
  Canvas canvas;
  auto stats = draw(tree, display, canvas);
  stats.written = writePng(canvas, pngPath);
  return stats;
}

uint64_t Picture::next() {
  static std::atomic<uint64_t> made{0};
  return ++made;
}

void setGpuPaint(OxyOSPaint* paint) { gpuPaintSet = paint; }

OxyOSPaint* gpuPaint() { return gpuPaintSet; }

CpuPaintScope::CpuPaintScope() { ++cpuOnly; }

CpuPaintScope::~CpuPaintScope() { --cpuOnly; }

uint32_t gpuCanvasId(const Canvas& canvas) { return canvas.gpu ? canvas.gpu->id : 0; }

std::vector<uint32_t> canvasPixels(const Canvas& canvas) {
  if (!canvas.gpu) return canvas.pixels;
  std::vector<uint32_t> pixels(static_cast<size_t>(std::max(canvas.width, 0)) * std::max(canvas.height, 0));
  int64_t read = oxyos_paint_canvas_read(canvas.gpu->paint, canvas.gpu->id, pixels.data(), pixels.size());
  if (read != static_cast<int64_t>(pixels.size())) return {};
  return pixels;
}

FrameDifference compareFrames(const std::vector<uint32_t>& a, const std::vector<uint32_t>& b) {
  FrameDifference difference;
  difference.pixels = std::min(a.size(), b.size());
  if (a.size() != b.size()) {
    difference.largest = 255;
    difference.beyondThree = std::max(a.size(), b.size());
    return difference;
  }
  for (size_t i = 0; i < a.size(); ++i) {
    if (a[i] == b[i]) continue;
    int largest = 0;
    for (int shift = 0; shift < 32; shift += 8) {
      largest = std::max(largest, std::abs(static_cast<int>((a[i] >> shift) & 0xff) -
                                           static_cast<int>((b[i] >> shift) & 0xff)));
    }
    difference.largest = std::max(difference.largest, largest);
    difference.beyondThree += largest > 3;
  }
  return difference;
}

}  // namespace oxyos
