// Text for the OxyOS React Native host, as Android's Minikin lays it out: font files shaped by
// HarfBuzz with their unhinted, fractional advances and their own metrics (a variable font's weight
// on its `wght` axis), as Minikin shapes over MinikinFontSkia, and lines broken at ICU's line break
// opportunities by Android's break strategies (Minikin's optimal breaker for highQuality, React
// Native's default, and balanced; greedy for simple), ellipsized as Android's StaticLayout
// ellipsizes. Skia draws the glyphs from the same files (renderer.cpp).
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

typedef struct hb_font_t hb_font_t;

namespace oxyos {

// A font file at a weight and a size, owned by the engine for its lifetime: what a glyph is laid
// out in, and what the renderer draws it with.
struct Face {
  std::string path;
  int weight = 400;
  bool variable = false;  // whether the weight is on the file's `wght` axis
  float size = 0;
  // The optical size it is set at on the file's `opsz` axis (0: the file has none).
  float opticalSize = 0;
  // Android's fake italic (Minikin: a skew of -1/4): an italic asked of a font that has none; and
  // its fake bold (emboldened outlines): a bold asked of a font much lighter.
  bool fakeItalic = false, fakeBold = false;
  hb_font_t* hb = nullptr;
  // The font's metrics at its size (OS/2's typographic ones where it asks for them, hhea's
  // otherwise, as Skia's FreeType port reads them), the ascent and descent taken outward to whole
  // pixels as Android's Paint.getFontMetricsInt takes them.
  float ascent = 0, descent = 0, capHeight = 0, xHeight = 0;
  // Where its underline and strike-through are drawn, relative to the baseline (y down), and how
  // thick, as HWUI's drawTextDecorations draws them.
  float underlineTop = 0, underlineThickness = 0, strikeTop = 0, strikeThickness = 0;
};

struct Glyph {
  uint32_t id;
  float x;  // pen position relative to the paragraph origin
  const Face* face;
};

// A fragment's glyphs on one line, and what Android draws with them: its colour, its decorations
// (HWUI's drawTextDecorations, over the run's whole advance) and its shadow (React Native's
// ShadowStyleSpan: Paint.setShadowLayer).
struct GlyphRun {
  std::vector<Glyph> glyphs;
  float baseline;  // relative to the paragraph origin
  uint32_t color;  // 0xAARRGGBB
  float left = 0, right = 0;  // the run's advance, relative to the paragraph origin
  bool underline = false, strikethrough = false;
  // Its shadow's colour (0: none), offset and blur radius (setShadowLayer's radius).
  uint32_t shadowColor = 0;
  float shadowDx = 0, shadowDy = 0, shadowRadius = 0;
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
  // and registers nothing, when the file is not a font Skia can open.
  bool registerFamily(const std::string& family, const std::string& path);
  // Used for code points the requested family lacks.
  void addFallback(const std::string& path);

  ParagraphLayout layout(
      const facebook::react::AttributedString& text,
      const facebook::react::ParagraphAttributes& paragraph,
      float maxWidth);

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
