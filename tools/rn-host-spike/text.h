// Text for the OxyOS React Native host: FreeType faces, HarfBuzz shaping, and line breaking at
// ICU's line break opportunities by Android's break strategies (Minikin's optimal breaker for
// highQuality, React Native's default, and balanced; greedy for simple), ellipsized as Android's
// StaticLayout ellipsizes.
// The same layout serves Fabric's measurement (TextLayoutManager) and drawing, so what Yoga sizes
// is exactly what is painted.
#pragma once

#include <react/renderer/attributedstring/AttributedString.h>
#include <react/renderer/attributedstring/ParagraphAttributes.h>

#include <cstdint>
#include <memory>
#include <string>
#include <utility>
#include <vector>

namespace oxyos {

struct Glyph {
  uint32_t id;
  float x;  // pen position relative to the paragraph origin
  void* face;  // FT_Face, owned by the engine
};

struct GlyphRun {
  std::vector<Glyph> glyphs;
  float baseline;  // relative to the paragraph origin
  uint32_t color;  // 0xAARRGGBB
};

// One laid-out line, relative to the paragraph origin.
struct LineLayout {
  // Its text, and the byte offset where it starts in the paragraph's string (its fragments in
  // order). A line ends where the next starts, or before the hard break that ends it.
  std::string text;
  size_t start = 0;
  // Where a caret goes on the line: the byte offset where each glyph cluster starts, and its x,
  // then the line's end. Left to right, as the engine shapes.
  std::vector<std::pair<size_t, float>> stops;
  // `width` is the ink width; `advance` also counts trailing spaces (where a caret after them sits).
  float x = 0, top = 0, width = 0, advance = 0, height = 0;
  float ascent = 0, descent = 0, capHeight = 0, xHeight = 0;
};

struct ParagraphLayout {
  std::vector<GlyphRun> runs;
  std::vector<LineLayout> lines;
  float width = 0;
  float height = 0;
};

class TextEngine {
 public:
  static TextEngine& shared();

  // Registers a font file under a family name (variable fonts cover every weight). Returns false,
  // and registers nothing, when FreeType cannot open the file.
  bool registerFamily(const std::string& family, const std::string& path);
  // Used for code points the requested family lacks.
  void addFallback(const std::string& path);

  ParagraphLayout layout(
      const facebook::react::AttributedString& text,
      const facebook::react::ParagraphAttributes& paragraph,
      float maxWidth);

  // Gray coverage bitmap for a glyph laid out in `face`, rasterized at `scale` device pixels per
  // logical pixel: width, height, left, top and pixels (row-major).
  struct Bitmap {
    int width = 0, height = 0, left = 0, top = 0;
    std::vector<uint8_t> pixels;
  };
  const Bitmap& rasterize(void* face, uint32_t glyph, float scale);

 private:
  TextEngine();
  ParagraphLayout layoutUncached(
      const facebook::react::AttributedString& text,
      const facebook::react::ParagraphAttributes& paragraph,
      float maxWidth);
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace oxyos
