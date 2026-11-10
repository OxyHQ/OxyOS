//! A session's [`Audio`]: one connection to PipeWire on a thread of its own ([`PipeWire::follow`],
//! on that thread's PipeWire loop), which follows the graph as PipeWire tells it (the `Audio/Sink`
//! and `Audio/Source` nodes, their volumes, their devices' routes, WirePlumber's `default`
//! metadata) and never looks at it on a clock: a session doing nothing wakes for nothing
//! (docs/performance.mdx, *Idle*). Requests reach the loop through PipeWire's own channel.
//!
//! - The default is chosen as `wpctl set-default` chooses it: `default.configured.audio.sink`
//!   (or `.source`) set in the `default` metadata to `{"name": node}`. WirePlumber makes it the
//!   default (`default.audio.sink`), remembers it with the ones chosen before it (so a device
//!   chosen once is chosen again when it comes back, and the one before when it goes), and moves
//!   every stream that follows the default to it.
//! - A volume, mute or balance goes where WirePlumber's mixer and `pipewire-pulse` put it: the
//!   device's `Route` with its `props`, saved (so it is the route's, kept by WirePlumber per
//!   device and port), or the node's own `Props` for a node with no device. A volume keeps the
//!   balance: each channel's is set, never one for all, as `wpctl set-volume` would.
//! - The chosen input is metered only while asked ([`Audio::meter`]): a capture stream that
//!   follows the default source, its peaks told as [`Event::InputLevel`].

use crate::graph::{Availability, DeviceObject, Graph, Props, Route, Target, Volumes};
use crate::{meter_level, Audio, Direction, Event, State};
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::pod::{Object, Pod, Property, Value, ValueArray};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// PipeWire's socket: what to wait to be made when PipeWire is not there (`PIPEWIRE_REMOTE` in
/// `PIPEWIRE_RUNTIME_DIR`, else `XDG_RUNTIME_DIR`; `pipewire-0` by default).
pub fn pipewire_socket() -> Option<PathBuf> {
    let remote = std::env::var("PIPEWIRE_REMOTE").unwrap_or_else(|_| "pipewire-0".into());
    if Path::new(&remote).is_absolute() {
        return Some(PathBuf::from(remote));
    }
    let dir =
        std::env::var_os("PIPEWIRE_RUNTIME_DIR").or_else(|| std::env::var_os("XDG_RUNTIME_DIR"))?;
    Some(PathBuf::from(dir).join(remote))
}

/// The quantum the meter asks for: 2048 frames at 48 kHz, so it reads the input about 23 times a
/// second, as often as a level meter needs, and no more.
const METER_LATENCY: &str = "2048/48000";

enum Command {
    Default(Direction, String),
    Volume(Direction, f64),
    Muted(Direction, bool),
    Balance(f64),
    Meter(bool),
    /// The graph changed: its state is told (one for a burst of changes).
    Publish,
}

/// What the session's threads share with the loop's.
struct Shared {
    state: Mutex<State>,
    events: Mutex<Vec<Event>>,
    /// Written when events wait; [`Audio::wake_fd`] is the other end.
    wake: UnixStream,
    woken: UnixStream,
    commands: Mutex<Option<pw::channel::Sender<Command>>>,
    metering: AtomicBool,
    generation: AtomicU64,
}

impl Shared {
    fn tell(&self, event: Event) {
        if let Event::State(state) = &event {
            *self.state.lock().expect("the sound's state") = state.clone();
        }
        self.events.lock().expect("the sound's events").push(event);
        let _ = (&self.wake).write(&[1]);
    }

    fn send(&self, command: Command) {
        if let Some(sender) = self.commands.lock().expect("the sound's commands").as_ref() {
            let _ = sender.send(command);
        }
    }
}

/// The session's sound (the module's doc). Clones share it.
#[derive(Clone)]
pub struct PipeWire {
    shared: Arc<Shared>,
}

impl PipeWire {
    /// Not yet following PipeWire: unavailable until [`Self::follow`] connects.
    pub fn new() -> io::Result<Self> {
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        Ok(Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State::unavailable()),
                events: Mutex::new(Vec::new()),
                wake,
                woken,
                commands: Mutex::new(None),
                metering: AtomicBool::new(false),
                generation: AtomicU64::new(0),
            }),
        })
    }

    /// Follows PipeWire on this thread until `stop` is readable (`Ok`) or PipeWire goes or
    /// cannot be reached (the error): the state is told as it changes, and unavailable once it
    /// ends. The caller follows it again once PipeWire's socket is made anew.
    pub fn follow(&self, stop: BorrowedFd<'_>) -> io::Result<()> {
        let generation = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let result = run(&self.shared, generation, stop);
        *self.shared.commands.lock().expect("the sound's commands") = None;
        let unavailable = State {
            generation,
            ..State::unavailable()
        };
        if *self.shared.state.lock().expect("the sound's state") != unavailable {
            self.shared.tell(Event::State(unavailable));
        }
        result
    }
}

impl Audio for PipeWire {
    fn state(&self) -> State {
        self.shared.state.lock().expect("the sound's state").clone()
    }

    fn set_default(&self, direction: Direction, id: &str) {
        // Said chosen at once, as WirePlumber will make it.
        {
            let mut state = self.shared.state.lock().expect("the sound's state");
            let side = state.side_mut(direction);
            if side.devices.iter().any(|device| device.id == id) {
                for device in &mut side.devices {
                    device.selected = device.id == id;
                }
            }
        }
        self.shared
            .send(Command::Default(direction, id.to_string()));
    }

    fn set_volume(&self, direction: Direction, volume: f64) {
        let volume = volume.clamp(0.0, 1.0);
        // Said at once, as it will be: a level read right after it is set is the one set.
        {
            let mut state = self.shared.state.lock().expect("the sound's state");
            let side = state.side_mut(direction);
            if side.volume.is_some() {
                side.volume = Some(volume);
            }
        }
        self.shared.send(Command::Volume(direction, volume));
    }

    fn set_muted(&self, direction: Direction, muted: bool) {
        {
            let mut state = self.shared.state.lock().expect("the sound's state");
            let side = state.side_mut(direction);
            if side.volume.is_some() {
                side.muted = muted;
            }
        }
        self.shared.send(Command::Muted(direction, muted));
    }

    fn set_balance(&self, balance: f64) {
        self.shared.send(Command::Balance(balance.clamp(-1.0, 1.0)));
    }

    fn meter(&self, on: bool) {
        if self.shared.metering.swap(on, Ordering::SeqCst) != on {
            self.shared.send(Command::Meter(on));
        }
    }

    fn take_events(&self) -> Vec<Event> {
        let mut drained = [0u8; 64];
        while matches!((&self.shared.woken).read(&mut drained), Ok(n) if n > 0) {}
        std::mem::take(&mut *self.shared.events.lock().expect("the sound's events"))
    }

    fn wake_fd(&self) -> Option<BorrowedFd<'_>> {
        Some(self.shared.woken.as_fd())
    }
}

/// A proxy bound and its listener, kept while its global is there.
enum Bound {
    Node {
        proxy: pw::node::Node,
        _listener: pw::node::NodeListener,
    },
    /// An application's output stream, followed for its process and whether it plays.
    Stream {
        _proxy: pw::node::Node,
        _listener: pw::node::NodeListener,
    },
    Device {
        proxy: pw::device::Device,
        _listener: pw::device::DeviceListener,
    },
    Metadata {
        proxy: pw::metadata::Metadata,
        _listener: pw::metadata::MetadataListener,
    },
}

/// The input metered: its stream and listener.
struct Meter {
    stream: pw::stream::StreamRc,
    _listener: pw::stream::StreamListener<f64>,
}

/// What the loop keeps.
struct Inner {
    graph: Graph,
    bound: HashMap<u32, Bound>,
    /// The `default` metadata's global id.
    metadata: Option<u32>,
    /// The graph was read whole once (two round trips after the registry's globals): what is
    /// told before would be a part of it.
    synced: bool,
    publishing: bool,
    /// Each output's balance (by node name) while it had a volume: a volume brought up from
    /// nothing gets it back.
    balances: HashMap<String, f64>,
    meter: Option<Meter>,
}

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

fn run(shared: &Arc<Shared>, generation: u64, stop: BorrowedFd<'_>) -> io::Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(failed)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(failed)?;
    let core = context
        .connect_rc(Some(properties! {
            *pw::keys::APP_NAME => "OxyOS",
        }))
        .map_err(failed)?;
    let registry = core.get_registry_rc().map_err(failed)?;
    let gone: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let inner = Rc::new(RefCell::new(Inner {
        graph: Graph::default(),
        bound: HashMap::new(),
        metadata: None,
        synced: false,
        publishing: false,
        balances: HashMap::new(),
        meter: None,
    }));
    let (sender, receiver) = pw::channel::channel::<Command>();
    *shared.commands.lock().expect("the sound's commands") = Some(sender.clone());
    // Changes are told once the loop has read what came with them.
    let publish = {
        let (inner, sender) = (inner.clone(), sender.clone());
        Rc::new(move || {
            let mut inner = inner.borrow_mut();
            if inner.synced && !inner.publishing {
                inner.publishing = true;
                let _ = sender.send(Command::Publish);
            }
        })
    };
    // Two round trips: the first after the registry's globals, the second after what binding
    // them asked for (each object's info and parameters).
    let first = core.sync(0).map_err(failed)?;
    let _core_listener = core
        .add_listener_local()
        .done({
            let (core, inner, publish) = (core.clone(), inner.clone(), publish.clone());
            let second: Rc<RefCell<Option<spa::utils::result::AsyncSeq>>> =
                Rc::new(RefCell::new(None));
            move |id, seq| {
                if id != pw::core::PW_ID_CORE {
                    return;
                }
                if seq == first {
                    *second.borrow_mut() = core.sync(0).ok();
                } else if Some(seq) == *second.borrow() {
                    inner.borrow_mut().synced = true;
                    publish();
                }
            }
        })
        .error({
            let (gone, mainloop) = (gone.clone(), mainloop.clone());
            move |id, _seq, _res, message| {
                if id == pw::core::PW_ID_CORE {
                    *gone.borrow_mut() = Some(message.to_string());
                    mainloop.quit();
                }
            }
        })
        .register();
    let _stop = mainloop.loop_().add_io(
        stop.try_clone_to_owned()?,
        spa::support::system::IoFlags::IN,
        {
            let mainloop = mainloop.clone();
            move |_: &mut OwnedFd| mainloop.quit()
        },
    );
    let _registry_listener = registry
        .add_listener_local()
        .global({
            let (registry, inner, publish) = (registry.downgrade(), inner.clone(), publish.clone());
            move |global| {
                let Some(registry) = registry.upgrade() else {
                    return;
                };
                let props = global.props;
                let class = props.and_then(|props| props.get("media.class"));
                let id = global.id;
                let bound = match global.type_ {
                    pw::types::ObjectType::Metadata
                        if props.and_then(|props| props.get("metadata.name"))
                            == Some("default") =>
                    {
                        let Ok(metadata) = registry.bind::<pw::metadata::Metadata, _>(global)
                        else {
                            return;
                        };
                        let listener = metadata
                            .add_listener_local()
                            .property({
                                let (inner, publish) = (inner.clone(), publish.clone());
                                move |subject, key, _type, value| {
                                    if subject == 0 {
                                        let direction = match key {
                                            Some("default.audio.sink") => Some(Direction::Output),
                                            Some("default.audio.source") => Some(Direction::Input),
                                            _ => None,
                                        };
                                        if let Some(direction) = direction {
                                            let mut inner = inner.borrow_mut();
                                            match value.and_then(name_in) {
                                                Some(name) => {
                                                    inner.graph.defaults.insert(direction, name)
                                                }
                                                None => inner.graph.defaults.remove(&direction),
                                            };
                                            drop(inner);
                                            publish();
                                        }
                                    }
                                    0
                                }
                            })
                            .register();
                        inner.borrow_mut().metadata = Some(id);
                        Bound::Metadata {
                            proxy: metadata,
                            _listener: listener,
                        }
                    }
                    pw::types::ObjectType::Node
                        if matches!(class, Some("Audio/Sink" | "Audio/Source")) =>
                    {
                        let Ok(node) = registry.bind::<pw::node::Node, _>(global) else {
                            return;
                        };
                        node.subscribe_params(&[spa::param::ParamType::Props]);
                        let listener = node
                            .add_listener_local()
                            .info({
                                let (inner, publish) = (inner.clone(), publish.clone());
                                move |info| {
                                    // Its properties come with an info only when they changed.
                                    let changed = info
                                        .change_mask()
                                        .contains(pw::node::NodeChangeMask::PROPS);
                                    if let Some(props) = info.props().filter(|_| changed) {
                                        let mut inner = inner.borrow_mut();
                                        inner.graph.nodes.entry(id).or_default().props =
                                            props_of(props);
                                        drop(inner);
                                        publish();
                                    }
                                }
                            })
                            .param({
                                let (inner, publish) = (inner.clone(), publish.clone());
                                move |_, kind, _, _, param| {
                                    if kind != spa::param::ParamType::Props {
                                        return;
                                    }
                                    let Some(volumes) = param.and_then(volumes_of) else {
                                        return;
                                    };
                                    let mut inner = inner.borrow_mut();
                                    let node = inner.graph.nodes.entry(id).or_default();
                                    node.volumes = Some(volumes);
                                    drop(inner);
                                    publish();
                                }
                            })
                            .register();
                        inner.borrow_mut().graph.nodes.entry(id).or_default();
                        Bound::Node {
                            proxy: node,
                            _listener: listener,
                        }
                    }
                    pw::types::ObjectType::Node if class == Some("Stream/Output/Audio") => {
                        let Ok(node) = registry.bind::<pw::node::Node, _>(global) else {
                            return;
                        };
                        let listener = node
                            .add_listener_local()
                            .info({
                                let (inner, publish) = (inner.clone(), publish.clone());
                                move |info| {
                                    let mut inner = inner.borrow_mut();
                                    let stream = inner.graph.streams.entry(id).or_default();
                                    let before = stream.clone();
                                    if let Some(pid) = info
                                        .props()
                                        .and_then(|props| props.get("application.process.id"))
                                        .and_then(|pid| pid.parse().ok())
                                    {
                                        stream.pid = Some(pid);
                                    }
                                    if info.change_mask().contains(pw::node::NodeChangeMask::STATE)
                                    {
                                        stream.running =
                                            matches!(info.state(), pw::node::NodeState::Running);
                                    }
                                    let changed = *stream != before;
                                    drop(inner);
                                    if changed {
                                        publish();
                                    }
                                }
                            })
                            .register();
                        inner.borrow_mut().graph.streams.entry(id).or_default();
                        Bound::Stream {
                            _proxy: node,
                            _listener: listener,
                        }
                    }
                    pw::types::ObjectType::Device if class == Some("Audio/Device") => {
                        let Ok(device) = registry.bind::<pw::device::Device, _>(global) else {
                            return;
                        };
                        device.subscribe_params(&[spa::param::ParamType::Route]);
                        let listener = device
                            .add_listener_local()
                            .info({
                                let (inner, publish) = (inner.clone(), publish.clone());
                                move |info| {
                                    let changed = info
                                        .change_mask()
                                        .contains(pw::device::DeviceChangeMask::PROPS);
                                    if let Some(props) = info.props().filter(|_| changed) {
                                        let mut inner = inner.borrow_mut();
                                        inner.graph.devices.entry(id).or_default().props =
                                            props_of(props);
                                        drop(inner);
                                        publish();
                                    }
                                }
                            })
                            .param({
                                let (inner, publish) = (inner.clone(), publish.clone());
                                move |_, kind, _, _, param| {
                                    if kind != spa::param::ParamType::Route {
                                        return;
                                    }
                                    let Some(route) = param.and_then(route_of) else {
                                        return;
                                    };
                                    let mut inner = inner.borrow_mut();
                                    let device: &mut DeviceObject =
                                        inner.graph.devices.entry(id).or_default();
                                    // A sub-device has one route at a time in each direction.
                                    device.routes.retain(|old| {
                                        !(old.device == route.device
                                            && old.direction == route.direction)
                                    });
                                    device.routes.push(route);
                                    drop(inner);
                                    publish();
                                }
                            })
                            .register();
                        inner.borrow_mut().graph.devices.entry(id).or_default();
                        Bound::Device {
                            proxy: device,
                            _listener: listener,
                        }
                    }
                    _ => return,
                };
                inner.borrow_mut().bound.insert(id, bound);
            }
        })
        .global_remove({
            let (inner, publish) = (inner.clone(), publish.clone());
            move |id| {
                let mut inner = inner.borrow_mut();
                let removed = inner.bound.remove(&id);
                inner.graph.nodes.remove(&id);
                inner.graph.devices.remove(&id);
                inner.graph.streams.remove(&id);
                if inner.metadata == Some(id) {
                    inner.metadata = None;
                    inner.graph.defaults.clear();
                }
                drop(inner);
                drop(removed);
                publish();
            }
        })
        .register();
    let _attached = receiver.attach(mainloop.loop_(), {
        let (inner, shared, core) = (inner.clone(), shared.clone(), core.clone());
        // Never sends to its own channel: PipeWire's channel holds its queue while it calls this.
        move |command| {
            let mut inner = inner.borrow_mut();
            match command {
                Command::Publish => inner.publishing = false,
                Command::Default(direction, name) => choose(&mut inner, direction, &name),
                Command::Volume(direction, volume) => {
                    set_volume(&mut inner, direction, Change::Volume(volume))
                }
                Command::Muted(direction, muted) => {
                    set_volume(&mut inner, direction, Change::Muted(muted))
                }
                Command::Balance(balance) => {
                    set_volume(&mut inner, Direction::Output, Change::Balance(balance))
                }
                Command::Meter(on) => {
                    meter(&mut inner, &shared, &core, on);
                    return;
                }
            }
            if inner.synced {
                tell_state(&mut inner, &shared, generation);
            }
        }
    });
    // Metering asked for before this connection was made.
    if shared.metering.load(Ordering::SeqCst) {
        meter(&mut inner.borrow_mut(), shared, &core, true);
    }
    mainloop.run();
    // The stream and the proxies before the connection.
    let mut inner = inner.borrow_mut();
    inner.meter = None;
    inner.bound.clear();
    drop(inner);
    let gone = gone.borrow_mut().take();
    match gone {
        Some(message) => Err(failed(format!("PipeWire went away: {message}"))),
        None => Ok(()),
    }
}

/// Tells the state if it changed.
fn tell_state(inner: &mut Inner, shared: &Shared, generation: u64) {
    let state = inner.graph.state(generation);
    remember_balances(inner);
    if *shared.state.lock().expect("the sound's state") != state {
        shared.tell(Event::State(state));
    }
}

/// The node name in a `default` metadata value: `{"name": "alsa_output…"}`.
fn name_in(value: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(value).ok()?;
    value.get("name")?.as_str().map(str::to_string)
}

fn props_of(dict: &spa::utils::dict::DictRef) -> Props {
    dict.iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// One property of an object pod, as a value.
fn value_of(pod: &Pod) -> Option<Value> {
    spa::pod::deserialize::PodDeserializer::deserialize_any_from(pod.as_bytes())
        .ok()
        .map(|(_, value)| value)
}

/// A string pod's text ("" for none).
fn text_of(pod: &Pod) -> String {
    pod.get_string_raw()
        .ok()
        .flatten()
        .map(|text| text.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A node's `Props`: its channels' volumes, their positions and its mute; `None` without
/// channels' volumes.
pub(crate) fn volumes_of(param: &Pod) -> Option<Volumes> {
    let object = param.as_object().ok()?;
    let mut volumes = Volumes::default();
    let mut found = false;
    for prop in object.props() {
        match prop.key().0 {
            spa::sys::SPA_PROP_channelVolumes => {
                if let Some(Value::ValueArray(ValueArray::Float(channels))) = value_of(prop.value())
                {
                    volumes.channels = channels;
                    found = true;
                }
            }
            spa::sys::SPA_PROP_channelMap => {
                if let Some(Value::ValueArray(ValueArray::Id(positions))) = value_of(prop.value()) {
                    volumes.positions = positions.into_iter().map(|id| id.0).collect();
                }
            }
            spa::sys::SPA_PROP_mute => {
                if let Ok(mute) = prop.value().get_bool() {
                    volumes.mute = mute;
                }
            }
            _ => {}
        }
    }
    found.then_some(volumes)
}

/// A device's current `Route`.
pub(crate) fn route_of(param: &Pod) -> Option<Route> {
    let object = param.as_object().ok()?;
    let mut route = Route::default();
    for prop in object.props() {
        let value = prop.value();
        match prop.key().0 {
            spa::sys::SPA_PARAM_ROUTE_index => route.index = value.get_int().ok()?,
            spa::sys::SPA_PARAM_ROUTE_device => route.device = value.get_int().ok()?,
            spa::sys::SPA_PARAM_ROUTE_direction => {
                route.direction = match value.get_id().ok()?.0 {
                    spa::sys::SPA_DIRECTION_OUTPUT => Some(Direction::Output),
                    spa::sys::SPA_DIRECTION_INPUT => Some(Direction::Input),
                    _ => None,
                }
            }
            spa::sys::SPA_PARAM_ROUTE_name => route.name = text_of(value),
            spa::sys::SPA_PARAM_ROUTE_description => route.description = text_of(value),
            spa::sys::SPA_PARAM_ROUTE_available => {
                route.available = match value.get_id().ok()?.0 {
                    spa::sys::SPA_PARAM_AVAILABILITY_no => Availability::No,
                    spa::sys::SPA_PARAM_AVAILABILITY_yes => Availability::Yes,
                    _ => Availability::Unknown,
                }
            }
            spa::sys::SPA_PARAM_ROUTE_info => {
                // A struct: how many pairs, then each key and value.
                if let Some(Value::Struct(fields)) = value_of(value) {
                    let strings: Vec<String> = fields
                        .into_iter()
                        .filter_map(|field| match field {
                            Value::String(text) => Some(text),
                            _ => None,
                        })
                        .collect();
                    route.info = strings
                        .chunks_exact(2)
                        .map(|pair| (pair[0].clone(), pair[1].clone()))
                        .collect();
                }
            }
            _ => {}
        }
    }
    Some(route)
}

/// A pod's bytes.
fn pod_bytes(value: Value) -> Option<Vec<u8>> {
    spa::pod::serialize::PodSerializer::serialize(io::Cursor::new(Vec::new()), &value)
        .ok()
        .map(|(cursor, _)| cursor.into_inner())
}

/// The `Props` object setting channels' volumes and a mute, each where given.
pub(crate) fn props_value(channels: Option<Vec<f32>>, mute: Option<bool>) -> Value {
    let mut properties = Vec::new();
    if let Some(channels) = channels {
        properties.push(Property::new(
            spa::sys::SPA_PROP_channelVolumes,
            Value::ValueArray(ValueArray::Float(channels)),
        ));
    }
    if let Some(mute) = mute {
        properties.push(Property::new(spa::sys::SPA_PROP_mute, Value::Bool(mute)));
    }
    Value::Object(Object {
        type_: spa::sys::SPA_TYPE_OBJECT_Props,
        id: spa::sys::SPA_PARAM_Props,
        properties,
    })
}

/// The `Route` setting a route's `props`, saved (WirePlumber keeps it for the device's port).
pub(crate) fn route_value(index: i32, device: i32, props: Value) -> Value {
    Value::Object(Object {
        type_: spa::sys::SPA_TYPE_OBJECT_ParamRoute,
        id: spa::sys::SPA_PARAM_Route,
        properties: vec![
            Property::new(spa::sys::SPA_PARAM_ROUTE_index, Value::Int(index)),
            Property::new(spa::sys::SPA_PARAM_ROUTE_device, Value::Int(device)),
            Property::new(spa::sys::SPA_PARAM_ROUTE_props, props),
            Property::new(spa::sys::SPA_PARAM_ROUTE_save, Value::Bool(true)),
        ],
    })
}

/// Chooses the default as `wpctl set-default` does, and says it at once.
fn choose(inner: &mut Inner, direction: Direction, name: &str) {
    let known = inner.graph.node_named(name).is_some_and(|(_, node)| {
        inner
            .graph
            .device(node)
            .is_some_and(|(d, _)| d == direction)
    });
    let Some(Bound::Metadata {
        proxy: metadata, ..
    }) = inner.metadata.and_then(|id| inner.bound.get(&id))
    else {
        return;
    };
    if !known {
        return;
    }
    let value = serde_json::json!({ "name": name }).to_string();
    metadata.set_property(
        0,
        &format!("default.configured.{}", direction.metadata_suffix()),
        Some("Spa:String:JSON"),
        Some(&value),
    );
    inner.graph.defaults.insert(direction, name.to_string());
}

enum Change {
    Volume(f64),
    Muted(bool),
    Balance(f64),
}

fn remember_balances(inner: &mut Inner) {
    let mut remembered = Vec::new();
    for node in inner.graph.nodes.values() {
        let (Some(name), Some(volumes)) = (node.props.get("node.name"), node.volumes.as_ref())
        else {
            continue;
        };
        if let (volume, Some(balance)) = volumes.seen() {
            if volume > 0.0 {
                remembered.push((name.clone(), balance));
            }
        }
    }
    inner.balances.extend(remembered);
}

/// Sets the chosen device's volume, mute or balance where it goes, and holds it as set until
/// PipeWire says what it has.
fn set_volume(inner: &mut Inner, direction: Direction, change: Change) {
    let Some((id, node)) = inner.graph.default_node(direction) else {
        return;
    };
    let Some(volumes) = node.volumes.clone() else {
        return;
    };
    let name = node.props.get("node.name").cloned().unwrap_or_default();
    let (volume, balance) = volumes.seen();
    let balance = match balance {
        // All channels at nothing: the balance it had.
        Some(_) if volume == 0.0 => inner.balances.get(&name).copied().unwrap_or(0.0),
        Some(balance) => balance,
        None => 0.0,
    };
    let (channels, mute) = match change {
        Change::Volume(volume) => (Some(volumes.channels_for(volume, balance)), None),
        Change::Balance(balance) => {
            inner.balances.insert(name, balance);
            (Some(volumes.channels_for(volume, balance)), None)
        }
        Change::Muted(muted) => (None, Some(muted)),
    };
    let Some(target) = inner.graph.volume_target(direction) else {
        return;
    };
    let props = props_value(channels.clone(), mute);
    match target {
        Target::Route {
            device,
            index,
            route_device,
        } => {
            let Some(Bound::Device { proxy, .. }) = inner.bound.get(&device) else {
                return;
            };
            let Some(bytes) = pod_bytes(route_value(index, route_device, props)) else {
                return;
            };
            if let Some(pod) = Pod::from_bytes(&bytes) {
                proxy.set_param(spa::param::ParamType::Route, 0, pod);
            }
        }
        Target::Node(node) => {
            let Some(Bound::Node { proxy, .. }) = inner.bound.get(&node) else {
                return;
            };
            let Some(bytes) = pod_bytes(props) else {
                return;
            };
            if let Some(pod) = Pod::from_bytes(&bytes) {
                proxy.set_param(spa::param::ParamType::Props, 0, pod);
            }
        }
    }
    // Held as set: what PipeWire then says replaces it.
    if let Some(node) = inner.graph.nodes.get_mut(&id) {
        if let Some(held) = node.volumes.as_mut() {
            if let Some(channels) = channels {
                held.channels = channels;
            }
            if let Some(mute) = mute {
                held.mute = mute;
            }
        }
    }
}

/// Starts or stops metering the default input.
fn meter(inner: &mut Inner, shared: &Arc<Shared>, core: &pw::core::CoreRc, on: bool) {
    if !on {
        if let Some(meter) = inner.meter.take() {
            let _ = meter.stream.disconnect();
        }
        return;
    }
    if inner.meter.is_some() {
        return;
    }
    match start_meter(shared, core) {
        Ok(meter) => inner.meter = Some(meter),
        Err(error) => eprintln!("[audio] the input's level: {error}"),
    }
}

fn start_meter(shared: &Arc<Shared>, core: &pw::core::CoreRc) -> Result<Meter, pw::Error> {
    let stream = pw::stream::StreamRc::new(
        core.clone(),
        "oxyos-input-level",
        properties! {
            *pw::keys::NODE_NAME => "oxyos-input-level",
            *pw::keys::NODE_DESCRIPTION => "OxyOS input level",
            *pw::keys::APP_NAME => "OxyOS",
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "DSP",
            *pw::keys::NODE_LATENCY => METER_LATENCY,
            // Never a target of its own: it follows the default input wherever it goes.
            "stream.capture.sink" => "false",
        },
    )?;
    // The last level told, so each step is told once.
    let listener = stream
        .add_local_listener_with_user_data(-1.0f64)
        .process({
            let shared = shared.clone();
            move |stream, told| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let datas = buffer.datas_mut();
                let Some(data) = datas.first_mut() else {
                    return;
                };
                let size = data.chunk().size() as usize;
                let offset = data.chunk().offset() as usize;
                let peak = data
                    .data()
                    .and_then(|bytes| bytes.get(offset..offset + size))
                    .map(|bytes| {
                        bytes
                            .chunks_exact(4)
                            .map(|sample| {
                                f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]])
                                    .abs()
                            })
                            .fold(0.0f32, f32::max)
                    })
                    .unwrap_or(0.0);
                let level = meter_level(peak);
                if level != *told {
                    *told = level;
                    shared.tell(Event::InputLevel(level));
                }
            }
        })
        .register()?;
    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_channels(1);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_MONO;
    info.set_position(position);
    let format = pod_bytes(Value::Object(Object {
        type_: spa::sys::SPA_TYPE_OBJECT_Format,
        id: spa::sys::SPA_PARAM_EnumFormat,
        properties: info.into(),
    }))
    .ok_or(pw::Error::CreationFailed)?;
    let mut params = [Pod::from_bytes(&format).ok_or(pw::Error::CreationFailed)?];
    stream.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;
    Ok(Meter {
        stream,
        _listener: listener,
    })
}
