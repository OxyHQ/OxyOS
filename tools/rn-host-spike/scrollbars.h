// A scroll view's overlay scrollbars, as macOS draws its overlay scrollers (NSScroller's
// overlay style): nothing while the view rests; a thin rounded thumb along the far edge of each
// axis its content scrolls along while it scrolls, fading out once it stops; thicker, over a
// translucent track, while the pointer is over it, where it is dragged and a press in the track
// pages. React Native's `showsVerticalScrollIndicator` and `showsHorizontalScrollIndicator` hide
// one; Android's `persistentScrollbar` (View#setScrollbarFadingEnabled(false)) keeps it shown.
// What a scroll view shows (`ScrollbarShown`, renderer.h) is the pointer's to say (input.h), as a
// scroll moves it; where its thumb and track are is worked out here, from the view as it is drawn,
// for the renderer and for the pointer alike, on the display's pixels.
#pragma once

#include "renderer.h"

#include <include/core/SkCanvas.h>

#include <optional>

namespace oxyos {

// macOS's overlay scroller, in logical pixels: the track as wide as a legacy scroller (15), the
// thumb 6 wide at rest and 9 with the pointer over it, 3 from the view's edge and from the track's
// ends, never shorter than 20.
constexpr float kScrollbarTrack = 15, kScrollbarThin = 6, kScrollbarWide = 9, kScrollbarInset = 3,
                kScrollbarMinThumb = 20;
// How long a scrollbar stays once its view stops scrolling, and how long it takes to fade out
// (milliseconds); how long it takes to grow when the pointer comes over it.
constexpr double kScrollbarHoldMs = 500, kScrollbarFadeMs = 250, kScrollbarExpandMs = 120;

// Where a scroll view's scrollbar along `axis` (0 across, the bar along the bottom; 1 down, the bar
// along the right) is: its track and thumb in the view's own coordinates (its frame's origin at
// 0, 0, unscrolled), each edge on a whole pixel of a display of `scale`.
struct ScrollbarGeometry {
  facebook::react::Rect track, thumb;
};

// Whether `view` (a ScrollView) has a scrollbar along `axis`: its content scrolls that way and its
// props show it (`showsVerticalScrollIndicator`, `showsHorizontalScrollIndicator`).
bool hasScrollbar(const facebook::react::ShadowView& view, int axis);
// Whether its scrollbars never fade (Android's `persistentScrollbar`).
bool persistentScrollbars(const facebook::react::ShadowView& view);

// The scrollbar of `view` along `axis` with its content at `offset`, `expansion` of the way from
// thin (0) to wide (1); none if `hasScrollbar` says it has none.
std::optional<ScrollbarGeometry> scrollbarGeometry(const facebook::react::ShadowView& view,
                                                   facebook::react::Point offset, int axis, float expansion,
                                                   float scale);

// Draws `view`'s scrollbars as `shown` says, the view's frame at (x, y) of the canvas: a dark thumb
// in the light appearance and a light one in the dark (`dark`), as NSScroller's default knob style,
// unless `indicatorStyle` asks for black or white.
void drawScrollbars(SkCanvas& canvas, const facebook::react::ShadowView& view, float x, float y,
                    const ScrollbarShown& shown, bool dark, float scale);

}  // namespace oxyos
