#include "drag.h"

#include "report.h"

#include <react/renderer/core/propsConversions.h>

#include <fcntl.h>
#include <unistd.h>

#include <algorithm>
#include <cerrno>
#include <cmath>
#include <cstring>
#include <iostream>
#include <set>
#include <sstream>

using namespace facebook::react;

namespace facebook::react {

const char OxyOSDropTargetViewComponentName[] = "OxyOSDropTargetView";

OxyOSDropTargetViewProps::OxyOSDropTargetViewProps(const PropsParserContext& context,
                                                   const OxyOSDropTargetViewProps& sourceProps,
                                                   const RawProps& rawProps)
    : ViewProps(context, sourceProps, rawProps),
      acceptKinds(convertRawProp(context, rawProps, "acceptKinds", sourceProps.acceptKinds, std::string())),
      dropActions(convertRawProp(context, rawProps, "dropActions", sourceProps.dropActions, std::string())),
      disabled(convertRawProp(context, rawProps, "disabled", sourceProps.disabled, false)) {}

}  // namespace facebook::react

namespace oxyos {

namespace {

std::vector<std::string> lines(const std::string& text) {
  std::vector<std::string> out;
  std::istringstream in(text);
  for (std::string line; std::getline(in, line);) {
    if (!line.empty() && line.back() == '\r') line.pop_back();
    if (!line.empty()) out.push_back(line);
  }
  return out;
}

std::string lower(std::string text) {
  std::transform(text.begin(), text.end(), text.begin(), [](unsigned char c) { return std::tolower(c); });
  return text;
}

bool isDropTarget(const ShadowView& view) {
  return view.componentName && std::strcmp(view.componentName, OxyOSDropTargetViewComponentName) == 0;
}

}  // namespace

std::vector<std::string> kindTypes(const std::string& kind) {
  if (kind == "files") return {"text/uri-list"};
  if (kind == "url") return {"text/uri-list", "text/x-moz-url"};
  if (kind == "text") return {"text/plain;charset=utf-8", "text/plain"};
  if (kind == "html") return {"text/html"};
  return {lower(kind)};
}

uint8_t actionBit(const std::string& action) {
  if (action == "copy") return OXYOS_DRAG_COPY;
  if (action == "move") return OXYOS_DRAG_MOVE;
  if (action == "link") return OXYOS_DRAG_LINK;
  return 0;
}

std::string actionName(uint8_t bit) {
  switch (bit) {
    case OXYOS_DRAG_COPY: return "copy";
    case OXYOS_DRAG_MOVE: return "move";
    case OXYOS_DRAG_LINK: return "link";
    default: return "";
  }
}

std::vector<std::pair<std::string, std::string>> dragContents(const folly::dynamic& data) {
  std::vector<std::pair<std::string, std::string>> out;
  if (!data.isObject()) return out;
  auto text = [&](const char* key) -> std::optional<std::string> {
    auto found = data.find(key);
    if (found == data.items().end() || !found->second.isString()) return std::nullopt;
    return found->second.asString();
  };
  std::string uris, plain;
  if (auto files = data.find("files"); files != data.items().end() && files->second.isArray()) {
    for (const auto& file : files->second) {
      if (!file.isString() || file.asString().rfind("file://", 0) != 0) continue;
      uris += file.asString() + "\r\n";
      plain += (plain.empty() ? "" : "\n") + file.asString();
    }
  }
  auto url = text("url");
  if (url && uris.empty()) {
    uris = *url + "\r\n";
    if (plain.empty()) plain = *url;
  }
  // A URI list first (what file managers and browsers take), then a link as Firefox and Chromium
  // name it, the HTML, and text: the URIs or links when nothing else was given as text.
  if (!uris.empty()) out.emplace_back("text/uri-list", uris);
  if (url) out.emplace_back("text/x-moz-url", *url);
  if (auto html = text("html")) out.emplace_back("text/html", *html);
  if (auto given = text("text")) plain = *given;
  if (!plain.empty()) out.emplace_back("text/plain;charset=utf-8", plain);
  return out;
}

bool DragDrop::available() const { return window_.protocolVersion() >= 10; }

const OxyOSDropTargetViewProps* DragDrop::target(Tag view) const {
  auto it = tree_.views().find(view);
  if (it == tree_.views().end() || !isDropTarget(it->second.view)) return nullptr;
  auto props = std::static_pointer_cast<const OxyOSDropTargetViewProps>(it->second.view.props);
  if (!props || props->disabled) return nullptr;
  return props.get();
}

std::vector<std::string> DragDrop::offeredKinds(const OxyOSDropTargetViewProps& props) const {
  std::vector<std::string> out;
  if (!offer_) return out;
  std::set<std::string> offered;
  for (const auto& type : offer_->types) offered.insert(lower(type));
  for (const auto& kind : lines(props.acceptKinds)) {
    auto types = kindTypes(kind);
    if (std::any_of(types.begin(), types.end(), [&](const std::string& type) { return offered.count(type); })) {
      out.push_back(kind);
    }
  }
  return out;
}

folly::dynamic DragDrop::event(Tag view) const {
  folly::dynamic kinds = folly::dynamic::array();
  float x = 0, y = 0;
  if (offer_) {
    if (const auto* props = target(view)) {
      for (const auto& kind : offeredKinds(*props)) kinds.push_back(kind);
    }
    auto content = surfaces_(offer_->surface);
    auto frame = windowFrame(tree_, view);
    if (content && frame) {
      x = content->origin.x + offer_->x - frame->origin.x;
      y = content->origin.y + offer_->y - frame->origin.y;
    }
  }
  std::string action = offer_ ? actionName(offer_->action) : "";
  return folly::dynamic::object("x", x)("y", y)("action", action.empty() ? folly::dynamic() : folly::dynamic(action))(
      "kinds", kinds);
}

void DragDrop::dispatch(Tag view, const char* name, const folly::dynamic& payload) {
  auto it = tree_.views().find(view);
  if (it == tree_.views().end() || !it->second.view.eventEmitter) return;
  it->second.view.eventEmitter->dispatchEvent(name, folly::dynamic(payload));
  Report::get().deliver(std::string("drag.") + name, offer_ ? offer_->surface : 0, it->second.view, payload);
}

void DragDrop::update() {
  if (!offer_ || offer_->types.size() < offer_->expected) return;
  Tag found = -1;
  std::string mime;
  auto content = surfaces_(offer_->surface);
  if (content) {
    Tag hit = hitTest(tree_, *content, offer_->x, offer_->y);
    auto path = ancestry(tree_, hit);
    // The innermost drop target that takes a type offered.
    for (auto at = path.rbegin(); at != path.rend() && found < 0; ++at) {
      const auto* props = target(*at);
      if (!props) continue;
      for (const auto& kind : offeredKinds(*props)) {
        for (const auto& type : kindTypes(kind)) {
          auto offered = std::find_if(offer_->types.begin(), offer_->types.end(),
                                      [&](const std::string& t) { return lower(t) == type; });
          if (offered != offer_->types.end()) {
            mime = *offered;
            break;
          }
        }
        if (!mime.empty()) break;
      }
      if (!mime.empty()) found = *at;
    }
  }
  uint8_t actions = 0, preferred = 0;
  if (const auto* props = target(found)) {
    for (const auto& action : lines(props->dropActions)) {
      uint8_t bit = actionBit(action);
      if (preferred == 0) preferred = bit;
      actions |= bit;
    }
    if (actions == 0) actions = preferred = OXYOS_DRAG_COPY;
  }
  // The compositor is answered for the entering, and again whenever the answer changes.
  if (!offer_->answered || mime != offer_->mime || actions != offer_->actions || preferred != offer_->preferred) {
    window_.acceptDrag(offer_->offer, offer_->serial, mime, actions, preferred);
    offer_->answered = true;
    offer_->mime = mime;
    offer_->actions = actions;
    offer_->preferred = preferred;
  }
  if (found != offer_->view) {
    if (offer_->view >= 0) dispatch(offer_->view, "dragLeave", folly::dynamic::object());
    offer_->view = found;
    if (found >= 0) dispatch(found, "dragEnter", event(found));
  } else if (found >= 0) {
    dispatch(found, "dragOver", event(found));
  }
}

void DragDrop::leave() {
  if (offer_ && offer_->view >= 0) dispatch(offer_->view, "dragLeave", folly::dynamic::object());
  offer_.reset();
}

void DragDrop::drop(const OxyOSEvent& event) {
  if (!offer_ || offer_->offer != event.code) return;
  offer_->x = event.x / 256.0f;
  offer_->y = event.y / 256.0f;
  offer_->action = static_cast<uint8_t>(event.detail);
  Tag view = offer_->view;
  const auto* props = target(view);
  if (!props) {
    // Dropped on nothing that takes it (the compositor drops only on an answer, so a target that
    // went meanwhile): nothing is received.
    window_.finishDrop(offer_->offer);
    leave();
    return;
  }
  Receiving receiving{.offer = offer_->offer, .surface = offer_->surface, .view = view, .event = this->event(view)};
  receiving.started = std::chrono::steady_clock::now();
  // Each type a kind it takes is carried as, once: the first a kind offers.
  std::set<std::string> asked;
  for (const auto& kind : offeredKinds(*props)) {
    for (const auto& type : kindTypes(kind)) {
      auto offered = std::find_if(offer_->types.begin(), offer_->types.end(),
                                  [&](const std::string& t) { return lower(t) == type; });
      if (offered == offer_->types.end()) continue;
      if (asked.insert(*offered).second) {
        int fd = window_.receiveDrop(offer_->offer, *offered);
        if (fd >= 0) {
          fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK);
          receiving.pipes.push_back(Pipe{.fd = fd, .mime = *offered});
        }
      }
      break;
    }
  }
  // The drag leaves the target as it is dropped on, as the web's does.
  dispatch(view, "dragLeave", folly::dynamic::object());
  offer_.reset();
  receiving_.push_back(std::move(receiving));
}

bool DragDrop::deliver() {
  bool finished = false;
  for (auto it = receiving_.begin(); it != receiving_.end();) {
    bool open = false, failed = false;
    for (auto& pipe : it->pipes) {
      if (pipe.fd < 0) continue;
      char buffer[64 * 1024];
      ssize_t got = 0;
      while ((got = read(pipe.fd, buffer, sizeof(buffer))) > 0 && pipe.bytes.size() <= kMaxBytes) {
        pipe.bytes.append(buffer, static_cast<size_t>(got));
      }
      bool error = got < 0 && errno != EAGAIN && errno != EWOULDBLOCK;
      if (pipe.bytes.size() > kMaxBytes || error) {
        std::cerr << "[host] a drop's " << pipe.mime << " failed: " << (error ? std::strerror(errno) : "more than 16 MiB")
                  << "\n";
        failed = true;
      }
      if (got == 0 || error || pipe.bytes.size() > kMaxBytes) {
        close(pipe.fd);
        pipe.fd = -1;
      } else {
        open = true;
      }
    }
    if (open && std::chrono::steady_clock::now() - it->started > kTimeout) {
      std::cerr << "[host] a drop had no end in time\n";
      for (auto& pipe : it->pipes) {
        if (pipe.fd >= 0) close(pipe.fd);
        pipe.fd = -1;
      }
      failed = true;
      open = false;
    }
    if (open) {
      ++it;
      continue;
    }
    folly::dynamic received = folly::dynamic::object();
    if (!failed) {
      for (const auto& pipe : it->pipes) received[pipe.mime] = pipe.bytes;
    }
    auto event = it->event;
    event["received"] = received;
    auto view = tree_.views().find(it->view);
    if (view != tree_.views().end() && view->second.view.eventEmitter) {
      view->second.view.eventEmitter->dispatchEvent("drop", folly::dynamic(event));
      Report::get().deliver("drag.drop", it->surface, view->second.view, event);
    }
    window_.finishDrop(it->offer);
    it = receiving_.erase(it);
    finished = true;
  }
  return finished;
}

std::vector<int> DragDrop::fds() const {
  std::vector<int> out;
  for (const auto& receiving : receiving_) {
    for (const auto& pipe : receiving.pipes) {
      if (pipe.fd >= 0) out.push_back(pipe.fd);
    }
  }
  return out;
}

std::optional<std::chrono::steady_clock::time_point> DragDrop::nextDeadline() const {
  std::optional<std::chrono::steady_clock::time_point> next;
  for (const auto& receiving : receiving_) {
    auto due = receiving.started + kTimeout;
    if (!next || due < *next) next = due;
  }
  return next;
}

void DragDrop::observe(const OxyOSEvent& event) {
  switch (event.kind) {
    case OXYOS_EVENT_POINTER:
      if (event.detail == OXYOS_POINTER_PRESS) {
        pressSerial_ = event.serial;
        pressSurface_ = event.surface;
        auto content = surfaces_(event.surface);
        Point origin = content ? content->origin : Point{};
        pressPoint_ = Point{origin.x + event.x / 256.0f, origin.y + event.y / 256.0f};
      } else if (event.detail == OXYOS_POINTER_RELEASE || event.detail == OXYOS_POINTER_CANCEL) {
        pressSerial_ = 0;
      }
      return;
    case OXYOS_EVENT_DRAG_ENTER:
      leave();
      offer_ = Offer{.offer = event.code,
                     .serial = event.serial,
                     .surface = event.surface,
                     .sourceActions = static_cast<uint8_t>(event.detail),
                     .expected = event.width,
                     .x = event.x / 256.0f,
                     .y = event.y / 256.0f};
      update();
      return;
    case OXYOS_EVENT_DRAG_OFFER_TYPE:
      if (offer_ && offer_->offer == event.code && offer_->types.size() < offer_->expected) {
        offer_->types.emplace_back(event.text);
        update();
      }
      return;
    case OXYOS_EVENT_DRAG_MOTION:
      if (offer_ && offer_->offer == event.code) {
        offer_->x = event.x / 256.0f;
        offer_->y = event.y / 256.0f;
        update();
      }
      return;
    case OXYOS_EVENT_DRAG_ACTION:
      if (offer_ && offer_->offer == event.code && offer_->action != event.detail) {
        offer_->action = static_cast<uint8_t>(event.detail);
        if (offer_->view >= 0) dispatch(offer_->view, "dragOver", this->event(offer_->view));
      }
      return;
    case OXYOS_EVENT_DRAG_LEAVE:
      if (offer_ && offer_->offer == event.code) leave();
      return;
    case OXYOS_EVENT_DRAG_DROP:
      drop(event);
      return;
    case OXYOS_EVENT_DRAG_FINISHED:
    case OXYOS_EVENT_DRAG_CANCELLED: {
      auto it = drags_.find(event.code);
      if (it == drags_.end()) return;
      auto done = std::move(it->second);
      drags_.erase(it);
      if (drags_.empty()) endIcon();
      std::string action = event.kind == OXYOS_EVENT_DRAG_FINISHED ? actionName(static_cast<uint8_t>(event.detail)) : "";
      Report::get().input(event.kind == OXYOS_EVENT_DRAG_FINISHED ? "drag.finished" : "drag.cancelled", pressSurface_,
                          folly::dynamic::object("drag", event.code)("detail", event.detail));
      done(action.empty() ? std::nullopt : std::optional<std::string>(action));
      return;
    }
    case OXYOS_EVENT_DRAG_DROPPED:
      // The target receives what it wants, then finishes: the icon is no longer shown already.
      endIcon();
      return;
    default:
      return;
  }
}

bool DragDrop::start(const folly::dynamic& data, const std::vector<std::string>& actions, Tag preview,
                     std::function<void(std::optional<std::string>)> done) {
  uint8_t allowed = 0;
  for (const auto& action : actions) allowed |= actionBit(action);
  auto contents = dragContents(data);
  if (!available() || pressSerial_ == 0 || allowed == 0 || contents.empty() || contents.size() > 16) {
    done(std::nullopt);
    return false;
  }
  uint32_t icon = 0;
  endIcon();
  if (auto frame = preview >= 0 ? windowFrame(tree_, preview) : std::nullopt;
      frame && frame->size.width >= 1 && frame->size.height >= 1) {
    icon = window_.allocate();
    auto width = static_cast<uint32_t>(std::ceil(frame->size.width));
    auto height = static_cast<uint32_t>(std::ceil(frame->size.height));
    // Where it was pressed stays under the pointer, as a Mac's dragged image keeps its grip.
    auto hotspotX = static_cast<int32_t>(std::lround(std::clamp(pressPoint_.x - frame->origin.x, 0.0f, frame->size.width)));
    auto hotspotY = static_cast<int32_t>(std::lround(std::clamp(pressPoint_.y - frame->origin.y, 0.0f, frame->size.height)));
    window_.createDragIcon(icon, width, height, hotspotX, hotspotY);
    icon_ = icon;
    iconView_ = preview;
    iconDrawnFor_ = 0;
  }
  uint32_t drag = window_.startDrag(pressSurface_, icon, pressSerial_, allowed, contents);
  if (drag == 0) {
    endIcon();
    done(std::nullopt);
    return false;
  }
  folly::dynamic types = folly::dynamic::array();
  for (const auto& [mime, _] : contents) types.push_back(mime);
  Report::get().input("drag.start", pressSurface_, folly::dynamic::object("drag", drag)("types", types)("actions", allowed));
  drags_[drag] = std::move(done);
  return true;
}

void DragDrop::present() {
  if (icon_ == 0 || !window_.ready(icon_)) return;
  const auto* surface = window_.surface(icon_);
  if (!surface || surface->configure == iconDrawnFor_) return;
  Canvas canvas;
  drawDragImage(tree_, iconView_, Display{surface->width, surface->height, surface->scale}, canvas);
  window_.present(icon_, canvas);
  iconDrawnFor_ = surface->configure;
}

void DragDrop::endIcon() {
  if (icon_ == 0) return;
  window_.destroy(icon_);
  icon_ = 0;
  iconView_ = -1;
  iconDrawnFor_ = 0;
}

}  // namespace oxyos
