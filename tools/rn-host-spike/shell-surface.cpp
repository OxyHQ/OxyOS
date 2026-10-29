#include "shell-surface.h"

#include "report.h"

#include <react/renderer/core/propsConversions.h>

#include <chrono>
#include <cstdlib>
#include <iostream>
#include <cmath>
#include <set>

using namespace facebook::react;

namespace facebook::react {

const char OxyOSShellSurfaceComponentName[] = "OxyOSShellSurface";

OxyOSShellSurfaceProps::OxyOSShellSurfaceProps(const PropsParserContext& context,
                                               const OxyOSShellSurfaceProps& sourceProps, const RawProps& rawProps)
    : ViewProps(context, sourceProps, rawProps),
      kind(convertRawProp(context, rawProps, "kind", sourceProps.kind, std::string("panel"))),
      edge(convertRawProp(context, rawProps, "edge", sourceProps.edge, std::string("top"))),
      exclusive(convertRawProp(context, rawProps, "exclusive", sourceProps.exclusive, false)) {}

}  // namespace facebook::react

namespace oxyos {

bool ShellSurfaceHost::Placement::operator==(const Placement& other) const {
  return kind == other.kind && edge == other.edge && exclusive == other.exclusive && strip == other.strip &&
         input.x == other.input.x && input.y == other.input.y && input.width == other.input.width &&
         input.height == other.input.height;
}

std::optional<ShellSurfaceHost::Placement> ShellSurfaceHost::place(Tag tag) const {
  auto it = tree_.views().find(tag);
  auto region = windowFrame(tree_, tag);
  if (it == tree_.views().end() || !region || region->size.width <= 0 || region->size.height <= 0) {
    return std::nullopt;
  }
  auto props = std::static_pointer_cast<const OxyOSShellSurfaceProps>(it->second.view.props);
  // The root is the output.
  Float width = window_.width(), height = window_.height();
  Placement placed;
  if (props->kind == "overlay") {
    placed.kind = OXYOS_SHELL_OVERLAY;
    placed.edge = OXYOS_EDGE_NONE;
    placed.strip = Rect{{0, 0}, {width, height}};
  } else {
    placed.kind = OXYOS_SHELL_PANEL;
    const auto& edge = props->edge;
    // The strip from the edge to the far side of the region, in whole pixels.
    if (edge == "bottom") {
      Float thickness = std::ceil(height - region->origin.y);
      placed.edge = OXYOS_EDGE_BOTTOM;
      placed.strip = Rect{{0, height - thickness}, {width, thickness}};
    } else if (edge == "left") {
      placed.edge = OXYOS_EDGE_LEFT;
      placed.strip = Rect{{0, 0}, {std::ceil(region->origin.x + region->size.width), height}};
    } else if (edge == "right") {
      Float thickness = std::ceil(width - region->origin.x);
      placed.edge = OXYOS_EDGE_RIGHT;
      placed.strip = Rect{{width - thickness, 0}, {thickness, height}};
    } else {
      placed.edge = OXYOS_EDGE_TOP;
      placed.strip = Rect{{0, 0}, {width, std::ceil(region->origin.y + region->size.height)}};
    }
    if (props->exclusive) {
      bool vertical = placed.edge == OXYOS_EDGE_LEFT || placed.edge == OXYOS_EDGE_RIGHT;
      placed.exclusive = static_cast<uint32_t>(vertical ? placed.strip.size.width : placed.strip.size.height);
    }
  }
  // Input over the region alone, in the surface's coordinates, outward to whole pixels.
  Float left = std::floor(region->origin.x - placed.strip.origin.x);
  Float top = std::floor(region->origin.y - placed.strip.origin.y);
  Float right = std::ceil(region->origin.x + region->size.width - placed.strip.origin.x);
  Float bottom = std::ceil(region->origin.y + region->size.height - placed.strip.origin.y);
  placed.input = OxyOSRect{static_cast<int32_t>(left), static_cast<int32_t>(top),
                           static_cast<uint32_t>(std::max<Float>(0, right - left)),
                           static_cast<uint32_t>(std::max<Float>(0, bottom - top))};
  return placed;
}

void ShellSurfaceHost::sync() {
  if (!window_.shell()) return;
  std::set<Tag> mounted;
  for (const auto& [tag, entry] : tree_.views()) {
    const auto& view = entry.view;
    if (std::string(view.componentName ? view.componentName : "") != kShellSurfaceComponent) continue;
    auto placed = place(tag);
    if (!placed) continue;
    mounted.insert(tag);
    auto open = regions_.find(tag);
    if (open != regions_.end() && open->second.placed == *placed) continue;
    bool reopen = open == regions_.end() || open->second.placed.kind != placed->kind ||
                  open->second.placed.edge != placed->edge || open->second.placed.exclusive != placed->exclusive ||
                  !(open->second.placed.strip.size == placed->strip.size);
    if (reopen) {
      // A surface's role and size are fixed when it is made: a new strip is a new surface.
      if (open != regions_.end()) {
        window_.destroy(open->second.surface);
        accessibility_.forget(open->second.surface);
        Report::get().popup("shell.close", open->second.surface, &view);
      }
      uint32_t surface = window_.allocate();
      window_.openShellSurface(surface, placed->kind, placed->edge,
                               static_cast<uint32_t>(placed->strip.size.width),
                               static_cast<uint32_t>(placed->strip.size.height), placed->exclusive);
      window_.setInputRegion(surface, placed->input);
      regions_[tag] = Region{.surface = surface, .placed = *placed};
      Report::get().popup("shell.open", surface, &view,
                          folly::dynamic::object("kind", placed->kind == OXYOS_SHELL_OVERLAY ? "overlay" : "panel")(
                              "width", placed->strip.size.width)("height", placed->strip.size.height));
    } else {
      window_.setInputRegion(open->second.surface, placed->input);
      open->second.placed = *placed;
      open->second.dirty = true;
    }
  }
  for (auto it = regions_.begin(); it != regions_.end();) {
    if (mounted.count(it->first)) {
      ++it;
      continue;
    }
    window_.destroy(it->second.surface);
    accessibility_.forget(it->second.surface);
    Report::get().popup("shell.close", it->second.surface, nullptr);
    it = regions_.erase(it);
  }
}

void ShellSurfaceHost::changed() {
  for (auto& [tag, region] : regions_) region.dirty = true;
}

void ShellSurfaceHost::present(const TextInputEdit& edit, const std::string& focus) {
  // OXYOS_HOST_DAMAGE_CHECK: every frame drawn from its damage is also drawn whole and compared.
  static const bool checkDamage = std::getenv("OXYOS_HOST_DAMAGE_CHECK") != nullptr;
  for (auto& [tag, region] : regions_) {
    if (!window_.ready(region.surface)) continue;
    const auto* surface = window_.surface(region.surface);
    if (!region.dirty && surface->configure == region.drawnFor) continue;
    auto frame = windowFrame(tree_, tag);
    if (!frame) continue;
    Point at{frame->origin.x - region.placed.strip.origin.x, frame->origin.y - region.placed.strip.origin.y};
    Display display{surface->width, surface->height, surface->scale};
    auto started = std::chrono::steady_clock::now();
    // As the window (ADR 0005): only what changed is drawn again; a new configure, whole.
    auto damage = region.damage.update(tree_, tag, at, display, edit);
    bool configured = surface->configure != region.drawnFor;
    if (configured) damage.reset();
    if (damage && damage->empty()) {
      // Nothing visible changed (another region did): nothing is drawn or presented.
      accessibility_.publish(region.surface, tag, at);
      region.dirty = false;
      continue;
    }
    auto stats = damage ? drawRegionDamaged(tree_, tag, at, display, region.canvas, edit, *damage)
                        : drawRegion(tree_, tag, at, display, region.canvas, edit);
    double renderMs =
        std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - started).count();
    if (checkDamage && damage) {
      Canvas whole;
      drawRegion(tree_, tag, at, display, whole, edit);
      size_t differing = 0;
      for (size_t i = 0; i < whole.pixels.size() && i < region.canvas.pixels.size(); ++i) {
        differing += whole.pixels[i] != region.canvas.pixels[i];
      }
      if (differing) {
        std::cerr << "[host] a shell region's frame drawn from its damage differs in " << differing << " pixels\n";
        Report::get().damageMismatch(region.surface, differing);
      }
    }
    // The damage in the canvas's pixels, rounded outward.
    std::vector<OxyOSRect> pixels;
    for (const auto& rect : damage.value_or(std::vector<Rect>{})) {
      auto x0 = static_cast<int32_t>(std::floor(rect.origin.x * display.scale));
      auto y0 = static_cast<int32_t>(std::floor(rect.origin.y * display.scale));
      auto x1 = static_cast<int32_t>(std::ceil((rect.origin.x + rect.size.width) * display.scale));
      auto y1 = static_cast<int32_t>(std::ceil((rect.origin.y + rect.size.height) * display.scale));
      pixels.push_back(OxyOSRect{x0, y0, static_cast<uint32_t>(x1 - x0), static_cast<uint32_t>(y1 - y0)});
    }
    window_.present(region.surface, region.canvas, pixels);
    double area = 0;
    for (const auto& rect : pixels) area += static_cast<double>(rect.width) * rect.height;
    double whole = static_cast<double>(region.canvas.width) * region.canvas.height;
    double damaged = pixels.empty() ? 1.0 : std::min(1.0, area / std::max(whole, 1.0));
    Report::get().frame(region.surface, region.canvas.width, region.canvas.height, renderMs, stats.textContent, focus,
                        damaged);
    accessibility_.publish(region.surface, tag, at);
    region.dirty = false;
    region.drawnFor = surface->configure;
  }
}

std::optional<uint32_t> ShellSurfaceHost::surfaceOf(Tag view) const {
  for (Tag at : ancestry(tree_, view)) {
    auto it = regions_.find(at);
    if (it != regions_.end()) return it->second.surface;
  }
  return std::nullopt;
}

std::optional<SurfaceContent> ShellSurfaceHost::content(uint32_t surface) const {
  for (const auto& [tag, region] : regions_) {
    if (region.surface != surface) continue;
    return SurfaceContent{.surface = surface, .view = tag, .popup = true, .origin = region.placed.strip.origin};
  }
  return std::nullopt;
}

}  // namespace oxyos
