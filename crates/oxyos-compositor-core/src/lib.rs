//! Backend-independent compositor policy and presentation state. Nothing here touches a GPU,
//! an output device or a socket; backends and transports drive it and must establish GPU
//! readiness and output completion themselves.

pub mod placement;
pub mod presentation;
pub mod scene;

pub use presentation::{Error, Extent, Frame, Presentation};
pub use scene::physical;
pub use scene::{
    AccessibleNode, Appearance, AppearanceChange, AppearanceKind, BluetoothStatus, CaptureRequest,
    ClientId, ControlLevel, ControlRequest, DisplayChange, DisplayInfo, DisplayRequest,
    DrawingPreferences, Grants, HardwareKey, LaunchRequest, ModeInfo, NetworkStatus, Output,
    OutputId, PowerStatus, PreferenceChange, RenderItem, Role, Scene, Stats, SurfaceDamage,
    SurfaceKey, TextInputState, TextTransaction, WindowLook, MAX_PENDING_KEYS, MAX_PREEDIT_BYTES,
    MAX_PREEDIT_SEGMENTS, MAX_WORKSPACES,
};
