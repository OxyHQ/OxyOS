#include "renderer.h"

#include "android-view-props.h"
#include "background.h"
#include "expo-blur.h"
#include "masked-view.h"
#include "skia-gpu.h"

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

#include <include/core/SkCanvas.h>
#include <include/core/SkBlurTypes.h>
#include <include/core/SkColorFilter.h>
#include <include/core/SkFontArguments.h>
#include <include/core/SkFontMgr.h>
#include <include/core/SkTypeface.h>
#include <include/core/SkImage.h>
#include <include/core/SkM44.h>
#include <include/core/SkMaskFilter.h>
#include <include/core/SkPaint.h>
#include <include/core/SkPathBuilder.h>
#include <include/core/SkPicture.h>
#include <include/core/SkPixmap.h>
#include <include/core/SkRRect.h>
#include <include/core/SkSurface.h>
#include <include/core/SkTextBlob.h>
#include <include/gpu/ganesh/GrDirectContext.h>
#include <include/gpu/ganesh/SkSurfaceGanesh.h>
#include <include/effects/SkImageFilters.h>
#include <include/utils/SkShadowUtils.h>
#include <include/ports/SkFontMgr_empty.h>

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

struct Clip {
  float x0, y0, x1, y1;
  Clip intersect(const Clip& o) const {
    return {std::max(x0, o.x0), std::max(y0, o.y0), std::min(x1, o.x1), std::min(y1, o.y1)};
  }
  bool empty() const { return x1 <= x0 || y1 <= y0; }
};

constexpr float kInfinity = std::numeric_limits<float>::infinity();
constexpr Clip kUnclipped{-kInfinity, -kInfinity, kInfinity, kInfinity};

// A colour as Skia takes it, with the alpha a view without overlapping rendering multiplies into
// every draw (Android's alpha filter canvas).
SkColor4f faded(const SharedColor& color, float opacity, SkColor4f unset = {0, 0, 0, 0}) {
  if (!color) {
    unset.fA *= opacity;
    return unset;
  }
  auto c = colorComponentsFromColor(color);
  return SkColor4f{c.red, c.green, c.blue, c.alpha * opacity};
}

SkColor4f faded(uint32_t argb, float opacity) {
  SkColor4f c = SkColor4f::FromColor(argb);
  c.fA *= opacity;
  return c;
}

// What a traversal draws onto: the canvas (in logical pixels, its matrix scaling them to the
// device's) and what it gathers.
struct Draw {
  SkCanvas* canvas = nullptr;
  float scale = 1;
  RenderStats* stats = nullptr;
  const TextInputEdit* edit = nullptr;
  // Whether a window shown live is placed from what is drawn (docs/adr/0022-window-views.mdx): the
  // surface's canvas, or a view only moved and scaled (a dragged card); not a mask, a blur, a
  // modal animating or a view turned or skewed. `toSurface` maps the current coordinates to the
  // surface's logical pixels, and `surfaceClip` is what clips them there.
  bool placesWindows = true;
  Affine toSurface;
  Clip surfaceClip = kUnclipped;
};

// Whether a rectangle of the current coordinates has any pixel the canvas paints (its clip, the
// damage among it).
bool touches(const Draw& draw, float x, float y, float w, float h) {
  return !draw.canvas->quickReject(SkRect::MakeXYWH(x, y, w, h));
}

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

// The Skia font a glyph laid out in `face` (text.h) is drawn with: the same file as a Skia
// typeface (its weight on a variable font's `wght` axis), at the face's size in logical pixels (the
// canvas's matrix takes it to the device's), anti-aliased in gray with slight hinting and subpixel
// positioning, as Chromium draws text on Linux, and slanted as Android fakes an italic.
SkFont skiaFont(const Face* face) {
  static sk_sp<SkFontMgr> fonts = SkFontMgr_New_Custom_Empty();
  // Each file at each weight and optical size it is drawn at, as the text engine set them.
  static std::map<std::tuple<std::string, int, int>, sk_sp<SkTypeface>> typefaces;
  auto key = std::make_tuple(face->path, face->variable ? face->weight : 0,
                             static_cast<int>(std::lround(face->opticalSize * 64)));
  auto it = typefaces.find(key);
  if (it == typefaces.end()) {
    sk_sp<SkTypeface> typeface = fonts->makeFromFile(face->path.c_str(), 0);
    std::vector<SkFontArguments::VariationPosition::Coordinate> coordinates;
    if (face->variable) coordinates.push_back({SkSetFourByteTag('w', 'g', 'h', 't'), static_cast<float>(face->weight)});
    if (face->opticalSize > 0) coordinates.push_back({SkSetFourByteTag('o', 'p', 's', 'z'), face->opticalSize});
    if (typeface && !coordinates.empty()) {
      SkFontArguments arguments;
      arguments.setVariationDesignPosition({coordinates.data(), static_cast<int>(coordinates.size())});
      if (auto clone = typeface->makeClone(arguments)) typeface = std::move(clone);
    }
    it = typefaces.emplace(key, std::move(typeface)).first;
  }
  SkFont font(it->second, face->size);
  font.setEdging(SkFont::Edging::kAntiAlias);
  font.setHinting(SkFontHinting::kSlight);
  font.setSubpixel(true);
  if (face->fakeItalic) font.setSkewX(-0.25f);
  // Android's fake bold, as Paint.setFakeBoldText draws it.
  if (face->fakeBold) font.setEmbolden(true);
  return font;
}

// A paragraph's glyph runs at (ox, oy), as Android's Layout.draw draws them: each run's shadow
// (setShadowLayer) under it, its glyphs, then its underline and strike-through (HWUI's
// drawTextDecorations).
void drawGlyphs(const Draw& draw, const ParagraphLayout& layout, float ox, float oy, float opacity) {
  SkCanvas& canvas = *draw.canvas;
  for (const auto& run : layout.runs) {
    if (run.glyphs.empty()) continue;
    SkTextBlobBuilder builder;
    for (size_t start = 0; start < run.glyphs.size();) {
      size_t end = start + 1;
      while (end < run.glyphs.size() && run.glyphs[end].face == run.glyphs[start].face) ++end;
      SkFont font = skiaFont(run.glyphs[start].face);
      const auto& buffer = builder.allocRunPosH(font, static_cast<int>(end - start), run.baseline);
      for (size_t g = start; g < end; ++g) {
        buffer.glyphs[g - start] = static_cast<SkGlyphID>(run.glyphs[g].id);
        buffer.pos[g - start] = run.glyphs[g].x;
      }
      start = end;
    }
    sk_sp<SkTextBlob> blob = builder.make();
    if (!blob) continue;
    const Face& decoration = *run.glyphs.front().face;
    auto decorate = [&](const SkPaint& paint, float dx, float dy) {
      if (run.underline) {
        canvas.drawRect(SkRect::MakeLTRB(ox + dx + run.left, oy + dy + run.baseline + decoration.underlineTop,
                                         ox + dx + run.right,
                                         oy + dy + run.baseline + decoration.underlineTop + decoration.underlineThickness),
                        paint);
      }
      if (run.strikethrough) {
        canvas.drawRect(SkRect::MakeLTRB(ox + dx + run.left, oy + dy + run.baseline + decoration.strikeTop,
                                         ox + dx + run.right,
                                         oy + dy + run.baseline + decoration.strikeTop + decoration.strikeThickness),
                        paint);
      }
    };
    if (run.shadowColor >> 24) {
      SkPaint shadow(faded(run.shadowColor, opacity));
      shadow.setAntiAlias(true);
      // setShadowLayer's radius, in pixels, as HWUI's Blur::convertRadiusToSigma takes it.
      if (run.shadowRadius > 0) {
        float sigma = (0.57735f * run.shadowRadius * draw.scale + 0.5f) / draw.scale;
        shadow.setMaskFilter(SkMaskFilter::MakeBlur(kNormal_SkBlurStyle, sigma));
      }
      canvas.drawTextBlob(blob, ox + run.shadowDx, oy + run.shadowDy, shadow);
      decorate(shadow, run.shadowDx, run.shadowDy);
    }
    SkPaint paint(faded(run.color, opacity));
    paint.setAntiAlias(true);
    canvas.drawTextBlob(blob, ox, oy, paint);
    decorate(paint, 0, 0);
  }
}

void fillRect(const Draw& draw, float x, float y, float w, float h, SkColor4f color) {
  if (color.fA <= 0) return;
  SkPaint paint(color);
  paint.setAntiAlias(true);
  draw.canvas->drawRect(SkRect::MakeXYWH(x, y, w, h), paint);
}

// The caret, a bar kCaretWidth wide whose left edge is at x, on whole device pixels at the
// canvas's scale (1.25 included) as a Mac's and Android's carets are drawn: crisp, its left edge
// at the device pixel nearest x, at least one device pixel wide.
void fillCaret(const Draw& draw, float x, float y, float h, SkColor4f color) {
  if (color.fA <= 0) return;
  const SkMatrix& matrix = draw.canvas->getTotalMatrix();
  SkRect rect = SkRect::MakeXYWH(x, y, kCaretWidth, h);
  SkMatrix inverse;
  if (matrix.isScaleTranslate() && matrix.invert(&inverse)) {
    SkRect device = matrix.mapRect(rect);
    float left = std::round(device.left());
    float width = std::max(1.0f, std::round(kCaretWidth * matrix.getScaleX()));
    rect = inverse.mapRect(SkRect::MakeLTRB(left, std::round(device.top()), left + width, std::round(device.bottom())));
  }
  SkPaint paint(color);
  draw.canvas->drawRect(rect, paint);
}

// A text input's text, or its placeholder when empty. When it has focus (`edit`): the selection
// behind the text, the composition underlined, and the caret. A single-line input centres its
// line vertically, as Android's EditText does.
void drawTextInput(const ShadowView& view, float x, float y, float w, float h, float opacity, const Draw& draw) {
  const TextInputEdit& editing = *draw.edit;
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
  SkRect inside = SkRect::MakeXYWH(x, y, w, h);
  if (scroll.x != 0 || scroll.y != 0 || props->multiline) {
    float boxLeft = x + content.origin.x, boxTop = y + content.origin.y;
    inside = props->multiline ? SkRect::MakeXYWH(x, boxTop, w, content.size.height)
                              : SkRect::MakeXYWH(boxLeft, y, content.size.width, h);
    inside.intersect(SkRect::MakeXYWH(x, y, w, h));
  }
  SkCanvas& canvas = *draw.canvas;
  canvas.save();
  canvas.clipRect(inside, true);
  SharedColor textColor = props->getEffectiveTextAttributes(1).foregroundColor;
  std::optional<TextInputGeometry> geometry;
  if (edit) geometry.emplace(view, value);
  // The geometry's positions are relative to the content box, before scrolling.
  float boxTop = y + content.origin.y - scroll.y;

  if (edit && edit->selectionStart != edit->selectionEnd) {
    // In a window that is not key, macOS's unemphasizedSelectedTextBackgroundColor (#dcdcdc light,
    // #464646 dark), whatever `selectionColor` asks: NSTextView applies its selected-text
    // attributes only in the key window.
    float unemphasized = edit->dark ? 70 / 255.0f : 220 / 255.0f;
    SkColor4f selection = !edit->key             ? SkColor4f{unemphasized, unemphasized, unemphasized, opacity}
                          : props->selectionColor ? faded(props->selectionColor, opacity)
                                                  : SkColor4f{0.25f, 0.45f, 1.0f, 0.3f * opacity};
    for (const auto& rect : geometry->span(edit->selectionStart, edit->selectionEnd)) {
      fillRect(draw, left + rect.origin.x, boxTop + rect.origin.y, rect.size.width, rect.size.height, selection);
    }
  }
  drawGlyphs(draw, layout, left, top, opacity);
  if (edit) {
    SkColor4f ink = faded(textColor, opacity, SkColor4f{0, 0, 0, 1});
    if (edit->composeStart != edit->composeEnd) {
      // Underlined under each line it covers, just below the line's text.
      for (const auto& rect : geometry->span(edit->composeStart, edit->composeEnd)) {
        fillRect(draw, left + rect.origin.x, boxTop + rect.origin.y + rect.size.height - 1, rect.size.width, 1, ink);
      }
      // The segment being converted: a heavier underline on a tint of the text's colour, drawn over
      // the text as Android's composing background is under it; an error, underlined in red.
      for (const auto& segment : edit->segments) {
        if (segment.style == OXYOS_PREEDIT_COMPOSING) continue;
        bool converting = segment.style == OXYOS_PREEDIT_CONVERTING;
        SkColor4f line = converting ? ink : SkColor4f{0.86f, 0.15f, 0.15f, opacity};
        SkColor4f tint{ink.fR, ink.fG, ink.fB, ink.fA * 0.15f};
        for (const auto& rect : geometry->span(segment.start, segment.end)) {
          if (converting) {
            fillRect(draw, left + rect.origin.x, boxTop + rect.origin.y, rect.size.width, rect.size.height, tint);
          }
          float thickness = converting ? 2 : 1;
          fillRect(draw, left + rect.origin.x, boxTop + rect.origin.y + rect.size.height - thickness, rect.size.width,
                   thickness, line);
        }
      }
    }
    if (edit->selectionStart == edit->selectionEnd && edit->caretLit && edit->key && editing.dropTag != view.tag) {
      auto caret = geometry->caret(edit->selectionEnd);
      SkColor4f color = props->cursorColor ? faded(props->cursorColor, opacity) : ink;
      fillCaret(draw, left + caret.x, boxTop + caret.top, caret.height, color);
    }
  }
  // Text dragged over the input: where it would drop, as a caret that does not blink.
  if (editing.dropTag == view.tag) {
    auto caret = (geometry ? *geometry : TextInputGeometry(view, value)).caret(editing.dropOffset);
    SkColor4f ink = faded(textColor, opacity, SkColor4f{0, 0, 0, 1});
    fillCaret(draw, left + caret.x, boxTop + caret.top, caret.height,
              props->cursorColor ? faded(props->cursorColor, opacity) : ink);
  }
  canvas.restore();
}

// Fresco's IterativeBoxBlurPostProcessor, which React Native's Android Image blurs with: three box
// blurs of the radius in pixels, whose deviation is sqrt(r (r + 1)), the picture's edges extended.
sk_sp<SkImageFilter> imageBlur(float blurRadius, float scale, const SkRect& bounds) {
  float radius = std::round(blurRadius * scale);
  if (radius <= 0) return nullptr;
  float sigma = std::sqrt(radius * (radius + 1)) / scale;
  return SkImageFilters::Blur(sigma, sigma, SkTileMode::kClamp, nullptr, bounds);
}

// An Image view's picture within its content box, as React Native's resizeMode places it, rounded
// with the view's corners, tinted with its tintColor (Android's SRC_IN filter) and blurred by its
// blurRadius.
void drawImage(const ShadowView& view, const Picture& picture, float x, float y, float w, float h, float opacity,
               const Draw& draw) {
  if (!picture.image) return;
  auto props = std::static_pointer_cast<const ImageProps>(view.props);
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
  SkCanvas& canvas = *draw.canvas;
  canvas.save();
  auto border = props->resolveBorderMetrics(view.layoutMetrics);
  canvas.clipRect(SkRect::MakeXYWH(cx, cy, cw, ch), true);
  canvas.clipRRect(borderBox(border, x, y, w, h), true);
  SkPaint paint;
  paint.setAntiAlias(true);
  paint.setAlphaf(opacity);
  if (props->tintColor && *props->tintColor) {
    paint.setColorFilter(SkColorFilters::Blend(faded(*props->tintColor, 1), nullptr, SkBlendMode::kSrcIn));
  }
  const SkSamplingOptions sampling(SkFilterMode::kLinear);
  auto place = [&](float dx, float dy, float dw, float dh) {
    SkRect target = SkRect::MakeXYWH(dx, dy, dw, dh);
    SkPaint drawn = paint;
    if (props->blurRadius > 0) drawn.setImageFilter(imageBlur(props->blurRadius, draw.scale, target));
    canvas.drawImageRect(picture.image, target, sampling, &drawn);
  };
  switch (props->resizeMode) {
    case ImageResizeMode::Stretch:
      place(cx, cy, cw, ch);
      break;
    case ImageResizeMode::Cover:
    case ImageResizeMode::Contain: {
      bool cover = props->resizeMode == ImageResizeMode::Cover;
      float dw = cw, dh = cw / aspect;
      if (cover ? dh < ch : dh > ch) dh = ch, dw = ch * aspect;
      place(cx + (cw - dw) / 2, cy + (ch - dh) / 2, dw, dh);
      break;
    }
    case ImageResizeMode::Center: {
      auto [dw, dh] = within(nw, nh);
      place(cx + (cw - dw) / 2, cy + (ch - dh) / 2, dw, dh);
      break;
    }
    case ImageResizeMode::None:
      place(cx, cy, nw, nh);
      break;
    case ImageResizeMode::Repeat: {
      auto [dw, dh] = within(nw, nh);
      for (float ty = cy; ty < cy + ch; ty += dh) {
        for (float tx = cx; tx < cx + cw; tx += dw) place(tx, ty, dw, dh);
      }
      break;
    }
  }
  canvas.restore();
}

// A view's `transform` as Android's RenderNode applies it, perspective included: from the view's
// own coordinates (its frame's origin at 0, 0) to its parent's, around its transform origin.
SkM44 viewMatrix(const ShadowView& view) {
  const auto& frame = view.layoutMetrics.frame;
  const auto* props = dynamic_cast<const ViewProps*>(view.props.get());
  static const Transform identity = Transform::Identity();
  if (!props || props->transform == identity) return SkM44::Translate(frame.origin.x, frame.origin.y);
  const auto& m = props->resolveTransform(view.layoutMetrics).matrix;
  SkScalar columns[16];
  for (int i = 0; i < 16; ++i) columns[i] = static_cast<SkScalar>(m[i]);
  return SkM44::Translate(frame.origin.x + frame.size.width / 2, frame.origin.y + frame.size.height / 2) *
         SkM44::ColMajor(columns) * SkM44::Translate(-frame.size.width / 2, -frame.size.height / 2);
}

bool hasPerspective(const SkM44& m) {
  return m.rc(3, 0) != 0 || m.rc(3, 1) != 0 || m.rc(3, 2) != 0 || m.rc(3, 3) != 1;
}

void drawView(const ViewTree& tree, Tag tag, float ox, float oy, Clip clip, float opacity, Draw& draw,
              bool untransformed = false);

// A view whose transform scales, turns, skews or projects it: its subtree drawn through the
// canvas's matrix, as Android draws a render node through its matrix. `matrix` maps the view's
// own coordinates to the current ones.
void drawTransformed(const ViewTree& tree, Tag tag, const SkM44& matrix, Clip clip, float opacity, Draw& draw) {
  SkCanvas& canvas = *draw.canvas;
  Draw inside = draw;
  // Moved and scaled, not turned, skewed or projected: the windows in it are placed where it is.
  bool scales = !hasPerspective(matrix) && matrix.rc(0, 1) == 0 && matrix.rc(1, 0) == 0 && matrix.rc(0, 0) > 0 &&
                matrix.rc(1, 1) > 0;
  inside.placesWindows = draw.placesWindows && scales;
  Clip local = kUnclipped;
  SkM44 inverse;
  if (matrix.invert(&inverse) && !hasPerspective(matrix)) {
    if (!clip.empty() && std::isfinite(clip.x0) && std::isfinite(clip.x1) && std::isfinite(clip.y0) &&
        std::isfinite(clip.y1)) {
      SkRect mapped = inverse.asM33().mapRect(SkRect::MakeLTRB(clip.x0, clip.y0, clip.x1, clip.y1));
      local = Clip{mapped.fLeft, mapped.fTop, mapped.fRight, mapped.fBottom};
    }
  } else if (!matrix.invert(&inverse)) {
    local = Clip{0, 0, 0, 0};
  }
  if (inside.placesWindows) {
    Affine map{matrix.rc(0, 0), 0, 0, matrix.rc(1, 1), matrix.rc(0, 3), matrix.rc(1, 3)};
    Rect bounds = draw.toSurface.bounds(Rect{{clip.x0, clip.y0}, {clip.x1 - clip.x0, clip.y1 - clip.y0}});
    inside.surfaceClip = draw.surfaceClip.intersect(Clip{bounds.origin.x, bounds.origin.y, bounds.getMaxX(), bounds.getMaxY()});
    inside.toSurface = draw.toSurface.after(map);
  }
  canvas.save();
  canvas.concat(matrix);
  const auto& frame = tree.views().at(tag).view.layoutMetrics.frame;
  drawView(tree, tag, -frame.origin.x, -frame.origin.y, local, opacity, inside, true);
  canvas.restore();
}

// Whether the blur view being drawn is drawing its target: a blur view inside that target draws
// its tint alone, as Dimezis' does, so a target never blurs into itself.
thread_local int drawingBlurTarget = 0;

// expo-blur's view with a blur target (expo-blur.h), as Dimezis' BlurView draws on Android: the
// target's content under the view, drawn afresh (never what the canvas holds, which outside the
// damage is the last frame, this view's blur included), blurred (a Gaussian of the radius as its
// deviation, the edges extended), within the view and its rounded corners. Its tint is drawn over
// it by the caller.
void drawBlurred(const ViewTree& tree, Tag blur, const ExpoBlurTarget& blurTarget, float x, float y, float w, float h,
                 const BorderMetrics& border, float opacity, const Draw& draw) {
  if (!touches(draw, x, y, w, h)) return;
  auto blurAt = windowFrame(tree, blur);
  auto targetAt = windowFrame(tree, blurTarget.target);
  auto target = tree.views().find(blurTarget.target);
  if (!blurAt || !targetAt || target == tree.views().end()) return;
  SkCanvas& canvas = *draw.canvas;
  // Only the part of the view the canvas paints (its clip, the damage among it) is worked out.
  SkRect visible = SkRect::MakeXYWH(x, y, w, h);
  SkRect painted;
  if (!canvas.getLocalClipBounds(&painted) || !visible.intersect(painted)) return;
  // As Dimezis' BlurView draws it: the target drawn reduced into a picture of its own, blurred there
  // and stretched back over the view. A wide blur (16 device pixels of deviation or more:
  // Launchpad's backdrop, 37 at 1.25) is worked out k times smaller (an eighth of the deviation, at
  // most 8), by what the Gaussian has left to do at that size (drawing it reduced and stretching it
  // back blur by about k²/4 of variance between them); what a blur that wide keeps is far coarser
  // than k pixels, so it is within a level or two of the blur at full size, for a k²th of the work.
  // The picture is kept in half floats and stretched back dithered, so its gradients, shallower
  // than a level per pixel, never band.
  const float sigma = blurTarget.radius * draw.scale;
  const int k = sigma >= 16 ? std::min(8, static_cast<int>(sigma / 8)) : 1;
  const float reducedScale = draw.scale / k;
  const float reducedSigma = k > 1 ? std::sqrt(std::max(0.25f, sigma * sigma - k * k / 4.0f)) / k : sigma;
  // What the blur reads around a pixel: three deviations.
  const int margin = static_cast<int>(std::ceil(3 * reducedSigma));
  const float left = visible.fLeft - margin / reducedScale, top = visible.fTop - margin / reducedScale;
  const int width = static_cast<int>(std::ceil(visible.width() * reducedScale)) + 2 * margin;
  const int height = static_cast<int>(std::ceil(visible.height() * reducedScale)) + 2 * margin;
  const SkImageInfo info = SkImageInfo::Make(width, height, kRGBA_F16Norm_SkColorType, kPremul_SkAlphaType);
  sk_sp<SkSurface> drawn = canvas.makeSurface(info);
  sk_sp<SkSurface> blurred = canvas.makeSurface(info);
  if (!drawn || !blurred) return;
  // The target where it is relative to this view.
  Affine map = viewTransform(target->second.view);
  float targetX = x + (targetAt->origin.x - blurAt->origin.x) - map.tx;
  float targetY = y + (targetAt->origin.y - blurAt->origin.y) - map.ty;
  SkCanvas& reduced = *drawn->getCanvas();
  reduced.scale(reducedScale, reducedScale);
  reduced.translate(-left, -top);
  RenderStats unseen;
  Draw inside{&reduced, reducedScale, &unseen, draw.edit};
  inside.placesWindows = false;
  ++drawingBlurTarget;
  drawView(tree, blurTarget.target, targetX, targetY, kUnclipped, 1.0f, inside);
  --drawingBlurTarget;
  SkPaint gaussian;
  gaussian.setBlendMode(SkBlendMode::kSrc);
  if (reducedSigma > 0) {
    // The picture's edges extended, as Dimezis' blur reads past them.
    gaussian.setImageFilter(SkImageFilters::Blur(reducedSigma, reducedSigma, SkTileMode::kClamp, nullptr));
  }
  blurred->getCanvas()->drawImage(drawn->makeImageSnapshot(), 0, 0, SkSamplingOptions(), &gaussian);
  SkPaint stretched;
  stretched.setAlphaf(opacity);
  stretched.setDither(true);
  canvas.save();
  canvas.clipRRect(borderBox(border, x, y, w, h), true);
  canvas.drawImageRect(blurred->makeImageSnapshot(),
                       SkRect::MakeXYWH(left, top, width / reducedScale, height / reducedScale),
                       SkSamplingOptions(SkFilterMode::kLinear), &stretched);
  canvas.restore();
}

// A MaskedView (masked-view.h): its children but the first drawn, kept by the alpha of its first
// child (DST_IN, as RNCMaskedView composites them), at the view's opacity.
void drawMasked(const ViewTree& tree, Tag tag, float x, float y, float w, float h, Clip clip, float opacity,
                Draw& draw) {
  const auto& children = tree.views().at(tag).children;
  if (children.empty()) return;
  SkCanvas& canvas = *draw.canvas;
  Draw inside = draw;
  inside.placesWindows = false;
  Clip within = clip.intersect(Clip{x, y, x + w, y + h});
  SkRect bounds = SkRect::MakeXYWH(x, y, w, h);
  SkPaint layer;
  layer.setAlphaf(opacity);
  canvas.saveLayer(&bounds, &layer);
  for (size_t i = 1; i < children.size(); ++i) drawView(tree, children[i], x, y, within, 1.0f, inside);
  SkPaint keep;
  keep.setBlendMode(SkBlendMode::kDstIn);
  canvas.saveLayer(&bounds, &keep);
  drawView(tree, children.front(), x, y, within, 1.0f, inside);
  canvas.restore();
  canvas.restore();
}

// A scroll view's child at (ox, oy), and through `stretch` when the scroll view is stretched past
// its end (overscroll, input.h): as Android draws a stretched ScrollView's render node.
void drawChild(const ViewTree& tree, Tag child, float ox, float oy, const Clip& clip, float opacity, Draw& draw,
               const Affine* stretch) {
  if (!stretch) return drawView(tree, child, ox, oy, clip, opacity, draw);
  auto it = tree.views().find(child);
  if (it == tree.views().end()) return;
  std::string name = it->second.view.componentName ? it->second.view.componentName : "";
  if (ownSurface(name) || isModal(name)) return;
  SkM44 matrix = SkM44(SkMatrix::MakeAll(stretch->a, stretch->c, stretch->tx, stretch->b, stretch->d, stretch->ty, 0, 0, 1)) *
                 SkM44::Translate(ox, oy) * viewMatrix(it->second.view);
  drawTransformed(tree, child, matrix, clip, opacity, draw);
}

// A view's `elevation`, as Android casts it: HWUI's shadow of the view's outline (its background's:
// the border box with its corners, which a view has once it has any background style: a colour, a
// border, rounded corners, a shadow or an outline; one with none casts nothing, as on Android) raised
// `elevation` pixels, by Skia's SkShadowUtils, with Android's light: 600 dp above the top of the
// window, at its middle, 800 dp wide, an ambient shadow at 0.039 and a spot shadow at 0.19 of the
// shadow colour (React Native's shadowColor, Android's outline ambient and spot colours; black by
// default), a translucent caster seen through. Android places the light at the middle of the
// display; a window here is placed by the compositor and does not know where it is, so the light
// is at the middle of its own surface.
void drawElevation(SkCanvas& canvas, const ViewProps& props, const LayoutMetrics& layout, float x, float y, float w,
                   float h, float elevation, float opacity, float scale) {
  if (!hasBackground(props, layout)) return;
  auto border = props.resolveBorderMetrics(layout);
  SkPath outline = SkPath::RRect(borderBox(border, x, y, w, h));
  SkColor4f color = faded(props.shadowColor, 1, SkColor4f{0, 0, 0, 1});
  auto alpha = [&](float share) { return SkColor4f{color.fR, color.fG, color.fB, color.fA * share * opacity}.toSkColor(); };
  SkISize device = canvas.getBaseLayerSize();
  const SkPoint3 light = SkPoint3::Make(device.width() / 2.0f, 0, 600 * scale);
  // HWUI sees through a caster drawn translucent (its alpha below 1).
  bool opaque = opacity >= 1;
  SkShadowUtils::DrawShadow(&canvas, outline, SkPoint3::Make(0, 0, elevation * scale), light, 800 * scale,
                            alpha(0.039f), alpha(0.19f), opaque ? kNone_ShadowFlag : kTransparentOccluder_ShadowFlag);
}

// Whether a view's own drawing needs a layer of its own (saveLayer): a filter, a blend mode, or
// an opacity it asks to be composited offscreen (needsOffscreenAlphaCompositing, Android's
// hasOverlappingRendering); every other opacity multiplies into each draw, as Android's does.
struct Group {
  bool layer = false;
  SkPaint paint;
};

Group groupOf(const ViewProps& props, const AndroidViewProps& android, float opacity) {
  Group group;
  if (android.needsOffscreenAlphaCompositing && opacity < 1) group.layer = true;
  if (auto filter = viewFilter(props)) {
    group.layer = true;
    group.paint.setImageFilter(std::move(filter));
  }
  if (props.mixBlendMode != BlendMode::Normal) {
    static const std::map<BlendMode, SkBlendMode> modes = {
        {BlendMode::Multiply, SkBlendMode::kMultiply},   {BlendMode::Screen, SkBlendMode::kScreen},
        {BlendMode::Overlay, SkBlendMode::kOverlay},     {BlendMode::Darken, SkBlendMode::kDarken},
        {BlendMode::Lighten, SkBlendMode::kLighten},     {BlendMode::ColorDodge, SkBlendMode::kColorDodge},
        {BlendMode::ColorBurn, SkBlendMode::kColorBurn}, {BlendMode::HardLight, SkBlendMode::kHardLight},
        {BlendMode::SoftLight, SkBlendMode::kSoftLight}, {BlendMode::Difference, SkBlendMode::kDifference},
        {BlendMode::Exclusion, SkBlendMode::kExclusion}, {BlendMode::Hue, SkBlendMode::kHue},
        {BlendMode::Saturation, SkBlendMode::kSaturation}, {BlendMode::Color, SkBlendMode::kColor},
        {BlendMode::Luminosity, SkBlendMode::kLuminosity}};
    if (auto mode = modes.find(props.mixBlendMode); mode != modes.end()) {
      group.layer = true;
      group.paint.setBlendMode(mode->second);
    }
  }
  if (group.layer) group.paint.setAlphaf(opacity);
  return group;
}

void drawView(const ViewTree& tree, Tag tag, float ox, float oy, Clip clip, float opacity, Draw& draw,
              bool untransformed) {
  auto it = tree.views().find(tag);
  if (it == tree.views().end()) return;
  const ShadowView& view = it->second.view;
  const auto& frame = view.layoutMetrics.frame;
  std::string name = view.componentName ? view.componentName : "";
  // A popup's or a shell region's subtree is drawn in its own surface, never in the window; a
  // modal's over everything else (drawModals).
  if (ownSurface(name) || isModal(name)) return;
  if (!untransformed) {
    SkM44 matrix = viewMatrix(view);
    if (hasPerspective(matrix) || matrix.rc(0, 0) != 1 || matrix.rc(1, 1) != 1 || matrix.rc(0, 1) != 0 ||
        matrix.rc(1, 0) != 0) {
      drawTransformed(tree, tag, SkM44::Translate(ox, oy) * matrix, clip, opacity, draw);
      return;
    }
  }
  Affine map = untransformed ? Affine::translation(frame.origin.x, frame.origin.y) : viewTransform(view);
  float x = ox + map.tx, y = oy + map.ty, w = frame.size.width, h = frame.size.height;
  RenderStats& stats = *draw.stats;
  ++stats.views;
  if (!kImplemented.count(name)) {
    ++stats.unimplemented;
    if (std::find(stats.unimplementedNames.begin(), stats.unimplementedNames.end(), name) ==
        stats.unimplementedNames.end()) {
      stats.unimplementedNames.push_back(name);
    }
  }

  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  // The keyboard's focus ring (keyboard-focus.h), drawn once the view and its subtree are, over
  // them and outside the view, within what clips the view: a band of the system's blue, half
  // opaque, a pixel off the view's edge and following its corners, as a Mac draws its keyboard
  // focus ring around a control.
  struct FocusRing {
    SkCanvas* canvas = nullptr;
    SkRRect box;
    ~FocusRing() {
      if (!canvas) return;
      SkPaint paint(SkColor4f{0.0f, 0.478f, 1.0f, 0.55f});
      paint.setAntiAlias(true);
      paint.setStyle(SkPaint::kStroke_Style);
      paint.setStrokeWidth(3);
      canvas->drawRRect(box, paint);
    }
  } ring;
  if (draw.edit && draw.edit->focusRing == tag && w > 0 && h > 0 &&
      touches(draw, x - kFocusRingReach, y - kFocusRingReach, w + 2 * kFocusRingReach, h + 2 * kFocusRingReach)) {
    auto border = props ? props->resolveBorderMetrics(view.layoutMetrics) : BorderMetrics{};
    // The band's middle 2.5 pixels outside the edge: from 1 to 4 pixels off it.
    constexpr float kOff = 2.5f;
    SkVector radii[4] = {{border.borderRadii.topLeft.horizontal + kOff, border.borderRadii.topLeft.vertical + kOff},
                         {border.borderRadii.topRight.horizontal + kOff, border.borderRadii.topRight.vertical + kOff},
                         {border.borderRadii.bottomRight.horizontal + kOff, border.borderRadii.bottomRight.vertical + kOff},
                         {border.borderRadii.bottomLeft.horizontal + kOff, border.borderRadii.bottomLeft.vertical + kOff}};
    ring.box.setRectRadii(SkRect::MakeXYWH(x - kOff, y - kOff, w + 2 * kOff, h + 2 * kOff), radii);
    ring.canvas = draw.canvas;
  }
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
    SkColor4f bg = props ? faded(props->backgroundColor, 1) : SkColor4f{0, 0, 0, 0};
    std::fprintf(stderr,
                 "[tree] %*s%s tag=%d frame=(%.0f,%.0f %.0fx%.0f) at=(%.0f,%.0f) bg=%.2f,%.2f,%.2f,%.2f opacity=%.2f "
                 "z=%d elevation=%.1f shadows=%zu\n",
                 depth * 2, "", name.c_str(), tag, frame.origin.x, frame.origin.y, w, h, x, y, bg.fR, bg.fG, bg.fB,
                 bg.fA, props ? props->opacity : 1.0f, props && props->zIndex ? *props->zIndex : 0,
                 androidViewPropsOf(*view.props).elevation, props ? props->boxShadow.size() : size_t{0});
    ++depth;
  }
  SkCanvas& canvas = *draw.canvas;
  // What the view clips and the layer it draws in end with it (restoreToCount).
  const int saved = canvas.save();
  Clip childClip = clip;
  bool clips = false;
  Clip paddingBox{x, y, x + w, y + h};
  float childOx = x, childOy = y;
  if (props) {
    opacity *= props->opacity;
    if (opacity <= 0.001f) {
      canvas.restoreToCount(saved);
      return;
    }
    auto android = androidViewPropsOf(*view.props);
    // Its elevation's shadow, cast under it from its outline before anything of it is drawn, as
    // HWUI draws a raised render node's (at its reorder barrier, outside its own layer).
    if (android.elevation > 0) drawElevation(canvas, *props, view.layoutMetrics, x, y, w, h, android.elevation, opacity, draw.scale);
    Group group = groupOf(*props, android, opacity);
    if (group.layer) {
      canvas.saveLayer(nullptr, &group.paint);
      opacity = 1;
    }
    auto overflow = backgroundOverflow(*props);
    if (touches(draw, x - overflow.left, y - overflow.top, w + overflow.left + overflow.right,
                h + overflow.top + overflow.bottom)) {
      drawBackground(canvas, *props, view.layoutMetrics, x, y, w, h, opacity, draw.scale);
    }
    auto border = props->resolveBorderMetrics(view.layoutMetrics);
    // expo-blur's view, as on Android: with a blur target, the target under it blurred; then its
    // tint (all there is without a target).
    if (auto blurTarget = expoBlurTarget(view); blurTarget && drawingBlurTarget == 0) {
      drawBlurred(tree, tag, *blurTarget, x, y, w, h, border, opacity, draw);
    }
    if (auto tint = expoBlurTint(view)) {
      SkPaint paint(faded(*tint, opacity));
      paint.setAntiAlias(true);
      canvas.drawRRect(borderBox(border, x, y, w, h), paint);
    }
    if (props->getClipsContentToBounds() || name == "ScrollView") {
      clips = true;
      const auto& widths = border.borderWidths;
      paddingBox = Clip{x + widths.left, y + widths.top, x + w - widths.right, y + h - widths.bottom};
      childClip = clip.intersect(paddingBox);
    }
  }

  if (name == "ScrollView" && view.state) {
    const auto& state = static_cast<const ConcreteState<ScrollViewState>&>(*view.state);
    childOx -= state.getData().contentOffset.x;
    childOy -= state.getData().contentOffset.y;
  }

  bool visible = !clip.intersect(Clip{x, y, x + w, y + h}).empty() && opacity > 0.001f;
  if (name == "AndroidTextInput" && view.state) {
    if (touches(draw, x, y, w, h)) drawTextInput(view, x, y, w, h, opacity, draw);
    ++stats.texts;
    if (visible) {
      const auto& data = static_cast<const ConcreteState<AndroidTextInputState>&>(*view.state).getData();
      std::string value = data.attributedStringBox.getValue().getString();
      auto inputProps = std::static_pointer_cast<const AndroidTextInputProps>(view.props);
      stats.textContent.push_back(value.empty() ? inputProps->placeholder : shownText(*inputProps, value));
    }
  }

  // A window shown live (docs/adr/0022-window-views.mdx): the compositor draws it here, over what
  // the surface shows, within what clips the view; the host draws nothing of it.
  if (name == kWindowViewComponent && draw.placesWindows && visible) {
    if (auto window = windowViewNumber(view)) {
      Clip inside = clip.intersect(Clip{x, y, x + w, y + h});
      auto radii = props ? props->resolveBorderMetrics(view.layoutMetrics).borderRadii : BorderRadii{};
      float radius = std::max({radii.topLeft.horizontal, radii.topRight.horizontal, radii.bottomLeft.horizontal,
                               radii.bottomRight.horizontal});
      // In the surface's logical pixels.
      const Affine& toSurface = draw.toSurface;
      Rect rect = toSurface.bounds(Rect{{x, y}, {w, h}});
      Rect cut = toSurface.bounds(Rect{{inside.x0, inside.y0}, {inside.x1 - inside.x0, inside.y1 - inside.y0}});
      Clip kept = draw.surfaceClip.intersect(Clip{cut.origin.x, cut.origin.y, cut.getMaxX(), cut.getMaxY()});
      if (!kept.empty()) {
        stats.windowViews.push_back(RenderStats::WindowView{
            *window, rect, Rect{{kept.x0, kept.y0}, {kept.x1 - kept.x0, kept.y1 - kept.y0}}, radius * toSurface.a,
            opacity});
      }
    }
  }

  if (name == "Image" && it->second.picture && touches(draw, x, y, w, h)) {
    drawImage(view, *it->second.picture, x, y, w, h, opacity, draw);
  }

  // An SVG view's elements are its picture (svg.h), drawn over its content box.
  if (isSvgView(name)) {
    if (touches(draw, x, y, w, h)) {
      if (auto picture = svgPicture(tree, tag)) {
        auto content = view.layoutMetrics.getContentFrame();
        canvas.save();
        canvas.clipRect(SkRect::MakeXYWH(x, y, w, h), true);
        canvas.translate(x + content.origin.x, y + content.origin.y);
        SkPaint paint;
        paint.setAlphaf(opacity);
        canvas.drawPicture(picture, nullptr, opacity < 1 ? &paint : nullptr);
        canvas.restore();
      }
    }
    canvas.restoreToCount(saved);
    return;
  }

  if (name == "Paragraph" && view.state) {
    const auto& state = static_cast<const ConcreteState<ParagraphState>&>(*view.state).getData();
    // Glyphs stay within the paragraph's frame (a word wider than the paragraph is cut, not
    // painted over its neighbours), as a view's content is clipped to its bounds on Android: a
    // paragraph outside the damage is not laid out.
    if (touches(draw, x, y, w, h)) {
      auto content = view.layoutMetrics.getContentFrame();
      auto layout = TextEngine::shared().layout(state.attributedString, state.paragraphAttributes, content.size.width);
      canvas.save();
      canvas.clipRect(SkRect::MakeXYWH(x, y, w, h), true);
      drawGlyphs(draw, layout, x + content.origin.x, y + content.origin.y, opacity);
      canvas.restore();
    }
    ++stats.texts;
    if (visible) stats.textContent.push_back(state.attributedString.getString());
  }

  if (isMaskedView(name)) {
    drawMasked(tree, tag, x, y, w, h, clip, opacity, draw);
    canvas.restoreToCount(saved);
    return;
  }

  // A scroll view stretched past its end (overscroll, input.h), as Android 12's stretch draws its
  // render node: its content scaled along the axis by one and the stretch, from the edge it is
  // pulled past, within what it clips to.
  std::optional<Affine> stretch;
  if (name == "ScrollView") {
    Point amount = tree.stretch(tag);
    if (amount.x != 0 || amount.y != 0) {
      float bx = paddingBox.x0, by = paddingBox.y0, bw = paddingBox.x1 - paddingBox.x0, bh = paddingBox.y1 - paddingBox.y0;
      float edgeX = amount.x < 0 ? bx : bx + bw, edgeY = amount.y < 0 ? by : by + bh;
      float sx = 1 + std::abs(amount.x), sy = 1 + std::abs(amount.y);
      stretch = Affine{sx, 0, 0, sy, edgeX * (1 - sx), edgeY * (1 - sy)};
    }
  }
  if (clips && props) clipToPaddingBox(canvas, *props, view.layoutMetrics, x, y, w, h);
  for (Tag child : it->second.children) {
    drawChild(tree, child, childOx, childOy, childClip, opacity, draw, stretch ? &*stretch : nullptr);
  }
  canvas.restoreToCount(saved);
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

// Where canvases are drawn (docs/adr/0031-gpu-buffers.mdx, decision 12): on the GPU while a
// painter is set and no CpuPaintScope holds this thread, else on the CPU.
OxyOSPaint* gpuPaintSet = nullptr;
thread_local int cpuOnly = 0;

GrDirectContext* activeContext() { return cpuOnly == 0 ? skiaContext(gpuPaintSet) : nullptr; }

// A canvas value (premultiplied 0xAARRGGBB) as the colour Skia clears with.
SkColor4f fromPremultiplied(uint32_t value) {
  float a = (value >> 24) / 255.0f;
  if (a <= 0) return SkColor4f{0, 0, 0, 0};
  return SkColor4f{((value >> 16) & 0xff) / 255.0f / a, ((value >> 8) & 0xff) / 255.0f / a, (value & 0xff) / 255.0f / a,
                   a};
}

SkImageInfo infoOf(int width, int height) {
  return SkImageInfo::Make(width, height, kBGRA_8888_SkColorType, kPremul_SkAlphaType);
}

// A GPU canvas's surface: rows top to bottom, as the surface's buffers hold them (Ganesh's default
// origin is the bottom left, OpenGL's).
sk_sp<SkSurface> gpuSurface(GrDirectContext* context, int width, int height) {
  return SkSurfaces::RenderTarget(context, skgpu::Budgeted::kNo, infoOf(width, height), 0, kTopLeft_GrSurfaceOrigin,
                                  nullptr);
}
// The surface a canvas is drawn through: its GPU surface, or its pixels wrapped.
sk_sp<SkSurface> surfaceOf(Canvas& canvas) {
  if (canvas.gpu) return canvas.gpu->surface;
  if (canvas.width <= 0 || canvas.height <= 0) return nullptr;
  return SkSurfaces::WrapPixels(infoOf(canvas.width, canvas.height), canvas.pixels.data(),
                                static_cast<size_t>(canvas.width) * 4);
}

// A canvas drawn on the other kind than the one in use now is moved to it: its pixels uploaded to a
// GPU surface, or read back from one.
void adopt(Canvas& canvas) {
  GrDirectContext* context = activeContext();
  if (canvas.width <= 0 || canvas.height <= 0) return;
  if (context && !canvas.gpu) {
    auto surface = gpuSurface(context, canvas.width, canvas.height);
    if (!surface) return;
    surface->writePixels(SkPixmap(infoOf(canvas.width, canvas.height), canvas.pixels.data(),
                                  static_cast<size_t>(canvas.width) * 4),
                         0, 0);
    canvas.gpu = std::make_shared<GpuCanvas>(GpuCanvas{std::move(surface)});
    canvas.pixels.clear();
  } else if (!context && canvas.gpu) {
    canvas.pixels = canvasPixels(canvas);
    canvas.gpu.reset();
  }
}

// The canvas `width` × `height` pixels, every one `value`.
void allocate(Canvas& canvas, int width, int height, uint32_t value) {
  canvas.width = width;
  canvas.height = height;
  canvas.gpu.reset();
  canvas.pixels.clear();
  if (GrDirectContext* context = activeContext(); context && width > 0 && height > 0) {
    if (auto surface = gpuSurface(context, width, height)) {
      surface->getCanvas()->clear(fromPremultiplied(value));
      canvas.gpu = std::make_shared<GpuCanvas>(GpuCanvas{std::move(surface)});
      return;
    }
  }
  canvas.pixels.assign(static_cast<size_t>(std::max(width, 0)) * std::max(height, 0), value);
}

// The canvas at another size: its pixels kept where both have them (top left), the rest `value`.
void resize(Canvas& canvas, int width, int height, uint32_t value) {
  Canvas before = canvas;
  allocate(canvas, width, height, value);
  auto source = surfaceOf(before);
  auto target = surfaceOf(canvas);
  if (!source || !target) return;
  SkPaint copy;
  copy.setBlendMode(SkBlendMode::kSrc);
  source->draw(target->getCanvas(), 0, 0, &copy);
}

// A canvas's pixels as an image, to draw (a modal's last frame).
sk_sp<SkImage> imageOf(const Canvas& canvas) {
  if (canvas.gpu) return canvas.gpu->surface->makeImageSnapshot();
  if (canvas.width <= 0 || canvas.height <= 0) return nullptr;
  return SkImages::RasterFromPixmap(
      SkPixmap(infoOf(canvas.width, canvas.height), canvas.pixels.data(), static_cast<size_t>(canvas.width) * 4),
      nullptr, nullptr);
}

// Draws onto `canvas` in logical pixels at `scale`, only within the device pixels `region` when
// one is given (a damaged rectangle: what is drawn there is what a whole frame draws).
template <typename Body>
void drawOnto(Canvas& canvas, float scale, std::optional<SkIRect> region, Body body) {
  auto surface = surfaceOf(canvas);
  if (!surface) {
    // Nothing to paint; the walk still gathers the stats.
    SkCanvas none(0, 0);
    body(none);
    return;
  }
  SkCanvas* target = surface->getCanvas();
  target->save();
  if (region) target->clipIRect(*region);
  target->scale(scale, scale);
  body(*target);
  target->restore();
}

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
void drawModalOnto(const ViewTree& tree, Tag modal, const Display& display, Draw& draw) {
  Clip window{0, 0, display.width, display.height};
  const auto& entry = tree.views().at(modal);
  auto props = std::static_pointer_cast<const ModalHostViewProps>(entry.view.props);
  if (!props->transparent) {
    draw.canvas->drawRect(SkRect::MakeWH(display.width, display.height), SkPaint(SkColors::kWhite));
  }
  for (Tag child : entry.children) drawView(tree, child, 0, 0, window, 1.0f, draw);
}

// The modals over the window's content; one animating in is drawn as its window is, moved and
// faded, and those animating out after them, from their last frames.
void drawModals(const ViewTree& tree, const Display& display, Draw& draw) {
  SkCanvas& canvas = *draw.canvas;
  for (Tag modal : modals(tree)) {
    ModalPresentation presented = tree.presentation(modal);
    if (presented == ModalPresentation{}) {
      drawModalOnto(tree, modal, display, draw);
      continue;
    }
    Draw inside = draw;
    inside.placesWindows = false;
    canvas.saveLayerAlphaf(nullptr, presented.alpha);
    canvas.translate(0, presented.offset);
    drawModalOnto(tree, modal, display, inside);
    canvas.restore();
  }
  for (const auto& leaving : tree.leaving()) {
    if (!leaving.frame || leaving.frame->width <= 0) continue;
    auto image = imageOf(*leaving.frame);
    if (!image) continue;
    SkPaint paint;
    paint.setAlphaf(leaving.presentation.alpha);
    canvas.drawImageRect(image, SkRect::MakeXYWH(0, leaving.presentation.offset, display.width, display.height),
                         SkSamplingOptions(SkFilterMode::kLinear), &paint);
  }
}

// Draws the window's content and its modals.
void drawWindow(const ViewTree& tree, const Display& display, SkCanvas& canvas, RenderStats& stats,
                const TextInputEdit& edit) {
  Draw draw{&canvas, display.scale, &stats, &edit};
  drawView(tree, tree.root(), 0, 0, Clip{0, 0, display.width, display.height}, 1.0f, draw);
  drawModals(tree, display, draw);
}

void drawRegionChildren(const ViewTree& tree, Tag region, Point at, const Display& display, SkCanvas& canvas,
                        RenderStats& stats, const TextInputEdit& edit) {
  auto it = tree.views().find(region);
  if (it == tree.views().end()) return;
  Draw draw{&canvas, display.scale, &stats, &edit};
  for (Tag child : it->second.children) {
    drawView(tree, child, at.x, at.y, Clip{0, 0, display.width, display.height}, 1.0f, draw);
  }
}

// Each damaged rectangle in device pixels, within the canvas.
std::vector<SkIRect> damagedPixels(const std::vector<Rect>& damage, const Display& display) {
  std::vector<SkIRect> out;
  int width = display.pixelWidth(), height = display.pixelHeight();
  for (const auto& rect : damage) {
    int x0 = std::max(0, static_cast<int>(std::floor(rect.origin.x * display.scale)));
    int y0 = std::max(0, static_cast<int>(std::floor(rect.origin.y * display.scale)));
    int x1 = std::min(width, static_cast<int>(std::ceil((rect.origin.x + rect.size.width) * display.scale)));
    int y1 = std::min(height, static_cast<int>(std::ceil((rect.origin.y + rect.size.height) * display.scale)));
    if (x1 > x0 && y1 > y0) out.push_back(SkIRect::MakeLTRB(x0, y0, x1, y1));
  }
  return out;
}

}  // namespace

std::vector<Tag> modals(const ViewTree& tree) {
  std::vector<Tag> out;
  if (tree.root() >= 0) collectModals(tree, tree.root(), out);
  return out;
}

RenderStats drawModal(const ViewTree& tree, Tag modal, const Display& display, Canvas& canvas,
                      const TextInputEdit& edit) {
  allocate(canvas, display.pixelWidth(), display.pixelHeight(), 0);
  RenderStats stats;
  if (!tree.views().count(modal)) return stats;
  drawOnto(canvas, display.scale, std::nullopt, [&](SkCanvas& target) {
    Draw draw{&target, display.scale, &stats, &edit};
    drawModalOnto(tree, modal, display, draw);
  });
  return stats;
}

Rect Affine::bounds(const Rect& rect) const {
  return Rect::boundingRect(apply(rect.origin), apply(Point{rect.getMaxX(), rect.getMinY()}),
                            apply(Point{rect.getMinX(), rect.getMaxY()}), apply(Point{rect.getMaxX(), rect.getMaxY()}));
}

Affine viewTransform(const ShadowView& view) {
  SkM44 m = viewMatrix(view);
  // Its projection onto the screen's plane, as a press is hit and damage is reckoned on it.
  return Affine{m.rc(0, 0), m.rc(1, 0), m.rc(0, 1), m.rc(1, 1), m.rc(0, 3), m.rc(1, 3)};
}

Point transformOffset(const ShadowView& view) {
  const auto& frame = view.layoutMetrics.frame;
  Point centre{frame.size.width / 2, frame.size.height / 2};
  Point moved = viewTransform(view).apply(centre);
  return Point{moved.x - frame.origin.x - centre.x, moved.y - frame.origin.y - centre.y};
}

Rect inkBounds(const ShadowView& view) {
  Rect area{{0, 0}, view.layoutMetrics.frame.size};
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  if (!props) return area;
  auto overflow = backgroundOverflow(*props);
  float filter = filterOverflow(*props);
  // An elevation's shadow: its spot shadow's blur grows with the height (the light's 800 dp over
  // its 600), and it moves away from the light by as much again for a view at a window's edge.
  float elevation = androidViewPropsOf(*view.props).elevation;
  if (elevation > 0) filter = std::max(filter, elevation * 4 + 8);
  float left = std::max(overflow.left, filter), top = std::max(overflow.top, filter);
  float right = std::max(overflow.right, filter), bottom = std::max(overflow.bottom, filter);
  return Rect{{-left, -top}, {area.size.width + left + right, area.size.height + top + bottom}};
}

RenderStats draw(const ViewTree& tree, const Display& display, Canvas& canvas, const TextInputEdit& edit) {
  allocate(canvas, display.pixelWidth(), display.pixelHeight(), display.background);
  RenderStats stats;
  drawOnto(canvas, display.scale, std::nullopt,
           [&](SkCanvas& target) { drawWindow(tree, display, target, stats, edit); });
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
  // What drawView and the background layers read from a view's props, each with its own layout.
  return a->opacity == b->opacity && a->backgroundColor == b->backgroundColor &&
         a->resolveBorderMetrics(before.layoutMetrics) == b->resolveBorderMetrics(after.layoutMetrics) &&
         a->getClipsContentToBounds() == b->getClipsContentToBounds() && a->transform == b->transform &&
         a->transformOrigin == b->transformOrigin && a->boxShadow == b->boxShadow && a->filter == b->filter &&
         a->backgroundImage == b->backgroundImage && a->backgroundSize == b->backgroundSize &&
         a->backgroundPosition == b->backgroundPosition && a->backgroundRepeat == b->backgroundRepeat &&
         a->outlineColor == b->outlineColor && a->outlineOffset == b->outlineOffset &&
         a->outlineStyle == b->outlineStyle && a->outlineWidth == b->outlineWidth &&
         a->mixBlendMode == b->mixBlendMode &&
         a->yogaStyle.border(facebook::yoga::Edge::All) == b->yogaStyle.border(facebook::yoga::Edge::All) &&
         a->borderColors.all == b->borderColors.all && a->shadowColor == b->shadowColor &&
         androidViewPropsOf(*before.props) == androidViewPropsOf(*after.props);
}

bool drawsFlat(const ShadowView& view) {
  std::string name = view.componentName ? view.componentName : "";
  if (name != "View" && name != "RootView") return false;
  auto props = std::dynamic_pointer_cast<const ViewProps>(view.props);
  if (!props) return true;
  if (!(props->transform == Transform::Identity())) return false;
  if (!props->boxShadow.empty() || !props->filter.empty() || !props->backgroundImage.empty() ||
      props->outlineWidth != 0 || props->mixBlendMode != BlendMode::Normal ||
      androidViewPropsOf(*view.props) != AndroidViewProps{}) {
    return false;
  }
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
    resize(canvas, width, height, 0xffffffffu);
  }
  RenderStats stats;
  bool counted = false;
  for (const auto& region : damagedPixels(damage, display)) {
    RenderStats drawn;
    drawOnto(canvas, display.scale, region, [&](SkCanvas& target) {
      // The region starts as a new window does, then everything over it is drawn again.
      target.drawColor(fromPremultiplied(display.background), SkBlendMode::kSrc);
      drawWindow(tree, display, target, drawn, edit);
    });
    if (!counted) {
      stats = std::move(drawn);
      counted = true;
    }
  }
  if (!counted) {
    // Nothing to paint; the stats are still the window's.
    drawOnto(canvas, display.scale, SkIRect::MakeEmpty(),
             [&](SkCanvas& target) { drawWindow(tree, display, target, stats, edit); });
  }
  return stats;
}

RenderStats drawPopup(const ViewTree& tree, Tag popup, const Display& display, Canvas& canvas,
                      const TextInputEdit& edit) {
  return drawRegion(tree, popup, Point{0, 0}, display, canvas, edit);
}

RenderStats drawRegion(const ViewTree& tree, Tag region, Point at, const Display& display, Canvas& canvas,
                       const TextInputEdit& edit) {
  allocate(canvas, display.pixelWidth(), display.pixelHeight(), 0);
  RenderStats stats;
  drawOnto(canvas, display.scale, std::nullopt,
           [&](SkCanvas& target) { drawRegionChildren(tree, region, at, display, target, stats, edit); });
  return stats;
}

RenderStats drawRegionDamaged(const ViewTree& tree, Tag region, Point at, const Display& display, Canvas& canvas,
                              const TextInputEdit& edit, const std::vector<Rect>& damage) {
  adopt(canvas);
  if (canvas.width != display.pixelWidth() || canvas.height != display.pixelHeight()) {
    return drawRegion(tree, region, at, display, canvas, edit);
  }
  if (!tree.views().count(region)) return {};
  RenderStats stats;
  bool counted = false;
  for (const auto& pixels : damagedPixels(damage, display)) {
    RenderStats drawn;
    drawOnto(canvas, display.scale, pixels, [&](SkCanvas& target) {
      // The region starts transparent, as a new region does, then everything over it is drawn again.
      target.drawColor(SkColors::kTransparent, SkBlendMode::kSrc);
      drawRegionChildren(tree, region, at, display, target, drawn, edit);
    });
    if (!counted) {
      stats = std::move(drawn);
      counted = true;
    }
  }
  if (!counted) {
    drawOnto(canvas, display.scale, SkIRect::MakeEmpty(),
             [&](SkCanvas& target) { drawRegionChildren(tree, region, at, display, target, stats, edit); });
  }
  return stats;
}

RenderStats drawDragImage(const ViewTree& tree, Tag view, const Display& display, Canvas& canvas) {
  allocate(canvas, display.pixelWidth(), display.pixelHeight(), 0);
  RenderStats stats;
  auto it = tree.views().find(view);
  if (it == tree.views().end()) return stats;
  // The view at the canvas's origin, translucent, as a Mac draws a dragged item's image.
  const auto& origin = it->second.view.layoutMetrics.frame.origin;
  TextInputEdit edit;
  drawOnto(canvas, display.scale, std::nullopt, [&](SkCanvas& target) {
    Draw draw{&target, display.scale, &stats, &edit};
    draw.placesWindows = false;
    drawView(tree, view, -origin.x, -origin.y, Clip{0, 0, display.width, display.height}, 0.75f, draw);
  });
  return stats;
}

RenderStats drawTextDragImage(const ShadowView& view, const std::string& value, size_t from, size_t to, Point scroll,
                              Rect bounds, const Display& display, Canvas& canvas) {
  allocate(canvas, display.pixelWidth(), display.pixelHeight(), 0);
  RenderStats stats;
  auto props = std::static_pointer_cast<const AndroidTextInputProps>(view.props);
  if (!props || props->secureTextEntry || from >= to) return stats;
  auto content = view.layoutMetrics.getContentFrame();
  auto layout = layoutInput(*props, inputText(*props, value), content.size.width);
  TextInputGeometry geometry(view, value);
  // Where drawTextInput puts the text and the range's rectangles, in the view's coordinates.
  float left = content.origin.x - scroll.x;
  float top = content.origin.y - scroll.y + (props->multiline ? 0 : (content.size.height - layout.height) / 2);
  float boxTop = content.origin.y - scroll.y;
  TextInputEdit edit;
  drawOnto(canvas, display.scale, std::nullopt, [&](SkCanvas& target) {
    Draw draw{&target, display.scale, &stats, &edit};
    draw.placesWindows = false;
    target.translate(-bounds.origin.x, -bounds.origin.y);
    SkPathBuilder within;
    for (const auto& rect : geometry.span(from, to)) {
      within.addRect(SkRect::MakeXYWH(left + rect.origin.x, boxTop + rect.origin.y, rect.size.width, rect.size.height));
    }
    target.clipPath(within.detach(), true);
    drawGlyphs(draw, layout, left, top, 0.75f);
  });
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

void setGpuPaint(OxyOSPaint* paint) { gpuPaintSet = paint; }

OxyOSPaint* gpuPaint() { return gpuPaintSet; }

CpuPaintScope::CpuPaintScope() { ++cpuOnly; }

CpuPaintScope::~CpuPaintScope() { --cpuOnly; }


std::vector<uint32_t> canvasPixels(const Canvas& canvas) {
  if (!canvas.gpu) return canvas.pixels;
  std::vector<uint32_t> pixels(static_cast<size_t>(std::max(canvas.width, 0)) * std::max(canvas.height, 0));
  if (!canvas.gpu->surface->readPixels(infoOf(canvas.width, canvas.height), pixels.data(),
                                       static_cast<size_t>(canvas.width) * 4, 0, 0)) {
    return {};
  }
  return pixels;
}

FrameDifference compareFrames(const std::vector<uint32_t>& a, const std::vector<uint32_t>& b, int width) {
  FrameDifference difference;
  difference.pixels = std::min(a.size(), b.size());
  if (a.size() != b.size() || width <= 0) {
    difference.largest = difference.blocks = 255;
    difference.beyondThree = std::max(a.size(), b.size());
    return difference;
  }
  auto channel = [](uint32_t pixel, int shift) { return static_cast<int>((pixel >> shift) & 0xff); };
  for (size_t i = 0; i < a.size(); ++i) {
    if (a[i] == b[i]) continue;
    int largest = 0;
    for (int shift = 0; shift < 32; shift += 8) largest = std::max(largest, std::abs(channel(a[i], shift) - channel(b[i], shift)));
    difference.largest = std::max(difference.largest, largest);
    difference.beyondThree += largest > 3;
  }
  // Each 4 × 4 block's mean, channel by channel: anti-aliasing worked out otherwise averages away,
  // an edge a pixel off, a glyph missing or a layer misplaced does not.
  const int height = static_cast<int>(a.size() / static_cast<size_t>(width));
  for (int by = 0; by < height; by += 4) {
    for (int bx = 0; bx < width; bx += 4) {
      int sums[2][4] = {};
      for (int y = by; y < std::min(by + 4, height); ++y) {
        for (int x = bx; x < std::min(bx + 4, width); ++x) {
          size_t at = static_cast<size_t>(y) * width + x;
          for (int c = 0; c < 4; ++c) {
            sums[0][c] += channel(a[at], c * 8);
            sums[1][c] += channel(b[at], c * 8);
          }
        }
      }
      int count = (std::min(by + 4, height) - by) * (std::min(bx + 4, width) - bx);
      for (int c = 0; c < 4; ++c) {
        difference.blocks = std::max(difference.blocks, std::abs(sums[0][c] - sums[1][c]) / count);
      }
    }
  }
  return difference;
}

}  // namespace oxyos
