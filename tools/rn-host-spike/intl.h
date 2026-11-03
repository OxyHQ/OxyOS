// ECMA-402's Intl, as Hermes provides it on Android from the platform: OxyOS's Hermes has none
// (upstream deleted its ICU backend), so the host gives the runtime one before the bundle runs.
// The formatting is crates/oxyos-intl's, on ICU4X, through the C ABI; intl.js builds Intl's
// DateTimeFormat, NumberFormat and PluralRules and the toLocale*String methods on it, reading
// options as ECMA-402 does. Its default locale is the C ABI's at each call: the session's once the
// window is told it (docs/adr/0040-language-and-region.mdx), so it follows a change.
#pragma once

#include <jsi/jsi.h>

namespace oxyos {

// Installs `__oxyosIntl` (the C ABI's functions, a refusal thrown as a RangeError) and runs
// intl.js, which defines `Intl`.
void installIntl(facebook::jsi::Runtime& runtime);

}  // namespace oxyos
