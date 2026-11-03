//! OxyOS's input-method engine (docs/adr/0041-input-methods.mdx): fcitx5, run headless by the
//! session on a private D-Bus daemon, with the session as its frontend.
//!
//! [`Engine`] is the engine as installed (`tools/input-methods/build.sh`): which of OxyOS's input
//! methods it has. [`Service`] starts it (the bus, then fcitx5 with only the addons it needs and
//! the session's own configuration) and talks to it on a thread of its own: the compositor sends
//! [`Command`]s and takes [`Outcome`]s when its wake descriptor is readable, so it never waits on
//! the engine. One input context serves every field, one at a time; a key's whole effect comes in
//! the reply to `ProcessKeyEventBatch`, which becomes one [`Change`]; what the engine does on its
//! own (a candidate chosen, a page turned) comes as signals, in the order the bus delivered them.
//!
//! Text offsets are as fcitx5 counts them: characters (Unicode scalar values), never bytes.

mod engine;
mod frontend;

pub use engine::{fcitx_name, Engine, Paths, INPUT_METHODS};
pub use frontend::{
    capability, Change, Command, Outcome, Panel, Preedit, PreeditFormat, Service, Sources,
};
