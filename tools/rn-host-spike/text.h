// Text for the OxyOS React Native host: FreeType faces, HarfBuzz shaping, greedy line breaking.
// The same layout serves Fabric's measurement (TextLayoutManager) and drawing, so what Yoga sizes
// is exactly what is painted.
#pragma once

#include <react/renderer/attributedstring/AttributedString.h>
#include <react/renderer/attributedstring/ParagraphAttributes.h>

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

namespace oxy {

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

struct ParagraphLayout {
  std::vector<GlyphRun> runs;
  float width = 0;
  float height = 0;
  int lines = 0;
};

class TextEngine {
 public:
  static TextEngine& shared();

  // Registers a font file under a family name (variable fonts cover every weight).
  void registerFamily(const std::string& family, const std::string& path);
  // Used for code points the requested family lacks.
  void addFallback(const std::string& path);

  ParagraphLayout layout(
      const facebook::react::AttributedString& text,
      const facebook::react::ParagraphAttributes& paragraph,
      float maxWidth);

  // Gray coverage bitmap for a glyph: width, height, left, top and pixels (row-major).
  struct Bitmap {
    int width = 0, height = 0, left = 0, top = 0;
    std::vector<uint8_t> pixels;
  };
  const Bitmap& rasterize(void* face, uint32_t glyph);

 private:
  TextEngine();
  struct Impl;
  std::unique_ptr<Impl> impl_;
};

}  // namespace oxy
