// Drag and drop between applications for the Bloom host (docs/adr/0035-drag-and-drop.mdx): the
// `OxyOSDropTargetView` host component (sample/oxyos/platform-surface.js renders it as the OxyOS
// adapter's DropTarget, Bloom's DropZone) and the drags the compositor passes over this client's
// surfaces; and the application's own drags (OxyOSDrag.start, Bloom's usePlatformDrag), offered
// from its latest press, with a view's image as the drag's icon.
//
// The host answers the compositor from its own tree, never waiting on JavaScript: on each entering
// and motion it finds the innermost drop target under the point that takes a type offered, and
// tells the compositor that type and the target's actions. JavaScript follows: the target's
// dragEnter, dragOver and dragLeave, and at the drop its drop with the content, received through
// the pipes the source writes, from the host's loop, once complete.
//
// A drag of files says the volumes its files are on (protocol version 28,
// docs/adr/0047-finder-drags-and-the-desktop.mdx), and a drop target taking files into a folder
// (its `dropFolder`, Bloom's DropZone `folder`) prefers a move when each is that folder's volume and
// a copy when one is not, as Finder does. When a target's props change during a drag (a commit), the
// compositor is answered again at once.
//
// The window's drag (Bloom's useWindowDrag, the adapter's drag.watchWindow): JavaScript is told when
// a drag first comes over any of this client's surfaces and when it has left them all or ended,
// whether it was dropped on one of them, told once that drop was delivered (after its onDrop).
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
  // The media types (or `type/*`) of the files it takes, one a line (Bloom's acceptFileTypes,
  // version 26): a drag of files is taken as `files` only when each of its files' types matches
  // one; empty, any files.
  std::string acceptFileTypes;
  // The folder files dropped on it go into, a file:// URI (Bloom's DropZone `folder`, version 28):
  // a drag of files that says its volumes is answered with a move preferred when each is this
  // folder's volume, and a copy when one is not, if it allows both. Empty: its preferred action.
  std::string dropFolder;
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
// What the application drags (Bloom's DragData: files as file:// URIs, text, url, html, and other
// media types by name with their text) as the media types it offers with their content, the most
// faithful first, the others after.
std::vector<std::pair<std::string, std::string>> dragContents(const folly::dynamic& data);
// The media types of the files a drag of `data` carries (each file:// URI's, as the platform names
// it: oxyos_files_media_type), distinct, one a line; empty for none.
std::string dragFileTypes(const folly::dynamic& data);
// The volumes the files of a drag of `data` are on (each file:// URI's, as the platform names it:
// oxyos_files_volume, the item's own), distinct, at most 16, one a line; empty for none.
std::string dragFileVolumes(const folly::dynamic& data);
// The action a target taking files into a folder on `folderVolume` prefers for a drag of files on
// `volumes`, as Finder does: a move when each is the folder's volume, a copy when one is not. None
// (keep its own preference) when the drag told no volumes or the folder's is not known.
std::optional<uint8_t> folderPreference(const std::vector<std::string>& volumes, const std::string& folderVolume);
// Whether a drag of files of `fileTypes` is one a target accepting `accepted` (media types or
// `type/*`, case and parameters aside) takes: every one matches; none told, not taken.
bool filesTaken(const std::vector<std::string>& fileTypes, const std::vector<std::string>& accepted);

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
  // The events of one turn were all observed: a drag that left this client's last surface (and did
  // not enter another of them meanwhile) has left the window.
  void observed();
  // The tree changed (a commit mounted): the drop target under the drag, and its props, may have,
  // so the compositor is answered again, at once rather than at the next motion. The target is not
  // told dragOver for it (that would ask JavaScript for the commit that told it).
  void mounted();
  // JavaScript ran the events queued before it (the loop's beat and drain): a drop it was given is
  // delivered, and the window's drag that ended with it can be told.
  void ranJavaScript();
  // The batch of JavaScript's that asked for drags ran, with all it set off, and what it committed
  // is mounted (bloom-host.cpp): the drags it asked start (`start` from JavaScript), each from the
  // tree as it is now. Whether any was asked.
  bool startAsked();
  // Drags JavaScript asked for that have not started yet.
  bool asking() const { return !asked_.empty(); }
  // Who is given the files dropped on a target of the application's (`file://` URIs): the person
  // gave them to it (docs/adr/0030-installing-applications.mdx, decision 8), before its onDrop.
  void givesFiles(std::function<void(const std::string& uri)> give) { giveFiles_ = std::move(give); }
  // Who is told the window's drag: {"type": "enter"} and {"type": "exit", "dropped": bool}.
  void watchWindow(std::function<void(const folly::dynamic&)> tell) { tellWindow_ = std::move(tell); }
  // Who is told a drag started from the press: the press is the drag's from then on, so React
  // Native's touch is cancelled at once and given nothing more of it (PointerInput::cancelResponder),
  // as a Mac's drag session takes the mouse's events from the moment the view begins it. The
  // compositor's cancel of the press, a round trip later, then finds the touch already cancelled:
  // the motions it sent meanwhile reach the drag, not the view that let go of them.
  void takesPress(std::function<void()> take) { takePress_ = std::move(take); }

  // The application drags `data` (Bloom's DragData) from its latest press, allowing `actions`:
  // `count` items, the view `preview` (-1: none) being the one pressed, its image (dragStack: as
  // it looks, stacked and badged for several) under the pointer where it was pressed. `done`
  // receives the action the drop took, or nothing when it was cancelled or refused (no press,
  // nothing to offer). A press released by the time it starts is the compositor's to drop where it
  // was released, as a Mac's drag session takes the mouse-up queued after the drag began. False if it was not started; `done` is then called at once.
  // Asked from JavaScript, it starts once the batch of JavaScript's that asked has run and its
  // commits are mounted (`startAsked`): its image is the view as the press left it (selected), never
  // the tree from before the press, which a press and a motion handed to JavaScript together (on a
  // loaded machine) gave it.
  // `vanish`: dropped on nothing, its image vanishes in a poof where it is (version 26). A drag of
  // files says their types (dragFileTypes).
  // `cancelled`, if given, is told why a drag ended without a drop before `done` is ("escape",
  // "not-accepted", "refused" or "cancelled": Bloom's onCancel).
  bool start(const folly::dynamic& data, const std::vector<std::string>& actions, facebook::react::Tag preview,
             int count, bool vanish, std::function<void(std::optional<std::string>)> done,
             std::function<void(const std::string&)> cancelled = nullptr);
  // The same with the media types offered and their content, the actions allowed (OXYOS_DRAG_*)
  // and the drag's image, if any: a text input's selected text (TextEditing).
  bool start(const std::vector<std::pair<std::string, std::string>>& contents, uint8_t allowed,
             std::optional<DragImage> image, std::function<void(std::optional<std::string>)> done,
             bool vanish = false, const std::string& fileTypes = {},
             std::function<void(const std::string&)> cancelled = nullptr, const std::string& volumes = {});
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
    // The media types of the files it carries (version 26), as many as DRAG_ENTER said, and the
    // one being told in chunks.
    size_t fileExpected = 0;
    std::vector<std::string> fileTypes;
    std::string fileTypeChunks;
    // The volumes its files are on (version 28), as many as DRAG_ENTER said, and the one being told
    // in chunks.
    size_t volumesExpected = 0;
    std::vector<std::string> volumes;
    std::string volumeChunks;
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
  // told of a target entered, moved over (`moved`: the pointer moved, not the tree) or left.
  void update(bool moved = true);
  // The volume files dropped into `folder` go to (oxyos_files_folder_volume), once a drag.
  std::optional<std::string> folderVolume(const std::string& folder);
  // The window's drag: told JavaScript as it begins and ends.
  void tellWindow(folly::dynamic event);
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
  // The latest pointer press, held or released since (none once the compositor cancelled it): its
  // serial, its surface and where it was (root coordinates). A drag asked after its release (a
  // loaded machine reads a press, its motions and its release in one turn) is the compositor's to
  // drop where the press was released (docs/adr/0035-drag-and-drop.mdx, decision 2).
  uint64_t pressSerial_ = 0;
  uint32_t pressSurface_ = 0;
  facebook::react::Point pressPoint_{};
  // This client's drags going on or dropped, by number: what each one's caller is told; and the
  // one going on, not yet dropped (0: none), the drag over this client's surfaces meanwhile.
  std::map<uint32_t, std::function<void(std::optional<std::string>)>> drags_;
  // Of those, the ones whose caller is told why one ended without a drop.
  std::map<uint32_t, std::function<void(const std::string&)>> cancelled_;
  uint32_t live_ = 0;
  // The drag's icon surface and how it is drawn, once configured.
  uint32_t icon_ = 0;
  std::function<void(const Display&, Canvas&)> iconDraw_;
  uint64_t iconDrawnFor_ = 0;
  TextEditing* editing_ = nullptr;
  // The drop targets' folders' volumes, learnt once a drag (a folder's volume changes only as a
  // file system is mounted over it).
  std::map<std::string, std::optional<std::string>> folderVolumes_;
  // The window's drag, as JavaScript was told it: outside; over a surface of this client; dropped on
  // one and still being received; delivered (its onDrop queued), told at JavaScript's next run.
  enum class Presence { Outside, Over, Dropped, Delivered };
  Presence presence_ = Presence::Outside;
  std::function<void(const folly::dynamic&)> tellWindow_;
  std::function<void(const std::string& uri)> giveFiles_;
  // A drag of JavaScript's, started from the tree as it is (`start`'s work).
  bool startFrom(const folly::dynamic& data, const std::vector<std::string>& actions, facebook::react::Tag preview,
                 int count, bool vanish, std::function<void(std::optional<std::string>)> done,
                 std::function<void(const std::string&)> cancelled);
  // Drags JavaScript asked for, started once what it committed is mounted.
  std::vector<std::function<void()>> asked_;
  std::function<void()> takePress_;
};

}  // namespace oxyos
