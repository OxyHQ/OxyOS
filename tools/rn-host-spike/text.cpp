#include "text.h"

#include <react/renderer/graphics/Color.h>
#include <react/utils/hash_combine.h>

#include <ft2build.h>
#include FT_FREETYPE_H
#include FT_MULTIPLE_MASTERS_H
#include FT_TRUETYPE_TABLES_H
#include <hb-ft.h>
#include <hb.h>
#include <unicode/ubrk.h>
#include <unicode/utext.h>

#include <cmath>
#include <deque>
#include <limits>
#include <list>
#include <unordered_map>
#include <iostream>
#include <map>
#include <mutex>
#include <tuple>

using namespace facebook::react;

namespace oxyos {

namespace {

struct Face {
  FT_Face ft = nullptr;
  hb_font_t* hb = nullptr;
  float ascent = 0, descent = 0, capHeight = 0, xHeight = 0;
};

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

}  // namespace

struct TextEngine::Impl {
  FT_Library library = nullptr;
  std::map<std::string, std::string> families;
  std::vector<std::string> fallbacks;
  std::map<std::tuple<std::string, int, int>, Face> faces;
  // Each face's file, weight and size, so a glyph laid out in it is drawn at another size.
  std::map<void*, std::tuple<std::string, int, float>> origins;
  std::map<std::pair<void*, uint32_t>, Bitmap> bitmaps;
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

  Face& face(const std::string& path, int weight, float size) {
    auto key = std::make_tuple(path, weight, static_cast<int>(std::lround(size * 64)));
    auto it = faces.find(key);
    if (it != faces.end()) return it->second;
    Face entry;
    if (FT_New_Face(library, path.c_str(), 0, &entry.ft) != 0) {
      std::cerr << "[text] cannot open font " << path << "\n";
      return faces[key] = entry;
    }
    FT_Set_Char_Size(entry.ft, 0, static_cast<FT_F26Dot6>(std::lround(size * 64)), 72, 72);
    FT_MM_Var* axes = nullptr;
    if (FT_HAS_MULTIPLE_MASTERS(entry.ft) && FT_Get_MM_Var(entry.ft, &axes) == 0) {
      std::vector<FT_Fixed> coords(axes->num_axis);
      for (FT_UInt a = 0; a < axes->num_axis; ++a) {
        coords[a] = axes->axis[a].def;
        if (axes->axis[a].tag == FT_MAKE_TAG('w', 'g', 'h', 't')) {
          auto clamped = std::clamp<FT_Fixed>(weight << 16, axes->axis[a].minimum, axes->axis[a].maximum);
          coords[a] = clamped;
        }
      }
      FT_Set_Var_Design_Coordinates(entry.ft, axes->num_axis, coords.data());
      FT_Done_MM_Var(library, axes);
    }
    origins[entry.ft] = {path, weight, size};
    entry.hb = hb_ft_font_create_referenced(entry.ft);
    hb_ft_font_changed(entry.hb);
    entry.ascent = entry.ft->size->metrics.ascender / 64.0f;
    entry.descent = -entry.ft->size->metrics.descender / 64.0f;
    // Cap and x heights come from the OS/2 table when the font has one (version 2 or later).
    if (auto* os2 = static_cast<TT_OS2*>(FT_Get_Sfnt_Table(entry.ft, FT_SFNT_OS2)); os2 && os2->version >= 2) {
      float scale = size / entry.ft->units_per_EM;
      entry.capHeight = os2->sCapHeight * scale;
      entry.xHeight = os2->sxHeight * scale;
    }
    return faces[key] = entry;
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

TextEngine::TextEngine() : impl_(std::make_unique<Impl>()) {
  FT_Init_FreeType(&impl_->library);
}

bool TextEngine::registerFamily(const std::string& family, const std::string& path) {
  std::lock_guard lock(impl_->mutex);
  FT_Face face = nullptr;
  if (FT_New_Face(impl_->library, path.c_str(), 0, &face) != 0) {
    std::cerr << "[text] cannot open font " << path << " for " << family << "\n";
    return false;
  }
  FT_Done_Face(face);
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
};

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

  // Where a line may break, over the whole paragraph (a break depends on the text on both sides,
  // whatever fragments it is in): ICU's line break rules (UAX #14), as Android's.
  std::string whole;
  for (const auto& fragment : text.getFragments()) whole += fragment.string;
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

  size_t base = 0;  // where the fragment starts in the paragraph's string
  for (const auto& fragment : text.getFragments()) {
    size_t fragmentBase = base;
    base += fragment.string.size();
    if (fragment.isAttachment()) continue;
    const auto& attributes = fragment.textAttributes;
    if (!alignment) alignment = attributes.alignment;
    float size = std::isnan(attributes.fontSize) ? 14.0f : attributes.fontSize;
    if (!std::isnan(attributes.fontSizeMultiplier)) size *= attributes.fontSizeMultiplier;
    int weight = attributes.fontWeight ? static_cast<int>(*attributes.fontWeight) : 400;
    Face& primary = impl_->face(impl_->pathFor(attributes.fontFamily), weight, size);
    if (!primary.ft) continue;
    uint32_t color = argb(attributes.foregroundColor);
    float spacing = std::isnan(attributes.letterSpacing) ? 0 : attributes.letterSpacing;

    const std::string& s = fragment.string;
    size_t i = 0;
    while (i < s.size()) {
      Token token;
      token.color = color;
      token.lineHeight = attributes.lineHeight;
      token.ascent = primary.ascent;
      token.descent = primary.descent;
      token.capHeight = primary.capHeight;
      token.xHeight = primary.xHeight;
      token.face = &primary;
      token.spacing = spacing;
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
        Face* face = &primary;
        if (cp != ' ' && FT_Get_Char_Index(primary.ft, cp) == 0) {
          for (const auto& path : impl_->fallbacks) {
            Face& candidate = impl_->face(path, weight, size);
            if (candidate.ft && FT_Get_Char_Index(candidate.ft, cp) != 0) {
              face = &candidate;
              break;
            }
          }
        }
        size_t end = probe;
        while (end < i) {
          size_t look = end;
          uint32_t c = next(s, look);
          bool covered = c == ' ' || FT_Get_Char_Index(face->ft, c) != 0;
          bool primaryCovers = c == ' ' || FT_Get_Char_Index(primary.ft, c) != 0;
          if (!covered || (face != &primary && primaryCovers)) break;
          end = look;
        }
        hb_buffer_t* buffer = hb_buffer_create();
        hb_buffer_add_utf8(buffer, s.data(), static_cast<int>(s.size()), static_cast<unsigned>(segment),
                           static_cast<int>(end - segment));
        hb_buffer_guess_segment_properties(buffer);
        hb_shape(face->hb, buffer, nullptr, 0);
        unsigned count = 0;
        auto* infos = hb_buffer_get_glyph_infos(buffer, &count);
        auto* positions = hb_buffer_get_glyph_positions(buffer, &count);
        for (unsigned g = 0; g < count; ++g) {
          // HarfBuzz's clusters are byte offsets into the fragment's string.
          size_t cluster = fragmentBase + infos[g].cluster;
          if (token.stops.empty() || token.stops.back().first != cluster) token.stops.emplace_back(cluster, pen);
          token.glyphs.push_back({infos[g].codepoint, pen + positions[g].x_offset / 64.0f, face->ft});
          token.clusters.push_back(cluster);
          pen += positions[g].x_advance / 64.0f + spacing;
          if (infos[g].cluster < wordEnd) token.inkWidth = pen;
        }
        hb_buffer_destroy(buffer);
        token.ascent = std::max(token.ascent, face->ascent);
        token.descent = std::max(token.descent, face->descent);
        segment = end;
      }
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
  std::vector<Line> lines(1);
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
    // A word: this token and those after it a line cannot break before.
    size_t end = t + 1;
    while (end < tokens.size() && !tokens[end].newline && !tokens[end].breakBefore) ++end;
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
          Token ellipsis;
          ellipsis.color = like.color;
          ellipsis.lineHeight = like.lineHeight;
          ellipsis.ascent = like.ascent;
          ellipsis.descent = like.descent;
          ellipsis.capHeight = like.capHeight;
          ellipsis.xHeight = like.xHeight;
          ellipsis.face = like.face;
          ellipsis.spacing = like.spacing;
          ellipsis.start = like.start;
          ellipsis.breakBefore = false;
          const Face* face = like.face;
          static const std::string kEllipsis = "…";
          if (face && face->ft) {
            hb_buffer_t* buffer = hb_buffer_create();
            hb_buffer_add_utf8(buffer, kEllipsis.data(), static_cast<int>(kEllipsis.size()), 0,
                               static_cast<int>(kEllipsis.size()));
            hb_buffer_guess_segment_properties(buffer);
            hb_shape(face->hb, buffer, nullptr, 0);
            unsigned count = 0;
            auto* infos = hb_buffer_get_glyph_infos(buffer, &count);
            auto* positions = hb_buffer_get_glyph_positions(buffer, &count);
            float pen = 0;
            for (unsigned g = 0; g < count; ++g) {
              ellipsis.glyphs.push_back({infos[g].codepoint, pen + positions[g].x_offset / 64.0f, face->ft});
              ellipsis.clusters.push_back(like.start);
              pen += positions[g].x_advance / 64.0f + like.spacing;
            }
            hb_buffer_destroy(buffer);
            ellipsis.width = ellipsis.inkWidth = pen;
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

const TextEngine::Bitmap& TextEngine::rasterize(void* face, uint32_t glyph, float scale) {
  std::lock_guard lock(impl_->mutex);
  // Drawn at the surface's scale: the same face at the size in device pixels, not the logical
  // glyph moved to scaled positions.
  if (scale != 1.0f) {
    auto origin = impl_->origins.find(face);
    if (origin != impl_->origins.end()) {
      const auto& [path, weight, size] = origin->second;
      Face& scaled = impl_->face(path, weight, size * scale);
      if (scaled.ft) face = scaled.ft;
    }
  }
  auto key = std::make_pair(face, glyph);
  auto it = impl_->bitmaps.find(key);
  if (it != impl_->bitmaps.end()) return it->second;
  Bitmap bitmap;
  auto ft = static_cast<FT_Face>(face);
  if (FT_Load_Glyph(ft, glyph, FT_LOAD_DEFAULT | FT_LOAD_TARGET_LIGHT) == 0 &&
      FT_Render_Glyph(ft->glyph, FT_RENDER_MODE_NORMAL) == 0) {
    const auto& source = ft->glyph->bitmap;
    bitmap.width = static_cast<int>(source.width);
    bitmap.height = static_cast<int>(source.rows);
    bitmap.left = ft->glyph->bitmap_left;
    bitmap.top = ft->glyph->bitmap_top;
    bitmap.pixels.resize(static_cast<size_t>(bitmap.width) * bitmap.height);
    for (int y = 0; y < bitmap.height; ++y) {
      for (int x = 0; x < bitmap.width; ++x) {
        bitmap.pixels[static_cast<size_t>(y) * bitmap.width + x] = source.buffer[y * source.pitch + x];
      }
    }
  }
  return impl_->bitmaps[key] = std::move(bitmap);
}

}  // namespace oxyos
