// A view's background, border, shadows and outline drawn on Skia as React Native's Android
// implementation draws them: BackgroundStyleApplicator's CompositeBackgroundDrawable and its layers
// (OutsetBoxShadowDrawable, BackgroundDrawable, BackgroundImageDrawable, BorderDrawable,
// InsetBoxShadowDrawable, OutlineDrawable), each with the canvas calls Android makes, which are
// Skia's (android.graphics.Canvas is a thin layer over SkCanvas). Android works in device pixels;
// the canvas here is in logical pixels, scaled by the output's, so Android's lengths in pixels
// (its 0.8 px gaps, its whole-pixel rectangular borders) are divided by the scale.
#pragma once

#include <react/renderer/components/view/ViewProps.h>

#include <include/core/SkCanvas.h>
#include <include/core/SkImageFilter.h>
#include <include/core/SkRRect.h>

namespace oxyos {

// The view's background layers, in CompositeBackgroundDrawable's order: outset shadows, the
// background colour, the background image, the border, inset shadows, the outline. (x, y, w, h)
// is the view's frame on the canvas; `opacity` multiplies every layer's alpha, as a view's alpha
// does when it has no overlapping rendering (Drawable.setAlpha); `scale` is device pixels per
// logical pixel.
void drawBackground(SkCanvas& canvas, const facebook::react::ViewProps& props,
                    const facebook::react::LayoutMetrics& layout, float x, float y, float w, float h, float opacity,
                    float scale);

// Whether the view draws any background layer at all.
bool drawsBackground(const facebook::react::ViewProps& props);
// Whether Android would give the view a CompositeBackgroundDrawable (BackgroundStyleApplicator makes
// one for any background style: a colour, rounded corners, a border, a shadow, an outline, an
// image), whose outline its elevation's shadow is cast from.
bool hasBackground(const facebook::react::ViewProps& props, const facebook::react::LayoutMetrics& layout);

// How far outside its frame the view's background layers draw (outset shadows, the outline): the
// frame grown by these, left, top, right and bottom, in logical pixels.
struct Overflow {
  float left = 0, top = 0, right = 0, bottom = 0;
};
Overflow backgroundOverflow(const facebook::react::ViewProps& props);

// BackgroundStyleApplicator.clipToPaddingBox: the canvas clipped to the view's padding box, with
// its corners' inner radii (anti-aliased) when it is rounded, to its rectangle otherwise.
void clipToPaddingBox(SkCanvas& canvas, const facebook::react::ViewProps& props,
                      const facebook::react::LayoutMetrics& layout, float x, float y, float w, float h);

// The rounded rectangle of a frame with the view's corners (its border box).
SkRRect borderBox(const facebook::react::BorderMetrics& border, float x, float y, float w, float h);

// The view's `filter` as Android's FilterHelper chains it (RenderEffect), on Skia's image filters;
// null for none.
sk_sp<SkImageFilter> viewFilter(const facebook::react::ViewProps& props);
// How far the view's filter draws outside what it filters (blurs and drop shadows), in logical
// pixels on each side.
float filterOverflow(const facebook::react::ViewProps& props);

}  // namespace oxyos
