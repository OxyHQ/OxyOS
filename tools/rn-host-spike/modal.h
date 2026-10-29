// React Native's Modal (ModalHostView) as its Android window gives it: laid out at the window's
// size, told it is shown (onShow), and asked to close (onRequestClose) by Escape, the desktop's
// back. It is drawn over the window's content and takes its input (renderer.h, input.h).
#pragma once

#include "renderer.h"

#include <set>

namespace oxyos {

class ModalHost {
 public:
  explicit ModalHost(const ViewTree& tree) : tree_(tree) {}

  // After a mount or a new size: each modal is laid out at the window's size (its state, which
  // Android gives it from the screen), and each one newly shown is told so.
  void sync(const Display& display);
  // Escape pressed in the window: the topmost modal is asked to close; true if one was.
  bool requestClose();

 private:
  const ViewTree& tree_;
  std::set<facebook::react::Tag> shown_;
};

}  // namespace oxyos
