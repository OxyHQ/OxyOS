#include "popup.h"

#include "accessibility.h"
#include "report.h"

#include <react/renderer/core/propsConversions.h>

#include <algorithm>
#include <cstring>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <iostream>
#include <set>

using namespace facebook::react;

namespace facebook::react {

const char OxyOSPopupViewComponentName[] = "OxyOSPopupView";

OxyOSPopupViewProps::OxyOSPopupViewProps(
    const PropsParserContext& context,
    const OxyOSPopupViewProps& sourceProps,
    const RawProps& rawProps)
    : ViewProps(context, sourceProps, rawProps),
      anchorTag(convertRawProp(context, rawProps, "anchorTag", sourceProps.anchorTag, -1)),
      side(convertRawProp(context, rawProps, "side", sourceProps.side, std::string("bottom"))),
      align(convertRawProp(context, rawProps, "align", sourceProps.align, std::string("start"))),
      sideOffset(convertRawProp(context, rawProps, "sideOffset", sourceProps.sideOffset, 0.0f)),
      alignOffset(convertRawProp(context, rawProps, "alignOffset", sourceProps.alignOffset, 0.0f)),
      hasPoint(convertRawProp(context, rawProps, "hasPoint", sourceProps.hasPoint, false)),
      pointX(convertRawProp(context, rawProps, "pointX", sourceProps.pointX, 0.0f)),
      pointY(convertRawProp(context, rawProps, "pointY", sourceProps.pointY, 0.0f)) {}

}  // namespace facebook::react

namespace oxyos {

namespace {

// The protocol's anchor and gravity values.
enum Edge : uint8_t { Center = 0, Top = 1, Bottom = 2, Left = 3, Right = 4, TopLeft = 5, BottomLeft = 6,
                      TopRight = 7, BottomRight = 8 };
// Its adjustment flags.
constexpr uint8_t kSlideX = 1, kSlideY = 2, kFlipX = 4, kFlipY = 8;

bool tracing() {
  static const bool trace = std::getenv("OXYOS_HOST_TRACE") != nullptr;
  return trace;
}

}  // namespace

OxyOSPositioner positioner(const OxyOSPopupViewProps& props, Rect anchor, Size size) {
  // Zero-filled (padding included) so positioners compare bytewise.
  OxyOSPositioner p;
  std::memset(&p, 0, sizeof p);
  p.anchor_x = static_cast<int32_t>(std::floor(anchor.origin.x));
  p.anchor_y = static_cast<int32_t>(std::floor(anchor.origin.y));
  p.anchor_width = static_cast<uint32_t>(std::max(1.0f, std::ceil(anchor.size.width)));
  p.anchor_height = static_cast<uint32_t>(std::max(1.0f, std::ceil(anchor.size.height)));
  p.width = static_cast<uint32_t>(std::max(1.0f, std::ceil(size.width)));
  p.height = static_cast<uint32_t>(std::max(1.0f, std::ceil(size.height)));
  auto offset = [](Float value) { return static_cast<int32_t>(std::lround(value)); };
  const std::string& side = props.side;
  const std::string& align = props.align;
  // Anchor point and growth direction for each side and alignment (ADR 0002's table).
  auto pick = [&](Edge start, Edge centre, Edge end) { return align == "end" ? end : align == "center" ? centre : start; };
  if (side == "top" || side == "bottom") {
    bool bottom = side == "bottom";
    p.anchor = bottom ? pick(BottomLeft, Bottom, BottomRight) : pick(TopLeft, Top, TopRight);
    p.gravity = bottom ? pick(BottomRight, Bottom, BottomLeft) : pick(TopRight, Top, TopLeft);
    p.offset_x = offset(props.alignOffset);
    p.offset_y = offset(bottom ? props.sideOffset : -props.sideOffset);
    p.adjust = kFlipY | kSlideX | kSlideY;
  } else {
    bool right = side == "right";
    p.anchor = right ? pick(TopRight, Right, BottomRight) : pick(TopLeft, Left, BottomLeft);
    p.gravity = right ? pick(BottomRight, Right, TopRight) : pick(BottomLeft, Left, TopLeft);
    p.offset_x = offset(right ? props.sideOffset : -props.sideOffset);
    p.offset_y = offset(props.alignOffset);
    p.adjust = kFlipX | kSlideX | kSlideY;
  }
  return p;
}

std::optional<std::pair<uint32_t, Rect>> PopupHost::anchored(const OxyOSPopupViewProps& props) const {
  Tag anchor = props.anchorTag;
  auto frame = windowFrame(tree_, anchor);
  if (!frame) return std::nullopt;
  auto [surface, origin] = parent_ ? parent_(anchor) : std::pair<uint32_t, Point>{Window::kToplevel, {}};
  frame->origin.x -= origin.x;
  frame->origin.y -= origin.y;
  // A point in the anchor is a frame of no size there (the positioner makes it one pixel).
  if (props.hasPoint) *frame = Rect{{frame->origin.x + props.pointX, frame->origin.y + props.pointY}, {0, 0}};
  // The anchor is the part of the view the parent surface shows: the compositor refuses an anchor
  // reaching beyond the parent (InvalidPositioner, as xdg-shell's), and a view may: a trigger
  // scrolled partly away, or at a pane's edge as the window resizes. The parent's size is the one
  // the compositor has (Window::extent: its shown frame's, not the latest configure's), so a
  // window growing with a menu open keeps the anchor within the frame shown until the larger one
  // is, and the popup is placed again then. One shown nowhere is a pixel at the nearest edge.
  if (auto extent = window_.extent(surface)) {
    auto clip = [](Float start, Float length, Float limit) {
      Float from = std::clamp(start, Float{0}, limit - 1), to = std::min(start + length, limit);
      return std::pair{from, std::max(to - from, Float{1})};
    };
    auto [x, width] = clip(frame->origin.x, frame->size.width, extent->first);
    auto [y, height] = clip(frame->origin.y, frame->size.height, extent->second);
    *frame = Rect{{x, y}, {width, height}};
  }
  return std::pair{surface, *frame};
}

void PopupHost::sync(uint64_t serial) {
  std::set<Tag> mounted;
  for (const auto& [tag, entry] : tree_.views()) {
    const auto& view = entry.view;
    if (!view.componentName || std::strcmp(view.componentName, kPopupComponent) != 0) continue;
    mounted.insert(tag);
    if (auto open = popups_.find(tag); open != popups_.end()) {
      // An open popup follows its anchor and its content's size: place it again when either
      // changed (an inline submenu expanding, the window laid out anew).
      auto& popup = open->second;
      auto props = std::static_pointer_cast<const OxyOSPopupViewProps>(view.props);
      auto anchor = anchored(*props);
      if (popup.dismissed || !anchor || view.layoutMetrics.frame.size.width <= 0) continue;
      auto placed = positioner(*props, anchor->second, view.layoutMetrics.frame.size);
      if (std::memcmp(&placed, &popup.placed, sizeof placed) != 0) {
        window_.reposition(popup.surface, placed);
        popup.placed = placed;
        popup.dirty = true;
        Report::get().popup("reposition", popup.surface, &view,
                            folly::dynamic::object("width", placed.width)("height", placed.height)(
                                "anchor_x", placed.anchor_x)("anchor_y", placed.anchor_y));
      }
      continue;
    }
    auto props = std::static_pointer_cast<const OxyOSPopupViewProps>(view.props);
    auto anchor = anchored(*props);
    Size size = view.layoutMetrics.frame.size;
    // Opened once its anchor is mounted and its content laid out.
    if (!anchor || size.width <= 0 || size.height <= 0) continue;
    uint32_t surface = window_.allocate();
    auto placed = positioner(*props, anchor->second, size);
    window_.openPopup(surface, anchor->first, placed, serial);
    popups_[tag] = Popup{.surface = surface, .placed = placed};
    Report::get().popup("open", surface, &view,
                        folly::dynamic::object("anchor", Report::name(tree_.views().at(props->anchorTag).view))(
                            "width", size.width)("height", size.height));
    if (tracing()) {
      std::cerr << "[popup] open " << surface << " on " << anchor->first << " for view " << tag << " anchored to "
                << props->anchorTag << " at " << anchor->second.origin.x << "," << anchor->second.origin.y << " "
                << size.width << "x" << size.height << "\n";
    }
  }
  for (auto it = popups_.begin(); it != popups_.end();) {
    if (mounted.count(it->first)) {
      ++it;
      continue;
    }
    if (!it->second.dismissed) window_.destroy(it->second.surface);
    Report::get().popup("close", it->second.surface, nullptr);
    accessibility_.forget(it->second.surface);
    if (tracing()) std::cerr << "[popup] close " << it->second.surface << "\n";
    it = popups_.erase(it);
  }
}

void PopupHost::changed() {
  for (auto& [tag, popup] : popups_) popup.dirty = true;
}

void PopupHost::present(const TextInputEdit& edit, const std::string& focus) {
  for (auto& [tag, popup] : popups_) {
    if (popup.dismissed || !window_.ready(popup.surface)) continue;
    const auto* surface = window_.surface(popup.surface);
    if (!popup.dirty && surface->configure == popup.drawnFor) continue;
    Canvas canvas;
    auto started = std::chrono::steady_clock::now();
    auto stats = drawPopup(tree_, tag, Display{surface->width, surface->height, surface->scale}, canvas, edit);
    double renderMs =
        std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - started).count();
    window_.present(popup.surface, canvas);
    Report::get().frame(popup.surface, canvas.width, canvas.height, renderMs, stats.textContent, focus);
    accessibility_.publish(popup.surface, tag);
    popup.dirty = false;
    popup.drawnFor = surface->configure;
  }
}

void PopupHost::dismissed(const OxyOSEvent& event) {
  for (auto& [tag, popup] : popups_) {
    if (popup.surface != event.surface || popup.dismissed) continue;
    popup.dismissed = true;
    window_.destroy(popup.surface);
    auto it = tree_.views().find(tag);
    if (it == tree_.views().end()) return;
    // The protocol's reasons, in the adapter's vocabulary.
    const char* reason = event.detail == 1 ? "outside-press" : event.detail == 4 ? "denied" : "closed";
    it->second.view.eventEmitter->dispatchEvent("dismiss", folly::dynamic::object("reason", reason));
    Report::get().popup("dismissed", popup.surface, &it->second.view, folly::dynamic::object("reason", reason));
    if (tracing()) std::cerr << "[popup] dismissed " << popup.surface << " (" << reason << ")\n";
    return;
  }
}

uint32_t PopupHost::surfaceOf(Tag view) const {
  for (Tag at : ancestry(tree_, view)) {
    auto it = popups_.find(at);
    if (it != popups_.end()) return it->second.surface;
  }
  return Window::kToplevel;
}

namespace {

// W3C key names for the Linux input codes a popup's content can act on.
const char* keyName(uint32_t code) {
  switch (code) {
    case 1: return "Escape";
    case 15: return "Tab";
    case 28: case 96: return "Enter";
    case 57: return " ";
    case 102: return "Home";
    case 103: return "ArrowUp";
    case 104: return "PageUp";
    case 105: return "ArrowLeft";
    case 106: return "ArrowRight";
    case 107: return "End";
    case 108: return "ArrowDown";
    case 109: return "PageDown";
    default: return nullptr;
  }
}

}  // namespace

void PopupHost::key(const OxyOSEvent& event) {
  for (auto& [tag, popup] : popups_) {
    if (popup.surface != event.surface || popup.dismissed) continue;
    auto it = tree_.views().find(tag);
    if (it == tree_.views().end()) return;
    auto send = [&](const std::string& key) {
      it->second.view.eventEmitter->dispatchEvent("key", folly::dynamic::object("key", key));
      Report::get().deliver("key", popup.surface, it->second.view, folly::dynamic::object("key", key));
    };
    if (event.kind == OXYOS_EVENT_KEY && event.flag) {
      if (const char* name = keyName(event.code)) send(name);
    } else if (event.kind == OXYOS_EVENT_TEXT) {
      // One key per character of the committed text (UTF-8 sequences kept whole).
      std::string text(event.text);
      for (size_t i = 0; i < text.size();) {
        size_t length = 1;
        auto byte = static_cast<unsigned char>(text[i]);
        if (byte >= 0xf0) length = 4; else if (byte >= 0xe0) length = 3; else if (byte >= 0xc0) length = 2;
        send(text.substr(i, length));
        i += length;
      }
    }
    return;
  }
}

std::optional<SurfaceContent> PopupHost::content(uint32_t surface) const {
  for (const auto& [tag, popup] : popups_) {
    if (popup.surface != surface || popup.dismissed) continue;
    auto frame = windowFrame(tree_, tag);
    if (!frame) return std::nullopt;
    return SurfaceContent{.surface = surface, .view = tag, .popup = true, .origin = frame->origin};
  }
  return std::nullopt;
}

}  // namespace oxyos
