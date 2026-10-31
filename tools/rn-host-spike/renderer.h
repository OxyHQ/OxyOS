// Retained view tree built from Fabric's mounting transactions, rasterized into a 32-bit buffer.
#pragma once

#include <react/renderer/mounting/ShadowViewMutation.h>

#include <oxyos_client.h>

#include <cmath>
#include <cstdint>
#include <map>
#include <memory>
#include <optional>
#include <utility>
#include <string>
#include <unordered_map>
#include <vector>

namespace facebook::react {
class AndroidTextInputProps;
}

namespace oxyos {

struct Display {
  float width;
  float height;
  float scale;
  // What a window shows where its views draw nothing, premultiplied ARGB: the system theme's,
  // as Android's DayNight windowBackground (docs/adr/0019-settings.mdx's appearance).
  uint32_t background = kLightWindowBackground;
  static constexpr uint32_t kLightWindowBackground = 0xffffffffu;
  static constexpr uint32_t kDarkWindowBackground = 0xff121212u;

  // The buffer's size in pixels, as the protocol defines it (docs/adr/0001): ceil(logical ×
  // scale_120 / 120) in whole numbers, so it is exactly the size the compositor configured; a
  // float's rounding is a pixel short at some sizes (961 × 1.25), which the compositor refuses.
  int pixelWidth() const { return pixels(width); }
  int pixelHeight() const { return pixels(height); }

 private:
  int pixels(float logical) const {
    const long scale120 = std::lround(scale * 120);
    return static_cast<int>((std::lround(logical) * scale120 + 119) / 120);
  }
};

// A decoded image an Image view shows (images.h): `width` × `height` premultiplied RGBA pixels,
// sRGB, and the size it is drawn at by `center` and `repeat`, in logical pixels.
struct Picture {
  int width = 0, height = 0;
  std::vector<uint8_t> pixels;
  float naturalWidth = 0, naturalHeight = 0;
  // Which picture this is, for the GPU painter's upload of it (docs/adr/0031-gpu-buffers.mdx): a
  // picture is never changed once made, and each one made has its own.
  uint64_t id = next();
  static uint64_t next();
};

struct Canvas;

// How a modal is presented while its window animates in or out (modal.h), as Android's window
// animations move and fade a dialog's window: moved down by `offset` logical pixels, drawn at
// `alpha`.
struct ModalPresentation {
  float offset = 0, alpha = 1;
  bool operator==(const ModalPresentation&) const = default;
};

struct View {
  facebook::react::ShadowView view;
  std::vector<facebook::react::Tag> children;
  // The view it is a child of, as the last Insert placed it; -1 for the root and for a view
  // removed and not inserted again. What finds a view's place (windowFrame, ancestry) walks up
  // these rather than searching the tree from its root.
  facebook::react::Tag parent = -1;
  // What an Image view shows, as Android's ReactImageView holds its bitmap; none until loaded.
  std::shared_ptr<const Picture> picture;
};

class ViewTree {
 public:
  void apply(const facebook::react::ShadowViewMutationList& mutations);
  const std::unordered_map<facebook::react::Tag, View>& views() const { return views_; }
  // The views from the root to `tag`, inclusive, by their parents; empty if `tag` is not in the
  // root's tree.
  std::vector<facebook::react::Tag> path(facebook::react::Tag tag) const;
  // Gives an existing view the picture it shows (none for nullptr).
  void setPicture(facebook::react::Tag tag, std::shared_ptr<const Picture> picture) {
    if (auto it = views_.find(tag); it != views_.end()) it->second.picture = std::move(picture);
  }
  // Gives an existing view new props between commits (native-animated.h's updateViewProps).
  void setProps(facebook::react::Tag tag, facebook::react::Props::Shared props) {
    if (auto it = views_.find(tag); it != views_.end()) it->second.view.props = std::move(props);
  }
  facebook::react::Tag root() const { return root_; }

  // A modal animating in (modal.h): how it is presented; none when it is not animating.
  void present(facebook::react::Tag modal, std::optional<ModalPresentation> presentation) {
    if (presentation) {
      presenting_[modal] = *presentation;
    } else {
      presenting_.erase(modal);
    }
  }
  ModalPresentation presentation(facebook::react::Tag modal) const {
    auto it = presenting_.find(modal);
    return it == presenting_.end() ? ModalPresentation{} : it->second;
  }
  // Modals gone from the tree whose windows are still animating out, as Android animates a
  // dismissed dialog's window: each one's last frame, at the window's size, and how it is
  // presented, drawn over the window in this order.
  struct Leaving {
    std::shared_ptr<const Canvas> frame;
    ModalPresentation presentation;
    bool operator==(const Leaving&) const = default;
  };
  std::vector<Leaving>& leaving() { return leaving_; }
  const std::vector<Leaving>& leaving() const { return leaving_; }

 private:
  std::unordered_map<facebook::react::Tag, View> views_;
  facebook::react::Tag root_ = -1;
  std::map<facebook::react::Tag, ModalPresentation> presenting_;
  std::vector<Leaving> leaving_;
};

struct RenderStats {
  int views = 0;
  int texts = 0;
  int unimplemented = 0;
  std::vector<std::string> unimplementedNames;
  // The text drawn, in drawing order: each paragraph's string and each text input's value (its
  // placeholder when empty), unless clipped out entirely.
  std::vector<std::string> textContent;
  bool written = false;  // whether render() saved the PNG
  // The windows shown live in what was drawn (docs/adr/0022-window-views.mdx), in drawing order:
  // each OxyOSWindowView's window number, its frame and the part of it its ancestors leave
  // visible (in the canvas's logical pixels), its corners' radius and its opacity. Views under a
  // transform, a mask or a blur are not among them.
  struct WindowView {
    uint32_t window = 0;
    facebook::react::Rect rect, clip;
    float radius = 0, opacity = 1;
    bool operator==(const WindowView&) const = default;
  };
  std::vector<WindowView> windowViews;
};

// A canvas drawn on the GPU (docs/adr/0031-gpu-buffers.mdx, decision 12): the painter's, freed
// with the last canvas holding it.
struct GpuCanvas;

// Pixels are 0xAARRGGBB, premultiplied, row-major, `width * scale` by `height * scale`. A window's
// canvas starts opaque white; a popup's starts transparent. Drawn on the GPU, its pixels are the
// GPU's (`gpu`) and `pixels` is empty: canvasPixels reads them.
struct Canvas {
  int width = 0, height = 0;
  std::vector<uint32_t> pixels;
  std::shared_ptr<GpuCanvas> gpu;
};

// The painter canvases are drawn with (docs/adr/0031-gpu-buffers.mdx, decision 12): the GPU's,
// once set (a painter on the device the compositor shares buffers with), else the CPU's. A canvas
// drawn on one and then on the other is drawn whole again, or read back. Unset, for good, when the
// GPU path fails.
void setGpuPaint(OxyOSPaint* paint);
OxyOSPaint* gpuPaint();
// While it lives, canvases are drawn on the CPU on this thread whatever is set (the render check).
struct CpuPaintScope {
  CpuPaintScope();
  ~CpuPaintScope();
};
// The painter's canvas a canvas drawn on the GPU is (0 for one drawn on the CPU).
uint32_t gpuCanvasId(const Canvas& canvas);
// A canvas's pixels, read back from the GPU when it is drawn there.
std::vector<uint32_t> canvasPixels(const Canvas& canvas);
// The largest difference of any channel between two frames of one size, and how many pixels
// differ by more than 3 levels in a channel (decision 13's parity check).
struct FrameDifference {
  int largest = 0;
  size_t beyondThree = 0;
  size_t pixels = 0;
};
FrameDifference compareFrames(const std::vector<uint32_t>& a, const std::vector<uint32_t>& b);

// The focused text input's editing state, drawn over its text (textinput.h owns it). Offsets
// are UTF-8 byte offsets into the input's text.
struct TextInputEdit {
  facebook::react::Tag tag = -1;
  size_t selectionStart = 0, selectionEnd = 0;  // the caret when equal
  size_t composeStart = 0, composeEnd = 0;      // the composition, underlined; empty if equal
  // Segments of the composition an input method styled: the one being converted is underlined
  // more heavily on a background, as Android draws its selected composing span; an error in red.
  struct Segment {
    size_t start = 0, end = 0;
    uint8_t style = OXYOS_PREEDIT_COMPOSING;
    bool operator==(const Segment&) const = default;
  };
  std::vector<Segment> segments;
  // How far each input's text is scrolled within its content box: across for a single-line input,
  // down for a multiline one. An input keeps it when it loses focus, as an EditText does.
  std::map<facebook::react::Tag, facebook::react::Point> scrolls;
  bool operator==(const TextInputEdit&) const = default;
};

// A text input's caret geometry, from its value laid out as it is drawn (wrapped when multiline,
// centred vertically when single-line). Offsets are UTF-8 byte offsets into the value; positions
// are relative to the input's content box. A secure field's geometry is its bullets'.
class TextInputGeometry {
 public:
  TextInputGeometry(const facebook::react::ShadowView& view, const std::string& value);

  struct Caret {
    size_t line = 0;
    float x = 0, top = 0, height = 0;
  };
  // Where the caret at `offset` is drawn. An offset where a wrapped line ends is drawn at the start
  // of the next line.
  Caret caret(size_t offset) const;
  // The offset nearest a point: its line by y, then the character boundary nearest x.
  size_t offsetAt(float x, float y) const;
  // The offset on `line` nearest x.
  size_t offsetOnLine(size_t line, float x) const;
  // The first and last offsets on `line` (Home and End). A wrapped line's last offset is before
  // the character at which it wrapped, since the offset after it is drawn on the next line.
  std::pair<size_t, size_t> lineEdges(size_t line) const;
  size_t lineCount() const { return lines_.size(); }
  // The text's extent: its widest line's right edge, and its last line's bottom.
  float width() const;
  float height() const;
  // `scroll` kept within what the text's extent allows in a content box of `size`, with room
  // for the caret at the end of a line.
  facebook::react::Point clamp(facebook::react::Point scroll, facebook::react::Size size) const;
  // Rectangles covering [from, to): one per line it touches; a hard break inside it shows as a
  // little width past its line's end.
  std::vector<facebook::react::Rect> span(size_t from, size_t to) const;

 private:
  struct Line {
    size_t start = 0, end = 0;  // offsets into what is shown; `end` excludes a hard break
    bool broken = false;        // ended by a hard break (not wrapped, not the last)
    float x = 0, top = 0, height = 0;
    std::vector<std::pair<size_t, float>> stops;  // the layout's caret stops (LineLayout::stops)
  };
  float xOn(const Line& line, size_t shownOffset) const;
  // lineEdges, in offsets into what is shown.
  std::pair<size_t, size_t> shownEdges(size_t line) const;
  size_t toShown(size_t offset) const;
  size_t toValue(size_t shownOffset) const;

  std::shared_ptr<const facebook::react::AndroidTextInputProps> props_;
  std::string value_, shown_;
  std::vector<Line> lines_;
};

// Component whose subtree is shown in a popup of its own (popup.h), never in the window.
inline constexpr const char* kPopupComponent = "OxyOSPopupView";
// Component whose subtree is a shell region in a surface of its own (shell-surface.h).
inline constexpr const char* kShellSurfaceComponent = "OxyOSShellSurface";
// Component where the compositor draws a window live, over the shell surface it is in
// (shell-surface.h, docs/adr/0022-window-views.mdx).
inline constexpr const char* kWindowViewComponent = "OxyOSWindowView";
// The window an OxyOSWindowView shows: its number, if it names one.
std::optional<uint32_t> windowViewNumber(const facebook::react::ShadowView& view);
// Whether a view's subtree is drawn in a surface of its own, never in its root's.
inline bool ownSurface(const std::string& name) { return name == kPopupComponent || name == kShellSurfaceComponent; }
// React Native's Modal: on Android a window of its own over the activity's. Here its subtree is
// drawn over the window's content, at the window's origin and size (modal.h lays it out), and
// takes the window's input while it is shown.
inline constexpr const char* kModalComponent = "ModalHostView";
inline bool isModal(const std::string& name) { return name == kModalComponent; }
// A 2D affine map: x' = a x + c y + tx, y' = b x + d y + ty.
struct Affine {
  float a = 1, b = 0, c = 0, d = 1, tx = 0, ty = 0;
  facebook::react::Point apply(facebook::react::Point p) const { return {a * p.x + c * p.y + tx, b * p.x + d * p.y + ty}; }
  // This map after `inner`: first `inner`, then this one.
  Affine after(const Affine& inner) const {
    return {a * inner.a + c * inner.b, b * inner.a + d * inner.b, a * inner.c + c * inner.d,
            b * inner.c + d * inner.d, a * inner.tx + c * inner.ty + tx, b * inner.tx + d * inner.ty + ty};
  }
  static Affine translation(float x, float y) { return {1, 0, 0, 1, x, y}; }
  bool translates() const { return a == 1 && b == 0 && c == 0 && d == 1; }
  float determinant() const { return a * d - b * c; }
  // The inverse map; only for a map whose determinant is not zero.
  Affine inverse() const {
    float k = 1 / determinant();
    return {d * k, -b * k, -c * k, a * k, (c * ty - d * tx) * k, (b * tx - a * ty) * k};
  }
  // The smallest rectangle holding the map of `rect`.
  facebook::react::Rect bounds(const facebook::react::Rect& rect) const;
};
// A view's `transform`, as Android applies a view's matrix: from the view's own coordinates (its
// frame's origin at 0, 0) to its parent's, its frame's origin included, around its transform origin.
// Its subtree is drawn, hit and damaged through it. A 3D transform is drawn as its projection onto
// the screen's plane, without perspective, and the host reports it once.
Affine viewTransform(const facebook::react::ShadowView& view);
// Where a view's `transform` takes its centre, less the centre: how far a view is measured to have
// moved (windowFrame), its size kept.
facebook::react::Point transformOffset(const facebook::react::ShadowView& view);
// The modals shown in the window, in the order they are drawn: the last is on top.
std::vector<facebook::react::Tag> modals(const ViewTree& tree);
// Draws one modal alone, as its window shows it, onto a transparent canvas at the window's size:
// on white unless it is `transparent`.
RenderStats drawModal(const ViewTree& tree, facebook::react::Tag modal, const Display& display, Canvas& canvas,
                      const TextInputEdit& edit = {});

// Draws the window: the tree without popup subtrees. `edit` is the focused text input's caret,
// selection and composition, drawn with its text.
RenderStats draw(const ViewTree& tree, const Display& display, Canvas& canvas, const TextInputEdit& edit = {});
// Whether two versions of a view are drawn the same (damage.h): the props `drawView` reads are
// equal (background, opacity, borders, clipping), whatever new props object a re-render made.
// A text input, whose drawing reads many of its props, is the same only with the same props.
bool drawsTheSame(const facebook::react::ShadowView& before, const facebook::react::ShadowView& after);
// Whether a view's own drawing is a plain fill or nothing, whatever its size: a View with no
// corners, borders or transform. Such a view resized in place changes only where it gained or
// lost area (damage.h); what it draws under its children and clips them to is the same elsewhere.
bool drawsFlat(const facebook::react::ShadowView& view);
// Draws only the window's `damage` (window-local logical rectangles) onto `canvas`, which holds
// the previous frame: each rectangle is drawn again as a whole frame draws it. The stats are the
// whole window's. A canvas of another size (the window resized at the same scale: WindowDamage
// gives no damage at a new scale) keeps its pixels where both sizes have them, anchored at the top
// left, and the damage holds the rest.
RenderStats drawDamaged(const ViewTree& tree, const Display& display, Canvas& canvas, const TextInputEdit& edit,
                        const std::vector<facebook::react::Rect>& damage);
// Draws a popup view's children, relative to the popup, onto a transparent canvas of `display`'s
// size (the popup's configured size).
RenderStats drawPopup(const ViewTree& tree, facebook::react::Tag popup, const Display& display, Canvas& canvas,
                      const TextInputEdit& edit = {});
// Draws a view's children with the view at `at` onto a transparent canvas of `display`'s size: a
// shell region in its surface (shell-surface.h), wherever the region is in that surface.
RenderStats drawRegion(const ViewTree& tree, facebook::react::Tag region, facebook::react::Point at,
                       const Display& display, Canvas& canvas, const TextInputEdit& edit = {});
// Draws only `damage` (surface-local logical rectangles) of a region onto `canvas`, which holds its
// previous frame at this size, each rectangle as drawRegion draws it. A canvas of another size is
// drawn whole.
RenderStats drawRegionDamaged(const ViewTree& tree, facebook::react::Tag region, facebook::react::Point at,
                              const Display& display, Canvas& canvas, const TextInputEdit& edit,
                              const std::vector<facebook::react::Rect>& damage);
// A view's frame in its root's coordinates (window-local logical pixels), as drawn: scroll
// offsets applied. Empty if the view is not in the tree.
std::optional<facebook::react::Rect> windowFrame(const ViewTree& tree, facebook::react::Tag tag);
// The views from the root to `tag`, inclusive; empty if `tag` is not in the tree.
std::vector<facebook::react::Tag> ancestry(const ViewTree& tree, facebook::react::Tag tag);
// Draws, writes a PNG and returns the stats; `written` reports whether the PNG was saved.
RenderStats render(const ViewTree& tree, const Display& display, const std::string& pngPath);
bool writePng(const Canvas& canvas, const std::string& path);

}  // namespace oxyos
