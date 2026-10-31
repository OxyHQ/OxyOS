// Skia on the GPU (docs/adr/0031-gpu-buffers.mdx, decision 12): Ganesh on the Vulkan device
// oxyos-paint opened, the one the compositor shares buffers with.
#pragma once

#include <oxyos_client.h>

class GrDirectContext;

namespace oxyos {

// The Ganesh context on `paint`'s device, made when first asked for; null for none (no painter, or
// a device Skia cannot draw on).
GrDirectContext* skiaContext(OxyOSPaint* paint);

}  // namespace oxyos
