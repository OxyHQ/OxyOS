#include "skia-gpu.h"

namespace oxyos {

// Phase 3 of the migration (Ganesh on oxyos-paint's device) replaces this: until then, every
// canvas is drawn by Skia's raster backend.
GrDirectContext* skiaContext(OxyOSPaint*) { return nullptr; }

}  // namespace oxyos
