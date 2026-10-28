#include "renderer.h"

#include "text.h"

#include <react/renderer/components/androidtextinput/AndroidTextInputProps.h>
#include <react/renderer/components/androidtextinput/AndroidTextInputState.h>
#include <react/renderer/components/scrollview/ScrollViewState.h>
#include <react/renderer/components/text/ParagraphState.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/core/ConcreteState.h>
#include <react/renderer/graphics/Color.h>

#include <algorithm>
#include <array>
#include <cmath>
#include <fstream>
#include <cstdio>
#include <cstdlib>
#include <limits>
#include <set>

using namespace facebook::react;

namespace oxy {

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
    x0 = std::max(x0, region_[0]), y0 = std::max(y0, region_[1]);
    x1 = std::min(x1, region_[2]), y1 = std::min(y1, region_[3]);
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
        blend(px, py, color, std::clamp(coverage, 0.0f, 1.0f));
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
                                            "Image"};

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
  }
  if (edit->selectionStart == edit->selectionEnd) {
    auto caret = geometry->caret(edit->selectionEnd);
    Rgba color = props->cursorColor ? rgba(props->cursorColor) : ink;
    color.a *= opacity;
    painter.fill(left + caret.x, boxTop + caret.top, kCaretWidth, caret.height, BorderRadii{}, color, inside);
  }
}

void drawView(const ViewTree& tree, Tag tag, float ox, float oy, Clip clip, float opacity, Painter& painter,
              RenderStats& stats, const TextInputEdit& edit) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  const ShadowView& view = it->second.view;
  const auto& frame = view.layoutMetrics.frame;
  float x = ox + frame.origin.x, y = oy + frame.origin.y, w = frame.size.width, h = frame.size.height;
  std::string name = view.componentName ? view.componentName : "";
  // A popup's subtree is drawn in its own surface (drawPopup), never in the window.
  if (name == kPopupComponent) return;
  ++stats.views;
  if (!kImplemented.count(name)) {
    ++stats.unimplemented;
    if (std::find(stats.unimplementedNames.begin(), stats.unimplementedNames.end(), name) ==
        stats.unimplementedNames.end()) {
      stats.unimplementedNames.push_back(name);
    }
  }

  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  // Debug aid: OXY_HOST_DUMP=1 prints every drawn view, indented by depth.
  static const bool dump = std::getenv("OXY_HOST_DUMP") != nullptr;
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

RenderStats draw(const ViewTree& tree, const Display& display, Canvas& canvas, const TextInputEdit& edit) {
  canvas.width = static_cast<int>(std::lround(display.width * display.scale));
  canvas.height = static_cast<int>(std::lround(display.height * display.scale));
  canvas.pixels.assign(static_cast<size_t>(canvas.width) * canvas.height, 0xffffffff);
  Painter painter(canvas, display.scale);
  RenderStats stats;
  drawView(tree, tree.root(), 0, 0, Clip{0, 0, display.width, display.height}, 1.0f, painter, stats, edit);
  return stats;
}

bool drawsTheSame(const ShadowView& before, const ShadowView& after) {
  if (before.props == after.props) return true;
  std::string name = after.componentName ? after.componentName : "";
  if ((before.componentName ? before.componentName : "") != name || name == "AndroidTextInput") return false;
  auto a = std::dynamic_pointer_cast<const ViewProps>(before.props);
  auto b = std::dynamic_pointer_cast<const ViewProps>(after.props);
  if (!a || !b) return !a && !b;
  // What drawView reads from a view's props, each with its own layout.
  return a->opacity == b->opacity && a->backgroundColor == b->backgroundColor &&
         a->resolveBorderMetrics(before.layoutMetrics) == b->resolveBorderMetrics(after.layoutMetrics) &&
         a->getClipsContentToBounds() == b->getClipsContentToBounds();
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
  }
  return stats;
}

RenderStats drawPopup(const ViewTree& tree, Tag popup, const Display& display, Canvas& canvas,
                      const TextInputEdit& edit) {
  canvas.width = static_cast<int>(std::lround(display.width * display.scale));
  canvas.height = static_cast<int>(std::lround(display.height * display.scale));
  canvas.pixels.assign(static_cast<size_t>(canvas.width) * canvas.height, 0);
  Painter painter(canvas, display.scale);
  RenderStats stats;
  auto it = tree.views().find(popup);
  if (it == tree.views().end()) return stats;
  for (Tag child : it->second.children) {
    drawView(tree, child, 0, 0, Clip{0, 0, display.width, display.height}, 1.0f, painter, stats, edit);
  }
  return stats;
}

namespace {

std::optional<Rect> frameUnder(const ViewTree& tree, Tag tag, Tag at, Point offset) {
  auto it = tree.views().find(at);
  if (it == tree.views().end()) return std::nullopt;
  const auto& view = it->second.view;
  Point position{offset.x + view.layoutMetrics.frame.origin.x, offset.y + view.layoutMetrics.frame.origin.y};
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

}  // namespace oxy
