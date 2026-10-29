// React Native's Modal (ModalHostView) as its Android window gives it: laid out at the window's
// size, told it is shown (onShow), and asked to close (onRequestClose) by Escape, the desktop's
// back. It is drawn over the window's content and takes its input (renderer.h, input.h). Its
// `animationType` animates its window in and out as Android's do (React Native's
// `catalyst_slide_up`, `_slide_down`, `_fade_in` and `_fade_out`): for 200 ms, Android's
// `config_shortAnimTime`, sliding up from below the window and back down, accelerating then
// decelerating, or fading in and out, accelerating. A modal leaving is drawn from its last frame
// while it animates out, as the window manager animates a dismissed dialog's window.
#pragma once

#include "renderer.h"

#include <chrono>
#include <map>
#include <memory>
#include <set>

namespace oxyos {

class ModalHost {
 public:
  using Clock = std::chrono::steady_clock;
  static constexpr std::chrono::milliseconds kAnimation{200};

  explicit ModalHost(ViewTree& tree) : tree_(tree) {}

  // After a mount or a new size: each modal is laid out at the window's size (its state, which
  // Android gives it from the screen), and each one newly shown is told so and starts animating in.
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
