//! Backend-independent compositor policy and presentation state. Nothing here touches a GPU,
//! an output device or a socket; backends and transports drive it and must establish GPU
//! readiness and output completion themselves.

pub mod placement;
pub mod presentation;
pub mod scene;

pub use presentation::{Error, Extent, Frame, Presentation};
pub use scene::{
    AccessibleNode, ClientId, Grants, Output, OutputId, RenderItem, Role, Scene, Stats, SurfaceKey,
};
