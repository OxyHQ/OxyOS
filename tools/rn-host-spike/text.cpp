#include "text.h"

#include <react/renderer/graphics/Color.h>

#include <ft2build.h>
#include FT_FREETYPE_H
#include FT_MULTIPLE_MASTERS_H
#include <hb-ft.h>
#include <hb.h>

#include <cmath>
#include <iostream>
#include <map>
#include <mutex>
#include <tuple>

using namespace facebook::react;

namespace oxy {

namespace {

struct Face {
  FT_Face ft = nullptr;
  hb_font_t* hb = nullptr;
  float ascent = 0, descent = 0;
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
  std::map<std::pair<void*, uint32_t>, Bitmap> bitmaps;
  std::recursive_mutex mutex;

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
    entry.hb = hb_ft_font_create_referenced(entry.ft);
    hb_ft_font_changed(entry.hb);
    entry.ascent = entry.ft->size->metrics.ascender / 64.0f;
    entry.descent = -entry.ft->size->metrics.descender / 64.0f;
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

void TextEngine::registerFamily(const std::string& family, const std::string& path) {
  std::lock_guard lock(impl_->mutex);
  impl_->families[family] = path;
}

void TextEngine::addFallback(const std::string& path) {
  std::lock_guard lock(impl_->mutex);
  impl_->fallbacks.push_back(path);
}

namespace {

// A word with its trailing spaces, or a hard line break, shaped with one face per segment.
struct Token {
  std::vector<Glyph> glyphs;
  float width = 0;          // including trailing spaces
  float inkWidth = 0;       // excluding trailing spaces
  float ascent = 0, descent = 0, lineHeight = NAN;
  uint32_t color = 0xff000000;
  bool newline = false;
};

}  // namespace

ParagraphLayout TextEngine::layout(
    const AttributedString& text,
    const ParagraphAttributes& paragraph,
    float maxWidth) {
  std::lock_guard lock(impl_->mutex);
  std::vector<Token> tokens;
  std::optional<TextAlignment> alignment;

  for (const auto& fragment : text.getFragments()) {
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
      if (s[i] == '\n') {
        token.newline = true;
        tokens.push_back(token);
        ++i;
        continue;
      }
      // Word, then its trailing spaces.
      size_t start = i;
      while (i < s.size() && s[i] != ' ' && s[i] != '\n') next(s, i);
      size_t wordEnd = i;
      while (i < s.size() && s[i] == ' ') ++i;
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
          token.glyphs.push_back({infos[g].codepoint, pen + positions[g].x_offset / 64.0f, face->ft});
          pen += positions[g].x_advance / 64.0f + spacing;
          if (infos[g].cluster < wordEnd) token.inkWidth = pen;
        }
        hb_buffer_destroy(buffer);
        token.ascent = std::max(token.ascent, face->ascent);
        token.descent = std::max(token.descent, face->descent);
        segment = end;
      }
      token.width = pen;
      tokens.push_back(std::move(token));
    }
  }

  // Greedy line breaking.
  struct Line {
    std::vector<const Token*> tokens;
    float width = 0, ink = 0, ascent = 0, descent = 0, height = 0;
  };
  std::vector<Line> lines(1);
  for (const auto& token : tokens) {
    Line* line = &lines.back();
    if (token.newline) {
      if (line->tokens.empty()) {
        line->ascent = token.ascent;
        line->descent = token.descent;
        line->height = std::isnan(token.lineHeight) ? token.ascent + token.descent : token.lineHeight;
      }
      lines.emplace_back();
      continue;
    }
    if (!line->tokens.empty() && std::isfinite(maxWidth) && line->width + token.inkWidth > maxWidth + 0.01f) {
      lines.emplace_back();
      line = &lines.back();
    }
    line->ink = line->width + token.inkWidth;
    line->width += token.width;
    line->tokens.push_back(&token);
    float natural = token.ascent + token.descent;
    float height = std::isnan(token.lineHeight) ? natural : token.lineHeight;
    float lead = (height - natural) / 2;
    line->ascent = std::max(line->ascent, token.ascent + lead);
    line->descent = std::max(line->descent, token.descent + lead);
    line->height = std::max(line->height, height);
  }
  if (paragraph.maximumNumberOfLines > 0 && static_cast<int>(lines.size()) > paragraph.maximumNumberOfLines) {
    lines.resize(paragraph.maximumNumberOfLines);
  }

  ParagraphLayout result;
  for (const auto& line : lines) result.width = std::max(result.width, line.ink);
  float boxWidth = std::isfinite(maxWidth) ? std::min(maxWidth, result.width) : result.width;
  float top = 0;
  for (const auto& line : lines) {
    float offset = 0;
    if (alignment == TextAlignment::Center) offset = (boxWidth - line.ink) / 2;
    if (alignment == TextAlignment::Right) offset = boxWidth - line.ink;
    float x = offset;
    float baseline = top + line.ascent;
    for (const Token* token : line.tokens) {
      GlyphRun run;
      run.color = token->color;
      run.baseline = baseline;
      for (const auto& glyph : token->glyphs) run.glyphs.push_back({glyph.id, x + glyph.x, glyph.face});
      x += token->width;
      result.runs.push_back(std::move(run));
    }
    top += line.height;
  }
  result.height = top;
  result.lines = static_cast<int>(lines.size());
  return result;
}

const TextEngine::Bitmap& TextEngine::rasterize(void* face, uint32_t glyph) {
  std::lock_guard lock(impl_->mutex);
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

}  // namespace oxy
