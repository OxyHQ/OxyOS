//! Backend-independent compositor policy and presentation state. Nothing here touches a GPU,
//! an output device or a socket; backends and transports drive it and must establish GPU
//! readiness and output completion themselves.

pub mod placement;
pub mod presentation;
pub mod refusal;
pub mod scene;

pub use presentation::{Error, Extent, Frame, Presentation};
pub use refusal::{BufferRefusal, Refusal};
pub use scene::physical;
pub use scene::{
    AccessibleNode, Appearance, AppearanceChange, AppearanceKind, BluetoothStatus, CaptureRequest,
    ChooserAnswer, ClientId, ControlLevel, ControlRequest, DisplayChange, DisplayInfo,
    DisplayRequest, DrawingPreferences, FileRequest, Grants, HardwareKey, LaunchRequest, ModeInfo,
    NetworkStatus, Output, OutputId, PowerStatus, PreferenceChange, RenderItem, Role, Scene, Stats,
    SurfaceDamage, SurfaceKey, TextInputState, TextTransaction, TrashRequest, WindowLook,
    DRAG_VERSION, MAX_OPEN_DROPS, MAX_PENDING_KEYS, MAX_PREEDIT_BYTES, MAX_PREEDIT_SEGMENTS,
    MAX_WORKSPACES, REPEAT_DELAY, REPEAT_INTERVAL, SHEET_TOP,
};
