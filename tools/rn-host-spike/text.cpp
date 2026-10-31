#include "text.h"

#include <react/renderer/graphics/Color.h>
#include <react/utils/hash_combine.h>

#include <hb-ot.h>
#include <hb.h>
#include <unicode/brkiter.h>
#include <unicode/locid.h>
#include <unicode/ubrk.h>
#include <unicode/unistr.h>
#include <unicode/utext.h>

#include <algorithm>
#include <cmath>
#include <deque>
#include <limits>
#include <list>
#include <unordered_map>
#include <iostream>
#include <map>
#include <mutex>
#include <optional>
#include <tuple>

using namespace facebook::react;

namespace oxyos {

// HarfBuzz's positions are in units of this fraction of a logical pixel: a font's scale is its
// size times this, so advances keep their fractions, as Minikin's floats do.
constexpr float kShapingUnits = 65536.0f;

namespace {

uint32_t argb(const SharedColor& color) {
  if (!color) return 0xff000000;
  auto c = colorComponentsFromColor(color);
  auto byte = [](float v) { return static_cast<uint32_t>(std::lround(std::clamp(v, 0.0f, 1.0f) * 255)); };
  return byte(c.alpha) << 24 | byte(c.red) << 16 | byte(c.green) << 8 | byte(c.blue);
}

// Decodes one UTF-8 code point at `i`, advancing it.
uint32_t next(const std::string& text, size_t& i) {
  auto b = static_cast<unsigned char>(text[i]);
  int extra = b < 0x80 ? 0 : b < 0xe0 ? 1 : b < 0xf0 ? 2 : 3;
  uint32_t cp = extra == 0 ? b : b & (0x3f >> extra);
  ++i;
  for (int k = 0; k < extra && i < text.size(); ++k, ++i) {
    cp = cp << 6 | (static_cast<unsigned char>(text[i]) & 0x3f);
  }
  return cp;
}

bool covers(const Face& face, uint32_t cp) {
  hb_codepoint_t glyph = 0;
  return hb_font_get_nominal_glyph(face.hb, cp, &glyph) && glyph != 0;
}

// The OpenType features `fontVariant` asks for, as Android's ReactTypefaceUtils turns each into
// Paint.setFontFeatureSettings.
std::vector<hb_feature_t> features(const TextAttributes& attributes) {
  std::vector<hb_feature_t> out;
  if (!attributes.fontVariant) return out;
  auto variant = static_cast<int>(*attributes.fontVariant);
  auto add = [&](const char* tag) {
    out.push_back(hb_feature_t{hb_tag_from_string(tag, 4), 1, HB_FEATURE_GLOBAL_START, HB_FEATURE_GLOBAL_END});
  };
  if (variant & static_cast<int>(FontVariant::SmallCaps)) add("smcp");
  if (variant & static_cast<int>(FontVariant::OldstyleNums)) add("onum");
  if (variant & static_cast<int>(FontVariant::LiningNums)) add("lnum");
  if (variant & static_cast<int>(FontVariant::TabularNums)) add("tnum");
  if (variant & static_cast<int>(FontVariant::ProportionalNums)) add("pnum");
  static const char* kStylistic[] = {"ss01", "ss02", "ss03", "ss04", "ss05", "ss06", "ss07", "ss08", "ss09", "ss10",
                                     "ss11", "ss12", "ss13", "ss14", "ss15", "ss16", "ss17", "ss18", "ss19", "ss20"};
  for (int k = 0; k < 20; ++k) {
    if (variant & static_cast<int>(FontVariant::StylisticOne) << k) add(kStylistic[k]);
  }
  return out;
}

// A fragment's string as Android shows it: `textTransform` applied (React Native's TextTransform,
// with the default locale's case rules).
std::string transformed(const std::string& text, const TextAttributes& attributes) {
  if (!attributes.textTransform || *attributes.textTransform == TextTransform::None ||
      *attributes.textTransform == TextTransform::Unset) {
    return text;
  }
  icu::UnicodeString string = icu::UnicodeString::fromUTF8(text);
  switch (*attributes.textTransform) {
    case TextTransform::Uppercase:
      string.toUpper(icu::Locale::getDefault());
      break;
    case TextTransform::Lowercase:
      string.toLower(icu::Locale::getDefault());
      break;
    case TextTransform::Capitalize: {
      // Each word's first letter in upper case, the rest kept, as Android's capitalize.
      UErrorCode status = U_ZERO_ERROR;
      std::unique_ptr<icu::BreakIterator> words(
          icu::BreakIterator::createWordInstance(icu::Locale::getDefault(), status));
      if (U_FAILURE(status)) break;
      words->setText(string);
      icu::UnicodeString out;
      int32_t start = words->first();
      for (int32_t end = words->next(); end != icu::BreakIterator::DONE; start = end, end = words->next()) {
        icu::UnicodeString word(string, start, end - start);
        int32_t first = word.moveIndex32(0, 1);
        icu::UnicodeString head(word, 0, first);
        head.toUpper(icu::Locale::getDefault());
        out += head;
        out += icu::UnicodeString(word, first);
      }
      string = out;
      break;
    }
    default:
      break;
  }
  std::string out;
  string.toUTF8String(out);
  return out;
}

}  // namespace

struct TextEngine::Impl {
  std::map<std::string, std::string> families;
  std::vector<std::string> fallbacks;
  // Each file's HarfBuzz face, opened once, and what it is: its axes of weight and optical size,
  // whether it is italic, and its weight (OS/2's usWeightClass).
  struct File {
    hb_face_t* hb = nullptr;
    bool weightAxis = false, opticalAxis = false, italic = false;
    int weight = 400;
  };
  std::map<std::string, File> files;
  std::map<std::tuple<std::string, int, int, bool>, Face> faces;
  std::recursive_mutex mutex;
  // ICU's line break iterator, opened once (its rules are loaded when it is).
  UBreakIterator* lineBreaks = nullptr;

  // Paragraphs laid out lately, the most recent first: what Yoga measured is drawn without
  // shaping it again, and a frame damaging a paragraph draws it from here.
  struct LayoutKey {
    AttributedString text;
    ParagraphAttributes paragraph;
    float maxWidth;
    bool operator==(const LayoutKey& other) const {
      // NaN never equals itself; an unbounded width is +inf.
      return maxWidth == other.maxWidth && paragraph == other.paragraph && text == other.text;
    }
  };
  struct LayoutHash {
    size_t operator()(const LayoutKey& key) const {
      return facebook::react::hash_combine(key.text, key.paragraph, key.maxWidth);
    }
  };
  static constexpr size_t kCachedLayouts = 512;
  std::list<std::pair<LayoutKey, ParagraphLayout>> recent;
  std::unordered_map<LayoutKey, std::list<std::pair<LayoutKey, ParagraphLayout>>::iterator, LayoutHash> layouts;

  File* file(const std::string& path) {
    if (auto it = files.find(path); it != files.end()) return it->second.hb ? &it->second : nullptr;
    File entry;
    if (hb_blob_t* blob = hb_blob_create_from_file_or_fail(path.c_str())) {
      hb_face_t* face = hb_face_create(blob, 0);
      hb_blob_destroy(blob);
      // A file that is not a font gives an empty face: no glyphs.
      if (hb_face_get_glyph_count(face) > 0) {
        entry.hb = face;
      } else {
        hb_face_destroy(face);
      }
    }
    if (!entry.hb) {
      files[path] = entry;
      return nullptr;
    }
    hb_ot_var_axis_info_t axis;
    entry.weightAxis = hb_ot_var_find_axis_info(entry.hb, HB_TAG('w', 'g', 'h', 't'), &axis);
    entry.opticalAxis = hb_ot_var_find_axis_info(entry.hb, HB_TAG('o', 'p', 's', 'z'), &axis);
    hb_blob_t* os2 = hb_face_reference_table(entry.hb, HB_TAG('O', 'S', '/', '2'));
    unsigned length = 0;
    const char* data = hb_blob_get_data(os2, &length);
    auto u16 = [data](unsigned at) {
      return static_cast<unsigned>(static_cast<unsigned char>(data[at]) << 8 | static_cast<unsigned char>(data[at + 1]));
    };
    if (data && length >= 64) {
      entry.weight = static_cast<int>(u16(4));
      // fsSelection: ITALIC (bit 0) or OBLIQUE (bit 9).
      entry.italic = (u16(62) & 0x201) != 0;
    }
    hb_blob_destroy(os2);
    return &(files[path] = entry);
  }

  Face* face(const std::string& path, int weight, float size, bool italic) {
    auto key = std::make_tuple(path, weight, static_cast<int>(std::lround(size * 64)), italic);
    if (auto it = faces.find(key); it != faces.end()) return it->second.hb ? &it->second : nullptr;
    File* source = file(path);
    if (!source) {
      if (!path.empty()) std::cerr << "[text] cannot open font " << path << "\n";
      faces[key] = Face{};
      return nullptr;
    }
    Face entry;
    entry.path = path;
    entry.weight = weight;
    entry.variable = source->weightAxis;
    entry.size = size;
    entry.fakeItalic = italic && !source->italic;
    // Minikin's fake bold (FontFamily::computeFakery): a weight of 600 or more asked of a font at
    // least 200 lighter, its weight not on an axis.
    entry.fakeBold = !source->weightAxis && weight >= 600 && weight - source->weight >= 200;
    // The optical size the font is set at, as CSS's font-optical-sizing: auto sets it (Chromium,
    // and Apple's text), on a font with the axis: its size in pixels.
    if (source->opticalAxis) entry.opticalSize = size;
    entry.hb = hb_font_create(source->hb);
    hb_ot_font_set_funcs(entry.hb);
    auto scale = static_cast<int>(std::lround(size * kShapingUnits));
    hb_font_set_scale(entry.hb, scale, scale);
    // The weight on the `wght` axis and the optical size on `opsz`, which HarfBuzz (and Skia,
    // drawing them) clamps to their ranges.
    std::vector<hb_variation_t> variations;
    if (source->weightAxis) variations.push_back({HB_TAG('w', 'g', 'h', 't'), static_cast<float>(weight)});
    if (source->opticalAxis) variations.push_back({HB_TAG('o', 'p', 's', 'z'), size});
    if (!variations.empty()) {
      hb_font_set_variations(entry.hb, variations.data(), static_cast<unsigned>(variations.size()));
    }
    auto metric = [&](hb_ot_metrics_tag_t tag) -> std::optional<float> {
      hb_position_t position = 0;
      if (!hb_ot_metrics_get_position(entry.hb, tag, &position)) return std::nullopt;
      return position / kShapingUnits;
    };
    entry.ascent = std::ceil(metric(HB_OT_METRICS_TAG_HORIZONTAL_ASCENDER).value_or(size));
    entry.descent = std::ceil(-metric(HB_OT_METRICS_TAG_HORIZONTAL_DESCENDER).value_or(0));
    entry.capHeight = metric(HB_OT_METRICS_TAG_CAP_HEIGHT).value_or(0);
    entry.xHeight = metric(HB_OT_METRICS_TAG_X_HEIGHT).value_or(0);
    // HWUI's drawTextDecorations: the font's own underline where it states one (post's, below the
    // baseline), else Skia's standard offset (1/9 of the size) and thickness (1/18); the
    // strike-through 6/21 of the size above the baseline, 1/18 thick.
    auto thickness = metric(HB_OT_METRICS_TAG_UNDERLINE_SIZE);
    auto offset = metric(HB_OT_METRICS_TAG_UNDERLINE_OFFSET);
    entry.underlineThickness = thickness && *thickness > 0 ? *thickness : size / 18;
    entry.underlineTop = offset && thickness && *thickness > 0 ? -*offset : size / 9;
    entry.strikeTop = -size * 6 / 21;
    entry.strikeThickness = size / 18;
    return &(faces[key] = std::move(entry));
  }

  std::string pathFor(const std::string& family) {
    auto it = families.find(family);
    if (it != families.end()) return it->second;
    it = families.find("Inter");
    if (it != families.end()) return it->second;
    return fallbacks.empty() ? std::string() : fallbacks.front();
  }
};

TextEngine& TextEngine::shared() {
  static TextEngine engine;
  return engine;
}

TextEngine::TextEngine() : impl_(std::make_unique<Impl>()) {}

bool TextEngine::registerFamily(const std::string& family, const std::string& path) {
  std::lock_guard lock(impl_->mutex);
  if (!impl_->file(path)) {
    std::cerr << "[text] cannot open font " << path << " for " << family << "\n";
    return false;
  }
  impl_->families[family] = path;
  return true;
}

void TextEngine::addFallback(const std::string& path) {
  std::lock_guard lock(impl_->mutex);
  impl_->fallbacks.push_back(path);
}

namespace {

// A run of text between two places a line may break (ICU's line break iterator, as Android's
// minikin breaks lines), in one fragment's attributes, shaped with one face per segment: a word
// with its trailing spaces, part of one (a fragment ends inside it), or a hard line break.
struct Token {
  std::vector<Glyph> glyphs;
  std::vector<size_t> clusters;  // each glyph's cluster: its byte offset in the paragraph's string
  float width = 0;          // including trailing spaces
  float inkWidth = 0;       // excluding trailing spaces
  float ascent = 0, descent = 0, lineHeight = NAN;
  float capHeight = 0, xHeight = 0;
  uint32_t color = 0xff000000;
  bool newline = false;
  // Whether a line may begin with this token: false where only a fragment's end, not a break
  // opportunity, separates it from the token before.
  bool breakBefore = true;
  std::string text;
  size_t start = 0;  // byte offset in the paragraph's string
  std::vector<std::pair<size_t, float>> stops;  // each cluster's offset and its x in the token
  const Face* face = nullptr;  // the fragment's own face, which an ellipsis is shaped in
  float spacing = 0;
  float size = 14;  // the fragment's font size, which Minikin's line penalty is reckoned from
  // What the fragment draws with its glyphs (GlyphRun).
  bool underline = false, strikethrough = false;
  uint32_t shadowColor = 0;
  float shadowDx = 0, shadowDy = 0, shadowRadius = 0;
  const std::vector<hb_feature_t>* features = nullptr;
};

// Shapes `length` bytes of `text` from `offset` in `face` (HarfBuzz, the font's unhinted
// advances), calling `glyph(id, cluster, x offset, advance)` for each glyph, in logical pixels.
template <typename Each>
void shape(const Face& face, const std::string& text, size_t offset, size_t length,
           const std::vector<hb_feature_t>* features, Each glyph) {
  hb_buffer_t* buffer = hb_buffer_create();
  hb_buffer_add_utf8(buffer, text.data(), static_cast<int>(text.size()), static_cast<unsigned>(offset),
                     static_cast<int>(length));
  hb_buffer_guess_segment_properties(buffer);
  hb_shape(face.hb, buffer, features && !features->empty() ? features->data() : nullptr,
           features ? static_cast<unsigned>(features->size()) : 0);
  unsigned count = 0;
  auto* infos = hb_buffer_get_glyph_infos(buffer, &count);
  auto* positions = hb_buffer_get_glyph_positions(buffer, &count);
  for (unsigned g = 0; g < count; ++g) {
    glyph(infos[g].codepoint, infos[g].cluster, positions[g].x_offset / kShapingUnits,
          positions[g].x_advance / kShapingUnits);
  }
  hb_buffer_destroy(buffer);
}

// The token's clusters' x positions, with its end: where it can be cut.
float stopX(const Token& token, size_t index) {
  return index < token.stops.size() ? token.stops[index].second : token.width;
}

// `token` cut where its cluster `index` (not the first) starts: the part before and the part from
// there, each at its own origin.
std::pair<Token, Token> cut(const Token& token, size_t index) {
  size_t at = token.stops[index].first;
  float x = token.stops[index].second;
  Token before = token, after = token;
  for (Token* part : {&before, &after}) {
    part->glyphs.clear();
    part->clusters.clear();
    part->stops.clear();
  }
  for (size_t g = 0; g < token.glyphs.size(); ++g) {
    Token& part = token.clusters[g] < at ? before : after;
    Glyph glyph = token.glyphs[g];
    if (&part == &after) glyph.x -= x;
    part.glyphs.push_back(glyph);
    part.clusters.push_back(token.clusters[g]);
  }
  for (size_t k = 0; k < token.stops.size(); ++k) {
    if (k < index) {
      before.stops.push_back(token.stops[k]);
    } else {
      after.stops.emplace_back(token.stops[k].first, token.stops[k].second - x);
    }
  }
  before.text = token.text.substr(0, at - token.start);
  after.text = token.text.substr(at - token.start);
  after.start = at;
  after.breakBefore = true;
  before.width = x;
  before.inkWidth = std::min(token.inkWidth, x);
  after.width = token.width - x;
  after.inkWidth = std::max(0.0f, token.inkWidth - x);
  return {std::move(before), std::move(after)};
}

}  // namespace

ParagraphLayout TextEngine::layout(
    const AttributedString& text,
    const ParagraphAttributes& paragraph,
    float maxWidth) {
  std::lock_guard lock(impl_->mutex);
  // Measured by Yoga and drawn in every frame its paragraph is damaged in: the same paragraph at
  // the same width is laid out once.
  Impl::LayoutKey key{text, paragraph, maxWidth};
  if (auto cached = impl_->layouts.find(key); cached != impl_->layouts.end()) {
    impl_->recent.splice(impl_->recent.begin(), impl_->recent, cached->second);
    return cached->second->second;
  }
  ParagraphLayout result = layoutUncached(text, paragraph, maxWidth);
  impl_->recent.emplace_front(std::move(key), result);
  impl_->layouts.emplace(impl_->recent.front().first, impl_->recent.begin());
  if (impl_->recent.size() > Impl::kCachedLayouts) {
    impl_->layouts.erase(impl_->recent.back().first);
    impl_->recent.pop_back();
  }
  return result;
}

ParagraphLayout TextEngine::layoutUncached(
    const AttributedString& text,
    const ParagraphAttributes& paragraph,
    float maxWidth) {
  std::vector<Token> tokens;
  std::optional<TextAlignment> alignment;

  // Each fragment's string as it is shown (textTransform applied), and where a line may break, over
  // the whole paragraph (a break depends on the text on both sides, whatever fragments it is in):
  // ICU's line break rules (UAX #14), as Android's.
  std::vector<std::string> shown;
  std::string whole;
  for (const auto& fragment : text.getFragments()) {
    shown.push_back(fragment.isAttachment() ? fragment.string : transformed(fragment.string, fragment.textAttributes));
    whole += shown.back();
  }
  std::vector<bool> breakable(whole.size() + 1, false);
  {
    UErrorCode status = U_ZERO_ERROR;
    UText* utext = utext_openUTF8(nullptr, whole.data(), static_cast<int64_t>(whole.size()), &status);
    if (!impl_->lineBreaks && U_SUCCESS(status)) impl_->lineBreaks = ubrk_open(UBRK_LINE, "", nullptr, 0, &status);
    if (U_SUCCESS(status) && impl_->lineBreaks) {
      ubrk_setUText(impl_->lineBreaks, utext, &status);
      for (int32_t at = ubrk_first(impl_->lineBreaks); U_SUCCESS(status) && at != UBRK_DONE;
           at = ubrk_next(impl_->lineBreaks)) {
        breakable[static_cast<size_t>(at)] = true;
      }
      // The iterator keeps no reference to this paragraph's text.
      static const UChar kNothing[] = {0};
      ubrk_setText(impl_->lineBreaks, kNothing, 0, &status);
    } else {
      // Without ICU: after spaces, as before it.
      for (size_t i = 1; i <= whole.size(); ++i) breakable[i] = whole[i - 1] == ' ';
    }
    utext_close(utext);
  }

  // Each fragment's OpenType features, alive as long as its tokens (ellipses are shaped with them).
  std::deque<std::vector<hb_feature_t>> fragmentFeatures;
  size_t base = 0;  // where the fragment starts in the paragraph's string
  size_t index = 0;
  for (const auto& fragment : text.getFragments()) {
    const std::string& s = shown[index++];
    size_t fragmentBase = base;
    base += s.size();
    if (fragment.isAttachment()) continue;
    const auto& attributes = fragment.textAttributes;
    if (!alignment) alignment = attributes.alignment;
    float size = std::isnan(attributes.fontSize) ? 14.0f : attributes.fontSize;
    if (!std::isnan(attributes.fontSizeMultiplier)) size *= attributes.fontSizeMultiplier;
    int weight = attributes.fontWeight ? static_cast<int>(*attributes.fontWeight) : 400;
    bool italic = attributes.fontStyle && *attributes.fontStyle != FontStyle::Normal;
    Face* primary = impl_->face(impl_->pathFor(attributes.fontFamily), weight, size, italic);
    if (!primary) continue;
    uint32_t color = argb(attributes.foregroundColor);
    // Android's letter spacing (Paint.setLetterSpacing, em = letterSpacing / fontSize, as React
    // Native's CustomLetterSpacingSpan sets it), which Minikin adds to each cluster, half before it
    // and half after.
    float spacing = std::isnan(attributes.letterSpacing) ? 0 : attributes.letterSpacing;
    fragmentFeatures.push_back(features(attributes));
    const std::vector<hb_feature_t>* featureList = &fragmentFeatures.back();
    auto decoration = attributes.textDecorationLineType.value_or(TextDecorationLineType::None);
    bool underline = decoration == TextDecorationLineType::Underline ||
                     decoration == TextDecorationLineType::UnderlineStrikethrough;
    bool strikethrough = decoration == TextDecorationLineType::Strikethrough ||
                         decoration == TextDecorationLineType::UnderlineStrikethrough;
    // React Native's Android text shadow: drawn when it is offset or blurred, in its colour, or
    // ReactTextAttributeProps' default (0x55000000) when none is given.
    float shadowDx = attributes.textShadowOffset ? attributes.textShadowOffset->width : 0;
    float shadowDy = attributes.textShadowOffset ? attributes.textShadowOffset->height : 0;
    float shadowRadius = std::isnan(attributes.textShadowRadius) ? 0 : attributes.textShadowRadius;
    uint32_t shadowColor = 0;
    if (shadowDx != 0 || shadowDy != 0 || shadowRadius != 0) {
      shadowColor = attributes.textShadowColor ? argb(attributes.textShadowColor) : 0x55000000u;
    }

    size_t i = 0;
    while (i < s.size()) {
      Token token;
      token.color = color;
      token.lineHeight = attributes.lineHeight;
      token.ascent = primary->ascent;
      token.descent = primary->descent;
      token.capHeight = primary->capHeight;
      token.xHeight = primary->xHeight;
      token.face = primary;
      token.spacing = spacing;
      token.size = size;
      token.underline = underline;
      token.strikethrough = strikethrough;
      token.shadowColor = shadowColor;
      token.shadowDx = shadowDx;
      token.shadowDy = shadowDy;
      token.shadowRadius = shadowRadius;
      token.features = featureList;
      token.start = fragmentBase + i;
      token.breakBefore = breakable[fragmentBase + i];
      if (s[i] == '\n') {
        token.newline = true;
        tokens.push_back(token);
        ++i;
        continue;
      }
      // To the next break opportunity, the next hard break or the fragment's end.
      size_t start = i;
      do {
        next(s, i);
      } while (i < s.size() && !breakable[fragmentBase + i] && s[i] != '\n');
      size_t wordEnd = i;
      while (wordEnd > start && s[wordEnd - 1] == ' ') --wordEnd;
      // Segment by face coverage and shape each segment.
      size_t segment = start;
      float pen = 0;
      while (segment < i) {
        size_t probe = segment;
        uint32_t cp = next(s, probe);
        Face* face = primary;
        if (cp != ' ' && !covers(*primary, cp)) {
          for (const auto& path : impl_->fallbacks) {
            Face* candidate = impl_->face(path, weight, size, italic);
            if (candidate && covers(*candidate, cp)) {
              face = candidate;
              break;
            }
          }
        }
        size_t end = probe;
        while (end < i) {
          size_t look = end;
          uint32_t c = next(s, look);
          bool covered = c == ' ' || covers(*face, c);
          bool primaryCovers = c == ' ' || covers(*primary, c);
          if (!covered || (face != primary && primaryCovers)) break;
          end = look;
        }
        shape(*face, s, segment, end - segment, featureList,
              [&](uint32_t id, uint32_t at, float offset, float advance) {
                // HarfBuzz's clusters are byte offsets into the fragment's string.
                size_t cluster = fragmentBase + at;
                if (token.stops.empty() || token.stops.back().first != cluster) {
                  // A cluster's spacing is half before it and half after; where one cluster's
                  // ends and the next's begins is the stop between them.
                  if (!token.stops.empty()) pen += spacing - spacing / 2;
                  token.stops.emplace_back(cluster, pen);
                  pen += spacing / 2;
                }
                token.glyphs.push_back({id, pen + offset, face});
                token.clusters.push_back(cluster);
                pen += advance;
                if (at < wordEnd) token.inkWidth = pen + (spacing - spacing / 2);
              });
        token.ascent = std::max(token.ascent, face->ascent);
        token.descent = std::max(token.descent, face->descent);
        segment = end;
      }
      // The last cluster's spacing after it.
      if (!token.stops.empty()) pen += spacing - spacing / 2;
      token.width = pen;
      token.text = s.substr(start, i - start);
      tokens.push_back(std::move(token));
    }
  }

  // Greedy line breaking at the break opportunities; a word wider than a line is broken between
  // its clusters, as Android's StaticLayout breaks one (its "desperate" breaks).
  struct Line {
    std::vector<const Token*> tokens;
    size_t start = 0;
    float width = 0, ink = 0, ascent = 0, descent = 0, height = 0;
    bool afterHardBreak = false;  // the first line of a paragraph after the first
    // An ellipsized line's text: all it stands for, as Android's layout gives a line's text (its
    // start to its end, the part ellipsized included).
    std::optional<std::string> source;
  };
  // Tokens made by cutting a word, alive as long as the lines that hold them.
  std::deque<Token> pieces;
  // An empty line (before a hard break, or after one at the end) takes the break's metrics; the
  // first word on a line replaces them with its own.
  auto emptyLineMetrics = [](Line& line, const Token& token) {
    line.ascent = token.ascent;
    line.descent = token.descent;
    line.height = std::isnan(token.lineHeight) ? token.ascent + token.descent : token.lineHeight;
  };
  auto place = [](Line& line, const Token& token) {
    if (line.tokens.empty()) {
      line.ascent = line.descent = line.height = 0;
      line.start = token.start;
    }
    line.ink = line.width + token.inkWidth;
    line.width += token.width;
    line.tokens.push_back(&token);
    float natural = token.ascent + token.descent;
    float height = std::isnan(token.lineHeight) ? natural : token.lineHeight;
    float lead = (height - natural) / 2;
    line.ascent = std::max(line.ascent, token.ascent + lead);
    line.descent = std::max(line.descent, token.descent + lead);
    line.height = std::max(line.height, height);
  };
  bool bounded = std::isfinite(maxWidth);
  auto fits = [&](float width) { return !bounded || width <= maxWidth + 0.01f; };
  // A word: the token at `t` and those after it a line cannot break before; its end.
  auto wordEnd = [&](size_t t) {
    size_t end = t + 1;
    while (end < tokens.size() && !tokens[end].newline && !tokens[end].breakBefore) ++end;
    return end;
  };
  std::vector<Line> lines(1);
  // Greedy breaking (Android's BREAK_STRATEGY_SIMPLE) of the tokens [from, to), a paragraph without
  // hard breaks, onto the lines from the last.
  auto greedy = [&](size_t from, size_t to) {
    for (size_t t = from; t < to;) {
      const Token& first = tokens[t];
      size_t end = wordEnd(t);
      float width = 0, ink = 0;
      for (size_t k = t; k < end; ++k) {
        ink = width + tokens[k].inkWidth;
        width += tokens[k].width;
      }
      if (!lines.back().tokens.empty() && !fits(lines.back().width + ink)) {
        lines.emplace_back();
        lines.back().start = first.start;
      }
      if (fits(ink)) {
        for (size_t k = t; k < end; ++k) place(lines.back(), tokens[k]);
      } else {
        // Wider than a line: as many of its clusters on each line as fit, at least one.
        for (size_t k = t; k < end; ++k) {
          const Token* rest = &tokens[k];
          while (!fits(lines.back().width + rest->inkWidth)) {
            Line& line = lines.back();
            size_t index = 0;
            while (index + 1 < rest->stops.size() && fits(line.width + stopX(*rest, index + 1))) ++index;
            if (index == 0) {
              if (!line.tokens.empty()) {
                lines.emplace_back();
                continue;
              }
              if (rest->stops.size() < 2) break;  // one cluster: it is the line
              index = 1;
            }
            auto [before, after] = cut(*rest, index);
            pieces.push_back(std::move(before));
            place(line, pieces.back());
            pieces.push_back(std::move(after));
            rest = &pieces.back();
            lines.emplace_back();
          }
          place(lines.back(), *rest);
        }
      }
      t = end;
    }
  };

  // Minikin's optimal line breaker (OptimalLineBreaker.cpp, which StaticLayout runs for
  // BREAK_STRATEGY_HIGH_QUALITY and BALANCED) on the tokens [from, to), onto the lines from the
  // last: of every way to break the paragraph at its break opportunities, the one with the least
  // score, a line's score its slack squared plus a penalty for each line, so its lines are even
  // rather than each as full as it can be. Its constants are Minikin's:
  //   SCORE_OVERFULL 1e12, a line wider than the box (taken only when nothing else fits);
  //   SCORE_DESPERATE 1e10, a break between a word's characters, offered only in a word wider than
  //     the box (its "desperate" breaks), so as few are taken as can be;
  //   the line penalty, LINE_PENALTY_MULTIPLIER (2) times the hyphen penalty, which is
  //     0.5 × font size × the line's width (× 4 at hyphenation frequency normal), the largest of
  //     the paragraph's runs (Android has no line penalty for justified text, which the host does
  //     not lay out justified).
  // The last line's slack is free with highQuality (a short last line costs nothing) and scored
  // with balanced, which evens every line, the last too. As Minikin, it only looks back to the
  // earliest break a line from which still fits (`active`), and stops looking back once a line's
  // score alone is worse than the best (`bestHope`).
  auto optimal = [&](size_t from, size_t to, bool balanced, HyphenationFrequency hyphenation) {
    // A place to break: before the cluster `stop` of the token `token` (0: before the token), with
    // the paragraph's width up to it if the line goes on past it (`preBreak`: a word's trailing
    // spaces counted) and if a line ends there (`postBreak`: they hang past the line's end).
    struct Candidate {
      size_t token, stop;
      double preBreak, postBreak;
      float penalty;
    };
    constexpr float kScoreInfinity = std::numeric_limits<float>::max();
    constexpr float kScoreOverfull = 1e12f;
    constexpr float kScoreDesperate = 1e10f;
    constexpr float kLinePenaltyMultiplier = 2.0f;
    float linePenalty = 0;
    for (size_t k = from; k < to; ++k) {
      float hyphenPenalty = 0.5f * tokens[k].size * maxWidth;
      if (hyphenation == HyphenationFrequency::Normal) hyphenPenalty *= 4.0f;
      linePenalty = std::max(linePenalty, hyphenPenalty * kLinePenaltyMultiplier);
    }
    std::vector<Candidate> candidates{{from, 0, 0, 0, 0}};
    double x = 0;  // the paragraph's width before the token
    for (size_t t = from; t < to;) {
      size_t end = wordEnd(t);
      double width = 0, ink = 0;
      for (size_t k = t; k < end; ++k) {
        ink = width + tokens[k].inkWidth;
        width += tokens[k].width;
      }
      if (!fits(static_cast<float>(ink))) {
        // Wider than a line: a desperate break at each of its clusters.
        double at = x;
        for (size_t k = t; k < end; ++k) {
          for (size_t stop = k == t ? 1 : 0; stop < tokens[k].stops.size(); ++stop) {
            double here = at + stopX(tokens[k], stop);
            candidates.push_back({k, stop, here, here, kScoreDesperate});
          }
          at += tokens[k].width;
        }
      }
      candidates.push_back({end, 0, x + width, x + ink, 0});
      x += width;
      t = end;
    }
    // Each candidate's best score and the candidate its line begins at (every line has the box's
    // width, so Minikin's line numbers are not needed).
    struct Best {
      float score;
      size_t previous;
    };
    std::vector<Best> best{{0, 0}};
    best.reserve(candidates.size());
    size_t active = 0;
    for (size_t i = 1; i < candidates.size(); ++i) {
      const bool atEnd = i == candidates.size() - 1;
      float bestScore = kScoreInfinity, bestHope = 0;
      size_t bestPrevious = 0;
      const double leftEdge = candidates[i].postBreak - maxWidth;
      for (size_t j = active; j < i; ++j) {
        const float jScore = best[j].score;
        if (jScore + bestHope >= bestScore) continue;
        // The slack of a line from j to i; within the host's rounding tolerance, none.
        float delta = static_cast<float>(candidates[j].preBreak - leftEdge);
        if (delta < 0 && delta >= -0.01f) delta = 0;
        float widthScore = 0, additionalPenalty = 0;
        if (delta < 0) {
          widthScore = kScoreOverfull;
        } else if (atEnd && !balanced) {
          // A hyphen ending the last line but one weighs four times as much (Minikin's
          // LAST_LINE_PENALTY_MULTIPLIER); with no hyphenation every penalty here is 0 or desperate.
          additionalPenalty = 4.0f * candidates[j].penalty;
        } else {
          widthScore = delta * delta;
        }
        if (delta < 0) {
          active = j + 1;
        } else {
          bestHope = widthScore;
        }
        const float score = jScore + widthScore + additionalPenalty;
        if (score <= bestScore) {
          bestScore = score;
          bestPrevious = j;
        }
      }
      best.push_back({bestScore + candidates[i].penalty + linePenalty, bestPrevious});
    }
    // The breaks taken, from the paragraph's end back to its start.
    std::vector<size_t> taken;
    for (size_t i = candidates.size() - 1; i > 0; i = best[i].previous) taken.push_back(i);
    std::reverse(taken.begin(), taken.end());
    size_t begin = 0;
    for (size_t n = 0; n < taken.size(); ++n) {
      const Candidate& a = candidates[begin];
      const Candidate& b = candidates[taken[n]];
      if (n > 0) {
        lines.emplace_back();
        lines.back().start = a.stop > 0 ? tokens[a.token].stops[a.stop].first : tokens[a.token].start;
      }
      Line& line = lines.back();
      for (size_t k = a.token; k <= b.token && k < to; ++k) {
        size_t first = k == a.token ? a.stop : 0;
        size_t last = k == b.token ? b.stop : tokens[k].stops.size();
        if (k == b.token && b.stop == 0) break;
        if (first == 0 && last == tokens[k].stops.size()) {
          place(line, tokens[k]);
          continue;
        }
        Token part = tokens[k];
        if (last < tokens[k].stops.size()) part = cut(part, last).first;
        if (first > 0) part = cut(part, first).second;
        pieces.push_back(std::move(part));
        place(line, pieces.back());
      }
      begin = taken[n];
    }
  };

  // Android's break strategy: highQuality unless the paragraph asks for another (React Native's
  // default for Text and TextInput, TextAttributeProps' DEFAULT_BREAK_STRATEGY). Hyphenation, off
  // in React Native's Android unless android_hyphenationFrequency asks for it, is not provided: a
  // paragraph asking for it is not hyphenated, said once.
  if (paragraph.android_hyphenationFrequency != HyphenationFrequency::None) {
    static bool reported = false;
    if (!reported) std::cerr << "[host] unsupported on OxyOS: android_hyphenationFrequency (no word is hyphenated)\n";
    reported = true;
  }
  for (size_t t = 0; t < tokens.size();) {
    const Token& first = tokens[t];
    if (first.newline) {
      Line* line = &lines.back();
      if (line->tokens.empty()) emptyLineMetrics(*line, first);
      lines.emplace_back();
      lines.back().start = first.start + 1;
      lines.back().afterHardBreak = true;
      emptyLineMetrics(lines.back(), first);
      ++t;
      continue;
    }
    // A paragraph: to the next hard break, broken on its own, as StaticLayout breaks each.
    size_t end = t;
    while (end < tokens.size() && !tokens[end].newline) ++end;
    // One that fits on a line is that line whatever the strategy, as Android lays it out as a
    // BoringLayout without breaking it: most labels, never weighed.
    float width = 0, ink = 0;
    for (size_t k = t; k < end; ++k) {
      ink = width + tokens[k].inkWidth;
      width += tokens[k].width;
    }
    if (!bounded || fits(ink) || paragraph.textBreakStrategy == TextBreakStrategy::Simple) {
      greedy(t, end);
    } else {
      optimal(t, end, paragraph.textBreakStrategy == TextBreakStrategy::Balanced,
              paragraph.android_hyphenationFrequency);
    }
    t = end;
  }

  int maxLines = paragraph.maximumNumberOfLines;
  if (maxLines > 0 && static_cast<int>(lines.size()) > maxLines) {
    // Android's StaticLayout: the last line shown holds the rest of its paragraph, ellipsized at
    // its end to fit; at its start or middle only when one line is shown. Otherwise the lines
    // after the last are clipped.
    auto mode = paragraph.ellipsizeMode;
    bool ellipsized = mode == EllipsizeMode::Tail || (maxLines == 1 && mode != EllipsizeMode::Clip);
    size_t last = static_cast<size_t>(maxLines) - 1;
    if (ellipsized) {
      std::vector<const Token*> rest;
      size_t next = last;
      for (; next < lines.size() && (next == last || !lines[next].afterHardBreak); ++next) {
        rest.insert(rest.end(), lines[next].tokens.begin(), lines[next].tokens.end());
      }
      bool more = next < lines.size();  // paragraphs after this one are not shown
      // Each cluster's advance, in order, with its token and index.
      struct Cluster {
        const Token* token;
        size_t index;
        float advance;
      };
      std::vector<Cluster> clusters;
      float total = 0;
      for (const Token* token : rest) {
        for (size_t k = 0; k < token->stops.size(); ++k) {
          float advance = stopX(*token, k + 1) - stopX(*token, k);
          clusters.push_back({token, k, advance});
          total += advance;
        }
      }
      if (!clusters.empty() && (!fits(total) || more)) {
        // The ellipsis, in the attributes of the text it replaces.
        auto ellipsisFor = [&](const Token& like) {
          Token ellipsis = like;
          ellipsis.glyphs.clear();
          ellipsis.clusters.clear();
          ellipsis.stops.clear();
          ellipsis.newline = false;
          ellipsis.breakBefore = false;
          ellipsis.width = ellipsis.inkWidth = 0;
          static const std::string kEllipsis = "…";
          if (const Face* face = like.face) {
            float pen = like.spacing / 2;
            shape(*face, kEllipsis, 0, kEllipsis.size(), like.features,
                  [&](uint32_t id, uint32_t, float offset, float advance) {
                    ellipsis.glyphs.push_back({id, pen + offset, face});
                    ellipsis.clusters.push_back(like.start);
                    pen += advance;
                  });
            ellipsis.width = ellipsis.inkWidth = pen + (like.spacing - like.spacing / 2);
          }
          ellipsis.text = kEllipsis;
          return ellipsis;
        };
        std::map<std::pair<const Face*, float>, float> ellipsisWidths;
        auto ellipsisWidth = [&](const Token& like) {
          auto key = std::make_pair(like.face, like.spacing);
          auto it = ellipsisWidths.find(key);
          if (it == ellipsisWidths.end()) it = ellipsisWidths.emplace(key, ellipsisFor(like).width).first;
          return it->second;
        };
        // The clusters kept from the front and from the back (calculateEllipsis' arithmetic).
        float available = bounded ? maxWidth : std::numeric_limits<float>::infinity();
        size_t front = 0, back = clusters.size();
        if (mode == EllipsizeMode::Head && maxLines == 1) {
          float width = ellipsisWidth(*clusters.back().token), sum = 0;
          for (; back > 0 && sum + clusters[back - 1].advance + width <= available; --back) sum += clusters[back - 1].advance;
          // With nothing to cut and more text after it, the first character gives way to it.
          if (back == 0 && more) back = 1;
          front = 0;
        } else if (mode == EllipsizeMode::Middle && maxLines == 1) {
          float width = ellipsisWidth(*clusters.front().token), right = 0, left = 0;
          float rightAvailable = (available - width) / 2;
          for (; back > 0 && right + clusters[back - 1].advance <= rightAvailable; --back) right += clusters[back - 1].advance;
          float leftAvailable = available - width - right;
          for (; front < back && left + clusters[front].advance <= leftAvailable; ++front) left += clusters[front].advance;
        } else {
          float sum = 0;
          for (; front < clusters.size(); ++front) {
            float width = ellipsisWidth(*clusters[front].token);
            if (sum + clusters[front].advance + width > available) break;
            sum += clusters[front].advance;
          }
          if (front == clusters.size() && more) front = clusters.size() - 1;
          back = clusters.size();
        }
        // The kept clusters of each token, as tokens of their own, with the ellipsis between.
        auto keep = [&](size_t from, size_t to, std::vector<const Token*>& out) {
          for (size_t c = from; c < to;) {
            const Token* token = clusters[c].token;
            size_t first = clusters[c].index, count = 0;
            while (c < to && clusters[c].token == token) ++c, ++count;
            if (first == 0 && count == token->stops.size()) {
              out.push_back(token);
              continue;
            }
            Token part = *token;
            if (first + count < token->stops.size()) part = cut(part, first + count).first;
            if (first > 0) part = cut(part, first).second;
            pieces.push_back(std::move(part));
            out.push_back(&pieces.back());
          }
        };
        std::vector<const Token*> shown;
        keep(0, front, shown);
        const Token& like = *clusters[std::min(front, clusters.size() - 1)].token;
        pieces.push_back(ellipsisFor(like));
        shown.push_back(&pieces.back());
        keep(back, clusters.size(), shown);
        Line line;
        line.afterHardBreak = lines[last].afterHardBreak;
        for (const Token* token : shown) place(line, *token);
        line.start = lines[last].start;
        line.source.emplace();
        for (const Token* token : rest) *line.source += token->text;
        lines[last] = std::move(line);
      } else {
        Line line;
        line.afterHardBreak = lines[last].afterHardBreak;
        for (const Token* token : rest) place(line, *token);
        line.start = lines[last].start;
        lines[last] = std::move(line);
      }
    }
    lines.resize(static_cast<size_t>(maxLines));
  }

  ParagraphLayout result;
  for (const auto& line : lines) result.width = std::max(result.width, line.ink);
  // Lines align within the available width; without one, within the widest line.
  float boxWidth = bounded ? maxWidth : result.width;
  float top = 0;
  for (const auto& line : lines) {
    float offset = 0;
    if (alignment == TextAlignment::Center) offset = (boxWidth - line.ink) / 2;
    if (alignment == TextAlignment::Right) offset = boxWidth - line.ink;
    float x = offset;
    float baseline = top + line.ascent;
    LineLayout metrics{.start = line.start, .x = offset, .top = top, .width = line.ink, .advance = line.width, .height = line.height,
                       .ascent = line.ascent, .descent = line.descent};
    for (const Token* token : line.tokens) {
      if (!line.source) metrics.text += token->text;
      for (const auto& [at, stop] : token->stops) metrics.stops.emplace_back(at, x + stop);
      metrics.capHeight = std::max(metrics.capHeight, token->capHeight);
      metrics.xHeight = std::max(metrics.xHeight, token->xHeight);
      GlyphRun run;
      run.color = token->color;
      run.baseline = baseline;
      for (const auto& glyph : token->glyphs) run.glyphs.push_back({glyph.id, x + glyph.x, glyph.face});
      // Decorated over its advance, the line's trailing spaces left out.
      run.left = x;
      run.right = x + (token == line.tokens.back() ? token->inkWidth : token->width);
      run.underline = token->underline;
      run.strikethrough = token->strikethrough;
      run.shadowColor = token->shadowColor;
      run.shadowDx = token->shadowDx;
      run.shadowDy = token->shadowDy;
      run.shadowRadius = token->shadowRadius;
      x += token->width;
      result.runs.push_back(std::move(run));
    }
    if (line.source) {
      metrics.text = *line.source;
      metrics.stops.emplace_back(line.start + line.source->size(), x);
    } else if (!line.tokens.empty()) {
      const Token& last = *line.tokens.back();
      metrics.stops.emplace_back(last.start + last.text.size(), x);
    }
    result.lines.push_back(std::move(metrics));
    top += line.height;
  }
  result.height = top;
  return result;
}

}  // namespace oxyos