// React Native's Modal (ModalHostView) as its Android window gives it: laid out at the window's
// size from its first rendering, told it is shown (onShow), and asked to close (onRequestClose) by Escape, the desktop's
// back. It is drawn over the window's content and takes its input (renderer.h, input.h). Its
// `animationType` animates its window in and out as Android's do (React Native's
// `catalyst_slide_up`, `_slide_down`, `_fade_in` and `_fade_out`): for 200 ms, Android's
// `config_shortAnimTime`, sliding up from below the window and back down, accelerating then
// decelerating, or fading in and out, accelerating. A modal leaving is drawn from its last frame
// while it animates out, as the window manager animates a dismissed dialog's window.
#pragma once

#include "renderer.h"

#include <react/renderer/components/modal/ModalHostViewShadowNode.h>
#include <react/renderer/core/ConcreteComponentDescriptor.h>

#include <chrono>
#include <map>
#include <memory>
#include <set>

namespace oxyos {

// The size the host lays its window's surface out at, which every modal is laid out at (ModalHost),
// kept in Fabric's context container under kModalScreenKey for the modal's descriptor. JavaScript
// runs on the host's one thread, which alone reads and writes it.
struct ModalScreen {
  facebook::react::Size size;
};
inline constexpr const char* kModalScreenKey = "OxyOSModalScreen";

// React Native's Modal descriptor (ModalHostViewComponentDescriptor, which is final, so its
// `adopt` is repeated here), whose modal starts at the window's size, as Android's
// createInitialState gives it the screen's: its first layout is its last, so what is laid out in
// it (a sheet's rows) is never first laid out at no size and then again, and nothing scrolled into
// view in between. React Native's own state for other platforms starts at 0 × 0 until the platform
// updates it.
class ModalHostViewComponentDescriptor final
    : public facebook::react::ConcreteComponentDescriptor<facebook::react::ModalHostViewShadowNode> {
 public:
  using ConcreteComponentDescriptor::ConcreteComponentDescriptor;

  void adopt(facebook::react::ShadowNode& shadowNode) const override;
  facebook::react::State::Shared createInitialState(
      const facebook::react::Props::Shared& props,
      const facebook::react::ShadowNodeFamily::Shared& family) const override;
};

class ModalHost {
 public:
  using Clock = std::chrono::steady_clock;
  static constexpr std::chrono::milliseconds kAnimation{200};

  explicit ModalHost(ViewTree& tree) : tree_(tree) {}

  // After a mount or a new size: each modal is laid out at the window's size (its state, which
  // Android gives it from the screen; a new modal already is, by its descriptor, above), and each
  // one newly shown is told so and starts animating in.
  void sync(const Display& display);
  // Before a transaction is applied: each animated modal it deletes keeps its last frame, drawn
  // with `edit` at `display`, to animate out.
  void leaving(const facebook::react::ShadowViewMutationList& mutations, const Display& display,
               const TextInputEdit& edit);
  // Moves the animations on to now, presenting each modal in the tree, once for each frame drawn
  // (an animation starts when its modal is shown, and its first frame is drawn from here); true if
  // any was running (the frame that ends one draws the modal still).
  bool tick(const Display& display);
  // Escape pressed in the window: the topmost modal is asked to close; true if one was.
  bool requestClose();

 private:
  enum class Kind { Slide, Fade };
  struct Animation {
    Kind kind;
    Clock::time_point start;
    std::string name;  // the modal's testID, for the report
  };
  struct Leaving {
    Animation animation;
    std::shared_ptr<const Canvas> frame;
  };
  ModalPresentation presentation(const Animation& animation, bool in, float progress, const Display& display) const;

  ViewTree& tree_;
  std::set<facebook::react::Tag> shown_;
  std::map<facebook::react::Tag, Animation> entering_;
  std::vector<Leaving> leaving_;
};

}  // namespace oxyos
