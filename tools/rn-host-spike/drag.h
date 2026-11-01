// Drag and drop between applications for the Bloom host (docs/adr/0035-drag-and-drop.mdx): the
// `OxyOSDropTargetView` host component (sample/oxyos/platform-surface.js renders it as the OxyOS
// adapter's DropTarget, Bloom's DropZone) and the drags the compositor passes over this client's
// surfaces; and the application's own drags (OxyOSDrag.start, Bloom's usePlatformDrag), offered
// from the press it holds, with a view's image as the drag's icon.
//
// The host answers the compositor from its own tree, never waiting on JavaScript: on each entering
// and motion it finds the innermost drop target under the point that takes a type offered, and
// tells the compositor that type and the target's actions. JavaScript follows: the target's
// dragEnter, dragOver and dragLeave, and at the drop its drop with the content, received through
// the pipes the source writes, from the host's loop, once complete.
//
// Every editable text input that is not secure is a drop target for text too, as an NSTextView is
// (docs/adr/0037-text-editing.mdx): TextEditing shows the drop caret and inserts what is dropped.
// The innermost target under the point takes the drag, so a field inside a drop target takes the
// text dropped on it, and the drop target around it what the field does not take (files).
#pragma once

#include "input.h"
#include "renderer.h"
#include "textinput.h"
#include "window.h"

#include <folly/dynamic.h>
#include <react/renderer/components/view/ConcreteViewShadowNode.h>
#include <react/renderer/components/view/ViewProps.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>

#include <chrono>
#include <functional>
#include <map>
#include <optional>
#include <string>
#include <vector>

namespace facebook::react {

extern const char OxyOSDropTargetViewComponentName[];

class OxyOSDropTargetViewProps final : public ViewProps {
 public:
  OxyOSDropTargetViewProps() = default;
  OxyOSDropTargetViewProps(const PropsParserContext& context, const OxyOSDropTargetViewProps& sourceProps,
                           const RawProps& rawProps);

  // The kinds of content it takes (Bloom's DropKind: files, text, url, html or media types), one a
  // line; its actions (copy, move, link), its preferred one first, one a line; whether it takes
  // nothing now.
  std::string acceptKinds;
  std::string dropActions;
  bool disabled{false};
};

using OxyOSDropTargetViewShadowNode =
    ConcreteViewShadowNode<OxyOSDropTargetViewComponentName, OxyOSDropTargetViewProps>;
using OxyOSDropTargetViewComponentDescriptor = ConcreteComponentDescriptor<OxyOSDropTargetViewShadowNode>;

}  // namespace facebook::react

namespace oxyos {

// The media types each of Bloom's kinds is carried as, the most faithful first; a kind that is a
// media type itself is carried as that type.
std::vector<std::string> kindTypes(const std::string& kind);
// The OXYOS_DRAG_* bit of an action's name (copy, move, link), 0 for any other; and the name of a
// bit, empty for none.
uint8_t actionBit(const std::string& action);
std::string actionName(uint8_t bit);
// What the application drags (Bloom's DragData: files as file:// URIs, text, url, html) as the
// media types it offers with their content, the most faithful first.
std::vector<std::pair<std::string, std::string>> dragContents(const folly::dynamic& data);

class DragDrop {
 public:
  using Surfaces = std::function<std::optional<SurfaceContent>(uint32_t surface)>;
  DragDrop(const ViewTree& tree, Window& window, Surfaces surfaces)
      : tree_(tree), window_(window), surfaces_(std::move(surfaces)) {}
  // The text inputs, which take text dropped on them.
  void editsText(TextEditing& editing) { editing_ = &editing; }

  // Whether the compositor has drag and drop (protocol version 10).
  bool available() const;
  // Every compositor event: a drag over this client's surfaces, what came of its own drag, and the
  // pointer presses a drag may start from.
  void observe(const OxyOSEvent& event);

  // The application drags `data` (Bloom's DragData) from the press it holds, allowing `actions`,
  // with the image of the view `preview` (-1: none) under the pointer where it was pressed. `done`
  // receives the action the drop took, or nothing when it was cancelled or refused (no press held,
  // nothing to offer). False if it was not started; `done` is then called at once.
  bool start(const folly::dynamic& data, const std::vector<std::string>& actions, facebook::react::Tag preview,
             std::function<void(std::optional<std::string>)> done);
  // The same with the media types offered and their content, the actions allowed (OXYOS_DRAG_*)
  // and the drag's image, if any: a text input's selected text (TextEditing).
  bool start(const std::vector<std::pair<std::string, std::string>>& contents, uint8_t allowed,
             std::optional<DragImage> image, std::function<void(std::optional<std::string>)> done);
  // Draws the drag's icon once the compositor configured it.
  void present();

  // Each turn: reads what drops have received; a drop complete is given to its target. True if
  // one was.
  bool deliver();
  // The pipes of the drops on their way, readable when more has arrived.
  std::vector<int> fds() const;
  // When the first drop on its way runs out of time, if one is.
  std::optional<std::chrono::steady_clock::time_point> nextDeadline() const;

  // A drop's limits: content beyond them is dropped and the drop given what arrived of nothing.
  static constexpr size_t kMaxBytes = 16 * 1024 * 1024;
  static constexpr std::chrono::seconds kTimeout{5};

 private:
  // The drag the compositor passes over one of this client's surfaces.
  struct Offer {
    uint32_t offer = 0;
    uint64_t serial = 0;
    uint32_t surface = 0;
    uint8_t sourceActions = 0;
    size_t expected = 0;
    std::vector<std::string> types;
    float x = 0, y = 0;  // surface-local, logical
    uint8_t action = 0;  // the action the compositor last told
    facebook::react::Tag view = -1;  // the drop target under the pointer
    bool field = false;  // that target is a text input
    bool own = false;    // the drag is this client's: it entered while one of its drags went on
    bool answered = false;
    std::string mime;
    uint8_t actions = 0, preferred = 0;
  };
  struct Pipe {
    int fd = -1;
    std::string mime;
    std::string bytes;
  };
  struct Receiving {
    uint32_t offer = 0;
    uint32_t surface = 0;
    facebook::react::Tag view = -1;
    folly::dynamic event;
    // A text input dropped on: where (surface-local), its text given it rather than an event.
    bool field = false;
    float x = 0, y = 0;
    std::vector<Pipe> pipes;
    std::chrono::steady_clock::time_point started;
  };
  // A target that takes something the offer carries: its props, or null.
  const facebook::react::OxyOSDropTargetViewProps* target(facebook::react::Tag view) const;
  // The kinds of a target that the offer carries, in the target's order.
  std::vector<std::string> offeredKinds(const facebook::react::OxyOSDropTargetViewProps& props) const;
  // The offered type a text input takes, the most faithful first; empty if none (files are the
  // drop target's around it, as a web page's input takes no files).
  std::string fieldType() const;
  // The drop target under the offer's point, the answer to give the compositor, and JavaScript
  // told of a target entered, moved over or left.
  void update();
  // The event a target is told: the point in it, the action, the kinds offered it takes.
  folly::dynamic event(facebook::react::Tag view) const;
  void dispatch(facebook::react::Tag view, const char* name, const folly::dynamic& payload);
  void leave();
  void drop(const OxyOSEvent& event);
  void endIcon();

  const ViewTree& tree_;
  Window& window_;
  Surfaces surfaces_;
  std::optional<Offer> offer_;
  std::vector<Receiving> receiving_;
  // The pointer press held, if any: its serial, its surface and where it was (root coordinates).
  uint64_t pressSerial_ = 0;
  uint32_t pressSurface_ = 0;
  facebook::react::Point pressPoint_{};
  // This client's drags going on or dropped, by number: what each one's caller is told; and the
  // one going on, not yet dropped (0: none), the drag over this client's surfaces meanwhile.
  std::map<uint32_t, std::function<void(std::optional<std::string>)>> drags_;
  uint32_t live_ = 0;
  // The drag's icon surface and how it is drawn, once configured.
  uint32_t icon_ = 0;
  std::function<void(const Display&, Canvas&)> iconDraw_;
  uint64_t iconDrawnFor_ = 0;
  TextEditing* editing_ = nullptr;
};

}  // namespace oxyos
