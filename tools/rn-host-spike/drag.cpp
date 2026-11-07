#include "drag.h"
#include "text-util.h"

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
      disabled(convertRawProp(context, rawProps, "disabled", sourceProps.disabled, false)),
      acceptFileTypes(convertRawProp(context, rawProps, "acceptFileTypes", sourceProps.acceptFileTypes, std::string())),
      dropFolder(convertRawProp(context, rawProps, "dropFolder", sourceProps.dropFolder, std::string())) {}

}  // namespace facebook::react

namespace oxyos {

namespace {

constexpr char kFilesType[] = "application/x-oxyos-files";

std::vector<std::string> lines(const std::string& text) {
  std::vector<std::string> out;
  std::istringstream in(text);
  for (std::string line; std::getline(in, line);) {
    if (!line.empty() && line.back() == '\r') line.pop_back();
    if (!line.empty()) out.push_back(line);
  }
  return out;
}

// A media type without its parameters, lowercase.
std::string plainType(const std::string& type) {
  auto plain = lowercase(type.substr(0, type.find(';')));
  auto first = plain.find_first_not_of(" \t");
  auto last = plain.find_last_not_of(" \t");
  return first == std::string::npos ? std::string() : plain.substr(first, last - first + 1);
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
  return {lowercase(kind)};
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
  const bool files = !uris.empty();
  auto url = text("url");
  if (url && uris.empty()) {
    uris = *url + "\r\n";
    if (plain.empty()) plain = *url;
  }
  // A URI list first (what file managers and browsers take), then a link as Firefox and Chromium
  // name it, the HTML, and text: the URIs or links when nothing else was given as text.
  if (!uris.empty()) out.emplace_back("text/uri-list", uris);
  // Files are said to be files before the drop, where a target sees only the types (a web page's
  // `Files`, docs/adr/0035-drag-and-drop.mdx, decision 4).
  if (files) out.emplace_back(kFilesType, uris);
  if (url) out.emplace_back("text/x-moz-url", *url);
  if (auto html = text("html")) out.emplace_back("text/html", *html);
  if (auto given = text("text")) plain = *given;
  if (!plain.empty()) out.emplace_back("text/plain;charset=utf-8", plain);
  // Other media types by name, with their text, after the kinds (Bloom's DragData.other: an
  // application dragged out of a launcher, `application/x-oxyos-application`).
  if (auto other = data.find("other"); other != data.items().end() && other->second.isObject()) {
    for (const auto& [key, value] : other->second.items()) {
      if (!key.isString() || !value.isString()) continue;
      auto mime = lowercase(key.asString());
      bool named = std::any_of(out.begin(), out.end(), [&](const auto& entry) { return entry.first == mime; });
      if (!mime.empty() && !named) out.emplace_back(mime, value.asString());
    }
  }
  return out;
}

std::string dragFileTypes(const folly::dynamic& data) {
  std::vector<std::string> types;
  auto files = data.isObject() ? data.find("files") : data.items().end();
  if (!data.isObject() || files == data.items().end() || !files->second.isArray()) return {};
  for (const auto& file : files->second) {
    if (!file.isString() || file.asString().rfind("file://", 0) != 0) continue;
    const auto& uri = file.asString();
    char type[256];
    int length = oxyos_files_media_type(uri.data(), uri.size(), type, sizeof type);
    if (length <= 0) continue;
    std::string named(type, static_cast<size_t>(length));
    if (std::find(types.begin(), types.end(), named) == types.end() && types.size() < 16) types.push_back(named);
  }
  std::string out;
  for (const auto& type : types) out += type + "\n";
  return out;
}

std::string dragFileVolumes(const folly::dynamic& data) {
  std::vector<std::string> volumes;
  auto files = data.isObject() ? data.find("files") : data.items().end();
  if (!data.isObject() || files == data.items().end() || !files->second.isArray()) return {};
  for (const auto& file : files->second) {
    if (!file.isString() || file.asString().rfind("file://", 0) != 0) continue;
    const auto& uri = file.asString();
    char volume[64];
    int length = oxyos_files_volume(uri.data(), uri.size(), volume, sizeof volume);
    if (length <= 0) continue;
    std::string named(volume, static_cast<size_t>(length));
    if (std::find(volumes.begin(), volumes.end(), named) == volumes.end() && volumes.size() < 16) {
      volumes.push_back(named);
    }
  }
  std::string out;
  for (const auto& volume : volumes) out += volume + "\n";
  return out;
}

std::optional<uint8_t> folderPreference(const std::vector<std::string>& volumes, const std::string& folderVolume) {
  if (volumes.empty() || folderVolume.empty()) return std::nullopt;
  bool same = std::all_of(volumes.begin(), volumes.end(), [&](const std::string& one) { return one == folderVolume; });
  return same ? OXYOS_DRAG_MOVE : OXYOS_DRAG_COPY;
}

bool filesTaken(const std::vector<std::string>& fileTypes, const std::vector<std::string>& accepted) {
  if (fileTypes.empty()) return false;
  return std::all_of(fileTypes.begin(), fileTypes.end(), [&](const std::string& type) {
    auto kind = plainType(type);
    auto slash = kind.find('/');
    if (slash == std::string::npos) return false;
    return std::any_of(accepted.begin(), accepted.end(), [&](const std::string& one) {
      auto taken = plainType(one);
      return taken == kind || taken == kind.substr(0, slash) + "/*";
    });
  });
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
  for (const auto& type : offer_->types) offered.insert(lowercase(type));
  auto acceptedFiles = lines(props.acceptFileTypes);
  for (const auto& kind : lines(props.acceptKinds)) {
    auto types = kindTypes(kind);
    if (!std::any_of(types.begin(), types.end(), [&](const std::string& type) { return offered.count(type); })) {
      continue;
    }
    // Files whose types it does not take are no files to it (Bloom's acceptFileTypes).
    if (kind == "files" && !acceptedFiles.empty() && !filesTaken(offer_->fileTypes, acceptedFiles)) continue;
    out.push_back(kind);
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
  // The types of the files it carries, as its source declared them (Bloom's fileTypes).
  folly::dynamic fileTypes = folly::dynamic::array();
  if (offer_) {
    for (const auto& type : offer_->fileTypes) fileTypes.push_back(type);
  }
  return folly::dynamic::object("x", x)("y", y)("action", action.empty() ? folly::dynamic() : folly::dynamic(action))(
      "kinds", kinds)("fileTypes", fileTypes);
}

void DragDrop::dispatch(Tag view, const char* name, const folly::dynamic& payload) {
  auto it = tree_.views().find(view);
  if (it == tree_.views().end() || !it->second.view.eventEmitter) return;
  it->second.view.eventEmitter->dispatchEvent(name, folly::dynamic(payload));
  Report::get().deliver(std::string("drag.") + name, offer_ ? offer_->surface : 0, it->second.view, payload);
}

std::string DragDrop::fieldType() const {
  if (!offer_) return {};
  auto offered = [&](const char* type) -> std::optional<std::string> {
    for (const auto& t : offer_->types) {
      if (lowercase(t) == type) return t;
    }
    return std::nullopt;
  };
  if (offered(kFilesType)) return {};
  for (const char* type : {"text/plain;charset=utf-8", "text/plain", "text/uri-list"}) {
    if (auto found = offered(type)) return *found;
  }
  return {};
}

std::optional<std::string> DragDrop::folderVolume(const std::string& folder) {
  auto known = folderVolumes_.find(folder);
  if (known != folderVolumes_.end()) return known->second;
  char volume[64];
  int length = oxyos_files_folder_volume(folder.data(), folder.size(), volume, sizeof volume);
  std::optional<std::string> found;
  if (length > 0) found = std::string(volume, static_cast<size_t>(length));
  folderVolumes_.emplace(folder, found);
  return found;
}

void DragDrop::update(bool moved) {
  if (!offer_ || offer_->types.size() < offer_->expected || offer_->fileTypes.size() < offer_->fileExpected ||
      offer_->volumes.size() < offer_->volumesExpected) {
    return;
  }
  Tag found = -1;
  bool field = false;
  std::string mime, kindTaken;
  uint8_t actions = 0, preferred = 0;
  auto content = surfaces_(offer_->surface);
  if (content) {
    Tag hit = hitTest(tree_, *content, offer_->x, offer_->y);
    auto path = ancestry(tree_, hit);
    // The innermost drop target that takes a type offered: a text input that takes text, or a
    // drop target (Bloom's DropZone).
    for (auto at = path.rbegin(); at != path.rend() && found < 0; ++at) {
      if (editing_ && editing_->takesDrop(*at)) {
        std::string type = fieldType();
        if (type.empty()) continue;
        found = *at;
        field = true;
        // It shows where the text would go; over the very text it drags, it takes nothing. Text
        // dragged within one field moves, and copies from anywhere else, as an NSTextView's does.
        if (auto move = editing_->dragOver(found, offer_->surface, offer_->x, offer_->y, offer_->own)) {
          mime = type;
          actions = OXYOS_DRAG_COPY | OXYOS_DRAG_MOVE;
          preferred = *move ? OXYOS_DRAG_MOVE : OXYOS_DRAG_COPY;
        }
        break;
      }
      const auto* props = target(*at);
      if (!props) continue;
      for (const auto& kind : offeredKinds(*props)) {
        for (const auto& type : kindTypes(kind)) {
          auto offered = std::find_if(offer_->types.begin(), offer_->types.end(),
                                      [&](const std::string& t) { return lowercase(t) == type; });
          if (offered != offer_->types.end()) {
            mime = *offered;
            kindTaken = kind;
            break;
          }
        }
        if (!mime.empty()) break;
      }
      if (!mime.empty()) found = *at;
    }
  }
  if (!field && editing_) editing_->dragLeft();
  if (const auto* props = field ? nullptr : target(found)) {
    for (const auto& action : lines(props->dropActions)) {
      uint8_t bit = actionBit(action);
      if (preferred == 0) preferred = bit;
      actions |= bit;
    }
    if (actions == 0) actions = preferred = OXYOS_DRAG_COPY;
    // Files into its folder: a move on the folder's volume, a copy onto another, as Finder does,
    // when it allows both (Option and Command still force either: the compositor's).
    bool both = (actions & OXYOS_DRAG_COPY) && (actions & OXYOS_DRAG_MOVE);
    if (kindTaken == "files" && both && !props->dropFolder.empty()) {
      if (auto volume = folderVolume(props->dropFolder)) {
        if (auto finder = folderPreference(offer_->volumes, *volume)) preferred = *finder;
      }
    }
  }
  // The compositor is answered for the entering, and again whenever the answer changes.
  if (!offer_->answered || mime != offer_->mime || actions != offer_->actions || preferred != offer_->preferred) {
    window_.acceptDrag(offer_->offer, offer_->serial, mime, actions, preferred);
    offer_->answered = true;
    offer_->mime = mime;
    offer_->actions = actions;
    offer_->preferred = preferred;
  }
  // JavaScript hears of the drop targets; a text input is the platform's, as an EditText's drags
  // are its own.
  Tag was = offer_->field ? -1 : offer_->view, now = field ? -1 : found;
  offer_->view = found;
  offer_->field = field;
  if (now != was) {
    if (was >= 0) dispatch(was, "dragLeave", folly::dynamic::object());
    if (now >= 0) dispatch(now, "dragEnter", event(now));
  } else if (now >= 0 && moved) {
    dispatch(now, "dragOver", event(now));
  }
}

void DragDrop::mounted() {
  if (offer_) update(false);
}

void DragDrop::tellWindow(folly::dynamic event) {
  Report::get().input("drag.window", offer_ ? offer_->surface : 0, event);
  if (tellWindow_) tellWindow_(event);
}

void DragDrop::observed() {
  if (presence_ == Presence::Over && !offer_) {
    presence_ = Presence::Outside;
    tellWindow(folly::dynamic::object("type", "exit")("dropped", false));
  }
}

void DragDrop::ranJavaScript() {
  if (presence_ != Presence::Delivered) return;
  presence_ = Presence::Outside;
  tellWindow(folly::dynamic::object("type", "exit")("dropped", true));
  // A drag came over this client while that drop was delivered: it is the window's now.
  if (offer_) {
    presence_ = Presence::Over;
    tellWindow(folly::dynamic::object("type", "enter"));
  }
}

void DragDrop::leave() {
  if (offer_ && offer_->view >= 0 && !offer_->field) dispatch(offer_->view, "dragLeave", folly::dynamic::object());
  if (editing_) editing_->dragLeft();
  offer_.reset();
}

void DragDrop::drop(const OxyOSEvent& event) {
  if (!offer_ || offer_->offer != event.code) return;
  // Dropped on this client: the window's drag ends once the drop is delivered.
  presence_ = Presence::Dropped;
  offer_->x = event.x / 256.0f;
  offer_->y = event.y / 256.0f;
  offer_->action = static_cast<uint8_t>(event.detail);
  Tag view = offer_->view;
  if (offer_->field && editing_) {
    editing_->dragLeft();
    std::string type = fieldType();
    // The field's own text dropped back into it is moved (or copied) there at once, as one edit.
    bool inPlace = offer_->own && editing_->dropOwn(view, offer_->surface, offer_->x, offer_->y,
                                                    offer_->action == OXYOS_DRAG_COPY);
    int fd = inPlace || type.empty() || !editing_->takesDrop(view) ? -1 : window_.receiveDrop(offer_->offer, type);
    if (fd < 0) {
      window_.finishDrop(offer_->offer);
      offer_.reset();
      presence_ = Presence::Delivered;
      return;
    }
    fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK);
    Receiving receiving{.offer = offer_->offer, .surface = offer_->surface, .view = view, .field = true,
                        .x = offer_->x, .y = offer_->y};
    receiving.started = std::chrono::steady_clock::now();
    receiving.pipes.push_back(Pipe{.fd = fd, .mime = type});
    offer_.reset();
    receiving_.push_back(std::move(receiving));
    return;
  }
  const auto* props = target(view);
  if (!props) {
    // Dropped on nothing that takes it (the compositor drops only on an answer, so a target that
    // went meanwhile): nothing is received.
    window_.finishDrop(offer_->offer);
    leave();
    presence_ = Presence::Delivered;
    return;
  }
  Receiving receiving{.offer = offer_->offer, .surface = offer_->surface, .view = view, .event = this->event(view)};
  receiving.started = std::chrono::steady_clock::now();
  // Each type a kind it takes is carried as, once: the first a kind offers.
  std::set<std::string> asked;
  for (const auto& kind : offeredKinds(*props)) {
    for (const auto& type : kindTypes(kind)) {
      auto offered = std::find_if(offer_->types.begin(), offer_->types.end(),
                                  [&](const std::string& t) { return lowercase(t) == type; });
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
    if (it->field) {
      // A text input takes the text, or a URI list's URIs as text, one a line.
      if (!failed && editing_ && !it->pipes.empty()) {
        const auto& pipe = it->pipes.front();
        std::string text = pipe.bytes;
        if (lowercase(pipe.mime) == "text/uri-list") {
          text.clear();
          for (const auto& line : lines(pipe.bytes)) {
            if (line[0] != '#') text += (text.empty() ? "" : "\n") + line;
          }
        }
        if (!text.empty()) editing_->dropText(it->view, it->surface, it->x, it->y, std::move(text));
      }
      window_.finishDrop(it->offer);
      it = receiving_.erase(it);
      finished = true;
      if (presence_ == Presence::Dropped) presence_ = Presence::Delivered;
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
    // Its onDrop runs with JavaScript's next events; the window's drag ends after (ranJavaScript).
    if (presence_ == Presence::Dropped) presence_ = Presence::Delivered;
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
                     .fileExpected = event.height,
                     .volumesExpected = event.flag,
                     .x = event.x / 256.0f,
                     .y = event.y / 256.0f,
                     .own = live_ != 0};
      folderVolumes_.clear();
      // The first of this client's surfaces the drag comes over: it is over the window now (a drop
      // still being delivered tells its end first, ranJavaScript).
      if (presence_ == Presence::Outside) {
        presence_ = Presence::Over;
        tellWindow(folly::dynamic::object("type", "enter"));
      }
      update();
      return;
    case OXYOS_EVENT_DRAG_OFFER_TYPE:
      if (offer_ && offer_->offer == event.code && offer_->types.size() < offer_->expected) {
        offer_->types.emplace_back(event.text);
        update();
      }
      return;
    // A chunk of a type of the files it carries (version 26), complete with its last.
    case OXYOS_EVENT_DRAG_OFFER_FILE_TYPE:
      if (offer_ && offer_->offer == event.code && offer_->fileTypes.size() < offer_->fileExpected) {
        if (event.detail == 0) offer_->fileTypeChunks.clear();
        offer_->fileTypeChunks += event.text;
        if (event.flag & 1) {
          offer_->fileTypes.push_back(std::move(offer_->fileTypeChunks));
          offer_->fileTypeChunks.clear();
          update();
        }
      }
      return;
    // A chunk of a volume its files are on (version 28), complete with its last.
    case OXYOS_EVENT_DRAG_OFFER_FILE_VOLUME:
      if (offer_ && offer_->offer == event.code && offer_->volumes.size() < offer_->volumesExpected) {
        if (event.detail == 0) offer_->volumeChunks.clear();
        offer_->volumeChunks += event.text;
        if (event.flag & 1) {
          offer_->volumes.push_back(std::move(offer_->volumeChunks));
          offer_->volumeChunks.clear();
          update();
        }
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
        if (offer_->view >= 0 && !offer_->field) dispatch(offer_->view, "dragOver", this->event(offer_->view));
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
      if (event.code == live_) live_ = 0;
      auto it = drags_.find(event.code);
      if (it == drags_.end()) return;
      auto done = std::move(it->second);
      drags_.erase(it);
      auto told = cancelled_.find(event.code);
      if (told != cancelled_.end()) {
        auto cancelled = std::move(told->second);
        cancelled_.erase(told);
        // Why it ended without a drop, as Bloom's onCancel names it, before its promise settles.
        if (event.kind == OXYOS_EVENT_DRAG_CANCELLED && cancelled) {
          cancelled(event.detail == OXYOS_DRAG_CANCEL_ESCAPE         ? "escape"
                    : event.detail == OXYOS_DRAG_CANCEL_NOT_ACCEPTED ? "not-accepted"
                    : event.detail == OXYOS_DRAG_CANCEL_REFUSED      ? "refused"
                                                                     : "cancelled");
        }
      }
      if (drags_.empty()) endIcon();
      std::string action = event.kind == OXYOS_EVENT_DRAG_FINISHED ? actionName(static_cast<uint8_t>(event.detail)) : "";
      Report::get().input(event.kind == OXYOS_EVENT_DRAG_FINISHED ? "drag.finished" : "drag.cancelled", pressSurface_,
                          folly::dynamic::object("drag", event.code)("detail", event.detail));
      done(action.empty() ? std::nullopt : std::optional<std::string>(action));
      return;
    }
    case OXYOS_EVENT_DRAG_DROPPED:
      // The target receives what it wants, then finishes: the icon is no longer shown already.
      if (event.code == live_) live_ = 0;
      endIcon();
      return;
    default:
      return;
  }
}

bool DragDrop::start(const folly::dynamic& data, const std::vector<std::string>& actions, Tag preview, int count,
                     bool vanish, std::function<void(std::optional<std::string>)> done,
                     std::function<void(const std::string&)> cancelled) {
  uint8_t allowed = 0;
  for (const auto& action : actions) allowed |= actionBit(action);
  std::optional<DragImage> image;
  auto frame = preview >= 0 ? windowFrame(tree_, preview) : std::nullopt;
  if (auto stack = frame ? dragStack(tree_, preview, count) : DragStack{}; stack.size.width >= 1) {
    // The point pressed in the view stays under the pointer, as a Mac's dragged image keeps its
    // grip: where the press was in what the view draws, wherever that is in the image.
    Point grip{pressPoint_.x - frame->origin.x - stack.drawn.origin.x,
               pressPoint_.y - frame->origin.y - stack.drawn.origin.y};
    image = DragImage{.size = stack.size,
                      .hotspot = {stack.at.x + std::clamp(grip.x, 0.0f, stack.drawn.size.width),
                                  stack.at.y + std::clamp(grip.y, 0.0f, stack.drawn.size.height)},
                      .draw = [this, preview, stack](const Display& display, Canvas& canvas) {
                        drawDragImage(tree_, preview, stack, display, canvas);
                      }};
  }
  return start(dragContents(data), allowed, std::move(image), std::move(done), vanish, dragFileTypes(data),
               std::move(cancelled), dragFileVolumes(data));
}

bool DragDrop::start(const std::vector<std::pair<std::string, std::string>>& contents, uint8_t allowed,
                     std::optional<DragImage> image, std::function<void(std::optional<std::string>)> done,
                     bool vanish, const std::string& fileTypes,
                     std::function<void(const std::string&)> cancelled, const std::string& volumes) {
  if (!available() || pressSerial_ == 0 || allowed == 0 || contents.empty() || contents.size() > 16) {
    if (cancelled) cancelled("refused");
    done(std::nullopt);
    return false;
  }
  uint32_t icon = 0;
  endIcon();
  if (image && image->size.width >= 1 && image->size.height >= 1) {
    icon = window_.allocate();
    auto width = static_cast<uint32_t>(std::ceil(image->size.width));
    auto height = static_cast<uint32_t>(std::ceil(image->size.height));
    window_.createDragIcon(icon, width, height, static_cast<int32_t>(std::lround(image->hotspot.x)),
                           static_cast<int32_t>(std::lround(image->hotspot.y)));
    icon_ = icon;
    iconDraw_ = std::move(image->draw);
    iconDrawnFor_ = 0;
  }
  uint32_t drag = window_.startDrag(pressSurface_, icon, pressSerial_, allowed, contents, vanish, fileTypes, volumes);
  if (drag == 0) {
    endIcon();
    if (cancelled) cancelled("refused");
    done(std::nullopt);
    return false;
  }
  folly::dynamic types = folly::dynamic::array();
  for (const auto& [mime, _] : contents) types.push_back(mime);
  Report::get().input("drag.start", pressSurface_,
                      folly::dynamic::object("drag", drag)("types", types)("actions", allowed)("vanish", vanish)(
                          "fileTypes", fileTypes)("volumes", volumes));
  drags_[drag] = std::move(done);
  if (cancelled) cancelled_[drag] = std::move(cancelled);
  live_ = drag;
  return true;
}

void DragDrop::present() {
  if (icon_ == 0 || !window_.ready(icon_)) return;
  const auto* surface = window_.surface(icon_);
  if (!surface || surface->configure == iconDrawnFor_) return;
  Canvas canvas;
  if (iconDraw_) iconDraw_(Display{surface->width, surface->height, surface->scale}, canvas);
  window_.present(icon_, canvas);
  iconDrawnFor_ = surface->configure;
}

void DragDrop::endIcon() {
  if (icon_ == 0) return;
  window_.destroy(icon_);
  icon_ = 0;
  iconDraw_ = nullptr;
  iconDrawnFor_ = 0;
}

}  // namespace oxyos
