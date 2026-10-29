#include "renderer.h"

#include "expo-blur.h"

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
#include <limits>
#include <set>

using namespace facebook::react;

namespace oxyos {

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
        views_[m.newChildShadowView.tag].view = m.newChildShadowView;
        break;
      }
      case ShadowViewMutation::Remove: {
        auto& children = views_[m.parentTag].children;
        children.erase(std::remove(children.begin(), children.end(), m.oldChildShadowView.tag), children.end());
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

class Painter {
 public:
  Painter(Canvas& canvas, float scale) : canvas_(canvas), scale_(scale) {}

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
    for (int py = y0; py < y1; ++py) {
      for (int px = x0; px < x1; ++px) {
        float lx = px / scale_, ly = py / scale_;
        if (lx + 1 / scale_ <= clip.x0 || lx >= clip.x1 || ly + 1 / scale_ <= clip.y0 || ly >= clip.y1) continue;
        float coverage;
        if (rounded) {
          float s = 1 / scale_;
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
    if (lx >= x && ly >= y && lx + s <= x + w && ly + s <= y + h) {
      float limit = std::min(w, h) / 2;
      float tl = std::min(r.topLeft.horizontal, limit), tr = std::min(r.topRight.horizontal, limit);
      float bl = std::min(r.bottomLeft.horizontal, limit), br = std::min(r.bottomRight.horizontal, limit);
      bool clear = (lx >= x + tl || ly >= y + tl) && (lx + s <= x + w - tr || ly >= y + tr) &&
                   (lx >= x + bl || ly + s <= y + h - bl) && (lx + s <= x + w - br || ly + s <= y + h - br);
      if (clear) return 1;
    }
    return roundedCoverageScaled(lx, ly, s, x, y, w, h, r);
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
    // size it is drawn), whole pixels inside the box, unrounded, untinted and opaque: each pixel
    // is its texel, the bilinear sample at a texel's centre, and an opaque one is copied. A
    // translucent texel is blended as below.
    auto whole = [](float value) { return std::abs(value - std::round(value)) < 1e-3f; };
    const float left = dx * scale_, top = dy * scale_;
    if (!rounded && !tint && opacity >= 1 && whole(left) && whole(top) &&
        std::abs(dw * scale_ - picture.width) < 1e-3f && std::abs(dh * scale_ - picture.height) < 1e-3f &&
        whole(box.x0 * scale_) && whole(box.y0 * scale_) && whole(box.x1 * scale_) && whole(box.y1 * scale_)) {
      const int ox = static_cast<int>(std::round(left)), oy = static_cast<int>(std::round(top));
      for (int py = y0; py < y1; ++py) {
        const uint8_t* row = picture.pixels.data() + (static_cast<size_t>(py - oy) * picture.width - ox) * 4;
        uint32_t* out = canvas_.pixels.data() + static_cast<size_t>(py) * canvas_.width;
        for (int px = x0; px < x1; ++px) {
          const uint8_t* t = row + static_cast<size_t>(px) * 4;
          if (t[3] == 255) {
            out[px] = 0xff000000u | (uint32_t{t[0]} << 16) | (uint32_t{t[1]} << 8) | t[2];
            continue;
          }
          if (t[3] == 0) continue;  // as below: nothing where the sample has no alpha
          const float alpha = t[3] / 255.0f;
          uint32_t& dst = out[px];
          auto channel = [&](int shift, float src) {
            float d = ((dst >> shift) & 0xff) / 255.0f;
            return static_cast<uint32_t>(std::lround(std::clamp(src + d * (1 - alpha), 0.0f, 1.0f) * 255)) << shift;
          };
          dst = channel(24, alpha) | channel(16, t[0] / 255.0f) | channel(8, t[1] / 255.0f) | channel(0, t[2] / 255.0f);
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

  // The device pixels painted, as limit() set them.
  const std::array<int, 4>& region() const { return region_; }

  void glyphs(const ParagraphLayout& layout, float ox, float oy, const Clip& clip, float opacity) {
    auto& engine = TextEngine::shared();
    for (const auto& run : layout.runs) {
      Rgba color{((run.color >> 16) & 0xff) / 255.0f, ((run.color >> 8) & 0xff) / 255.0f,
                 (run.color & 0xff) / 255.0f, ((run.color >> 24) & 0xff) / 255.0f * opacity};
      for (const auto& glyph : run.glyphs) {
        const auto& bitmap = engine.rasterize(glyph.face, glyph.id);
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

 private:
  Canvas& canvas_;
  float scale_;
  std::array<int, 4> region_{std::numeric_limits<int>::min(), std::numeric_limits<int>::min(),
                             std::numeric_limits<int>::max(), std::numeric_limits<int>::max()};
};

const std::set<std::string> kImplemented = {"RootView", "View", "Paragraph", "Text", "RawText", "ScrollView",
                                            "AndroidTextInput", "AndroidHorizontalScrollContentView",
                                            "Image", "RNSVGSvgView", "RNSVGSvgViewAndroid",
                                            "ViewManagerAdapter_ExpoBlur_ExpoBlurView",
                                            "ViewManagerAdapter_ExpoBlur_ExpoBlurTargetView"};

// The input's text laid out as it is drawn: single-line inputs do not wrap.
ParagraphLayout layoutInput(const AndroidTextInputProps& props, const AttributedString& text, float width) {
  float maxWidth = props.multiline ? width : std::numeric_limits<float>::infinity();
  return TextEngine::shared().layout(text, props.paragraphAttributes, maxWidth);
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
  Painter inside(layer, layerScale);
  bool invertible = std::abs(map.determinant()) >= 1e-6f && area.size.width > 0 && area.size.height > 0;
  const auto& region = painter.region();
  // The canvas's painted region, in logical pixels, as the layer's own coordinates see it.
  Rect painted{{std::max(region[0], 0) / painter.scale(), std::max(region[1], 0) / painter.scale()}, {}};
  painted.size = {std::min(region[2], 1 << 24) / painter.scale() - painted.origin.x,
                  std::min(region[3], 1 << 24) / painter.scale() - painted.origin.y};
  Rect needed = invertible ? Rect::intersect(map.bounds(area), painted) : Rect{};
  if (needed.size.width > 0 && needed.size.height > 0) {
    layer.width = static_cast<int>(std::ceil(area.size.width * layerScale));
    layer.height = static_cast<int>(std::ceil(area.size.height * layerScale));
    layer.pixels.assign(static_cast<size_t>(layer.width) * layer.height, 0);
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
  if (!layer.pixels.empty()) {
    painter.layer(layer, layerScale, area, map, clip);
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
  float childOx = x, childOy = y;
  if (props) {
    opacity *= props->opacity;
    if (opacity <= 0.001f) return;
    auto border = props->resolveBorderMetrics(view.layoutMetrics);
    Rgba background = rgba(props->backgroundColor);
    background.a *= opacity;
    painter.fill(x, y, w, h, border.borderRadii, background, clip);
    // expo-blur's view without a blur target, as on Android: its tint over what is behind it.
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
      childClip = clip.intersect(Clip{x, y, x + w, y + h});
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
    layer.width = static_cast<int>(std::lround(display.width * display.scale));
    layer.height = static_cast<int>(std::lround(display.height * display.scale));
    layer.pixels.assign(static_cast<size_t>(layer.width) * layer.height, 0);
    Painter inside(layer, display.scale);
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
  canvas.width = static_cast<int>(std::lround(display.width * display.scale));
  canvas.height = static_cast<int>(std::lround(display.height * display.scale));
  canvas.pixels.assign(static_cast<size_t>(canvas.width) * canvas.height, 0);
  RenderStats stats;
  if (!tree.views().count(modal)) return stats;
  Painter painter(canvas, display.scale);
  drawModalOnto(tree, modal, display, painter, stats, edit);
  return stats;
}

Rect Affine::bounds(const Rect& rect) const {
  return Rect::boundingRect(apply(rect.origin), apply(Point{rect.getMaxX(), rect.getMinY()}),
                            apply(Point{rect.getMinX(), rect.getMaxY()}), apply(Point{rect.getMaxX(), rect.getMaxY()}));
}

Affine viewTransform(const ShadowView& view) {
  const auto& origin = view.layoutMetrics.frame.origin;
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  if (!props || props->transform == Transform::Identity()) return Affine::translation(origin.x, origin.y);
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
  canvas.width = static_cast<int>(std::lround(display.width * display.scale));
  canvas.height = static_cast<int>(std::lround(display.height * display.scale));
  canvas.pixels.assign(static_cast<size_t>(canvas.width) * canvas.height, 0xffffffff);
  Painter painter(canvas, display.scale);
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

RenderStats drawDamaged(const ViewTree& tree, const Display& display, Canvas& canvas, const TextInputEdit& edit,
                        const std::vector<Rect>& damage) {
  int width = static_cast<int>(std::lround(display.width * display.scale));
  int height = static_cast<int>(std::lround(display.height * display.scale));
  if (canvas.width != width || canvas.height != height) return draw(tree, display, canvas, edit);
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
    for (int y = y0; y < y1; ++y) {
      std::fill_n(canvas.pixels.begin() + static_cast<long>(y) * width + x0, x1 - x0, 0xffffffffu);
    }
    Painter painter(canvas, display.scale);
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
    Painter painter(canvas, display.scale);
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
  canvas.width = static_cast<int>(std::lround(display.width * display.scale));
  canvas.height = static_cast<int>(std::lround(display.height * display.scale));
  canvas.pixels.assign(static_cast<size_t>(canvas.width) * canvas.height, 0);
  Painter painter(canvas, display.scale);
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
  int width = static_cast<int>(std::lround(display.width * display.scale));
  int height = static_cast<int>(std::lround(display.height * display.scale));
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
    for (int y = y0; y < y1; ++y) {
      std::fill_n(canvas.pixels.begin() + static_cast<long>(y) * width + x0, x1 - x0, 0u);
    }
    Painter painter(canvas, display.scale);
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
    Painter painter(canvas, display.scale);
    painter.limit(0, 0, 0, 0);
    drawChildren(painter, stats);
  }
  return stats;
}

namespace {

std::optional<Rect> frameUnder(const ViewTree& tree, Tag tag, Tag at, Point offset) {
  auto it = tree.views().find(at);
  if (it == tree.views().end()) return std::nullopt;
  const auto& view = it->second.view;
  Point moved = transformOffset(view);
  Point position{offset.x + view.layoutMetrics.frame.origin.x + moved.x,
                 offset.y + view.layoutMetrics.frame.origin.y + moved.y};
  // A modal is at the window's origin, wherever it is in the tree.
  if (isModal(view.componentName ? view.componentName : "")) position = Point{0, 0};
  if (at == tag) return Rect{position, view.layoutMetrics.frame.size};
  Point children = position;
  if (std::string(view.componentName ? view.componentName : "") == "ScrollView" && view.state) {
    const auto& state = static_cast<const ConcreteState<ScrollViewState>&>(*view.state).getData();
    children.x -= state.contentOffset.x;
    children.y -= state.contentOffset.y;
  }
  for (Tag child : it->second.children) {
    if (auto found = frameUnder(tree, tag, child, children)) return found;
  }
  return std::nullopt;
}

}  // namespace

namespace {

std::vector<Tag> pathTo(const ViewTree& tree, Tag at, Tag tag) {
  if (at == tag) return {tag};
  auto it = tree.views().find(at);
  if (it == tree.views().end()) return {};
  for (Tag child : it->second.children) {
    auto path = pathTo(tree, child, tag);
    if (!path.empty()) {
      path.insert(path.begin(), at);
      return path;
    }
  }
  return {};
}

}  // namespace

std::vector<Tag> ancestry(const ViewTree& tree, Tag tag) {
  return tree.root() < 0 ? std::vector<Tag>{} : pathTo(tree, tree.root(), tag);
}

std::optional<Rect> windowFrame(const ViewTree& tree, Tag tag) {
  return tree.root() < 0 ? std::nullopt : frameUnder(tree, tag, tree.root(), {0, 0});
}

bool writePng(const Canvas& canvas, const std::string& path) {
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

}  // namespace oxyos
