//! What PipeWire says of its graph, kept as plain data (no proxies), and the [`State`] a person
//! sees from it: which nodes are outputs and inputs, what each is called and how it is
//! connected, which is chosen, and the chosen one's volume. Where a change goes is decided here
//! too ([`Graph::volume_target`]): a device's route, as WirePlumber and PulseAudio set it, so it
//! is kept for that route (headphones keep their volume apart from speakers), else the node's
//! own properties.

use crate::{
    cubic, levels_for, linear, volume_and_balance, Device, Direction, Kind, Place, Side, State,
    Transport,
};
use std::collections::BTreeMap;

pub type Props = BTreeMap<String, String>;

/// A route's availability, as PipeWire's `SPA_PARAM_AVAILABILITY_*` says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Availability {
    #[default]
    Unknown,
    No,
    Yes,
}

/// One of a device's current routes (its `Route` parameter): what a node of it plays through.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Route {
    pub index: i32,
    /// The device's sub-device it is the route of: a node's `card.profile.device`.
    pub device: i32,
    pub direction: Option<Direction>,
    pub name: String,
    pub description: String,
    pub available: Availability,
    /// Its `info`: `port.type` (`speaker`, `headphones`, `hdmi`, `mic`, `headset`...) and, for a
    /// display's, `device.product.name` (the display's own name, from its EDID).
    pub info: Props,
}

/// A node's volumes (its `Props` parameter): each channel's, linear, where it is, and its mute.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Volumes {
    pub channels: Vec<f32>,
    /// SPA's channel positions (`SPA_AUDIO_CHANNEL_*`), as many as `channels` when known.
    pub positions: Vec<u32>,
    pub mute: bool,
}

impl Volumes {
    pub fn places(&self) -> Vec<Place> {
        (0..self.channels.len().max(self.positions.len()))
            .map(|index| match self.positions.get(index) {
                Some(position) => place_of(*position),
                // No map: two channels are a stereo pair, else none is on a side.
                None if self.channels.len() == 2 => {
                    if index == 0 {
                        Place::Left
                    } else {
                        Place::Right
                    }
                }
                None => Place::Middle,
            })
            .collect()
    }

    /// The volume and balance as the person sees them (the cubic scale).
    pub fn seen(&self) -> (f64, Option<f64>) {
        let levels: Vec<f64> = self.channels.iter().map(|linear| cubic(*linear)).collect();
        volume_and_balance(&levels, &self.places())
    }

    /// Every channel's linear volume for a volume and balance.
    pub fn channels_for(&self, volume: f64, balance: f64) -> Vec<f32> {
        levels_for(volume, balance, &self.places())
            .into_iter()
            .map(linear)
            .collect()
    }
}

/// Where SPA's channel position is.
pub fn place_of(position: u32) -> Place {
    use pipewire::spa::sys as spa;
    const LEFT: &[u32] = &[
        spa::SPA_AUDIO_CHANNEL_FL,
        spa::SPA_AUDIO_CHANNEL_SL,
        spa::SPA_AUDIO_CHANNEL_FLC,
        spa::SPA_AUDIO_CHANNEL_RL,
        spa::SPA_AUDIO_CHANNEL_TFL,
        spa::SPA_AUDIO_CHANNEL_TRL,
        spa::SPA_AUDIO_CHANNEL_RLC,
        spa::SPA_AUDIO_CHANNEL_FLW,
        spa::SPA_AUDIO_CHANNEL_FLH,
        spa::SPA_AUDIO_CHANNEL_TFLC,
        spa::SPA_AUDIO_CHANNEL_TSL,
        spa::SPA_AUDIO_CHANNEL_LLFE,
        spa::SPA_AUDIO_CHANNEL_BLC,
    ];
    const RIGHT: &[u32] = &[
        spa::SPA_AUDIO_CHANNEL_FR,
        spa::SPA_AUDIO_CHANNEL_SR,
        spa::SPA_AUDIO_CHANNEL_FRC,
        spa::SPA_AUDIO_CHANNEL_RR,
        spa::SPA_AUDIO_CHANNEL_TFR,
        spa::SPA_AUDIO_CHANNEL_TRR,
        spa::SPA_AUDIO_CHANNEL_RRC,
        spa::SPA_AUDIO_CHANNEL_FRW,
        spa::SPA_AUDIO_CHANNEL_FRH,
        spa::SPA_AUDIO_CHANNEL_TFRC,
        spa::SPA_AUDIO_CHANNEL_TSR,
        spa::SPA_AUDIO_CHANNEL_RLFE,
        spa::SPA_AUDIO_CHANNEL_BRC,
    ];
    if LEFT.contains(&position) {
        Place::Left
    } else if RIGHT.contains(&position) {
        Place::Right
    } else {
        Place::Middle
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub props: Props,
    pub volumes: Option<Volumes>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceObject {
    pub props: Props,
    pub routes: Vec<Route>,
}

/// The graph as PipeWire last said it, by global id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub nodes: BTreeMap<u32, Node>,
    pub devices: BTreeMap<u32, DeviceObject>,
    /// WirePlumber's choices (`default.audio.sink`, `default.audio.source`): node names.
    pub defaults: BTreeMap<Direction, String>,
    /// Applications' output streams (`Stream/Output/Audio`), by global id.
    pub streams: BTreeMap<u32, Stream>,
}

/// An application's output stream: its process, and whether it is running (playing).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stream {
    pub pid: Option<u32>,
    pub running: bool,
}

/// Where a volume, mute or balance change goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// The device's route: its `Route` parameter with these `props`, saved by WirePlumber.
    Route {
        device: u32,
        index: i32,
        route_device: i32,
    },
    /// The node's own `Props`.
    Node(u32),
}

impl Graph {
    fn direction_of(node: &Node) -> Option<Direction> {
        match node.props.get("media.class").map(String::as_str) {
            Some("Audio/Sink") => Some(Direction::Output),
            Some("Audio/Source") => Some(Direction::Input),
            _ => None,
        }
    }

    fn device_of(&self, node: &Node) -> Option<(u32, &DeviceObject)> {
        let id = node.props.get("device.id")?.parse().ok()?;
        self.devices.get(&id).map(|device| (id, device))
    }

    /// The route a node plays through: its device's current route of its sub-device.
    pub fn route_of(&self, node: &Node) -> Option<&Route> {
        let (_, device) = self.device_of(node)?;
        let sub: i32 = node.props.get("card.profile.device")?.parse().ok()?;
        let direction = Self::direction_of(node);
        device.routes.iter().find(|route| {
            route.device == sub && (route.direction.is_none() || route.direction == direction)
        })
    }

    /// The node of a name.
    pub fn node_named(&self, name: &str) -> Option<(u32, &Node)> {
        self.nodes
            .iter()
            .find(|(_, node)| node.props.get("node.name").map(String::as_str) == Some(name))
            .map(|(id, node)| (*id, node))
    }

    /// The node WirePlumber chose for a direction.
    pub fn default_node(&self, direction: Direction) -> Option<(u32, &Node)> {
        let name = self.defaults.get(&direction)?;
        self.node_named(name)
            .filter(|(_, node)| Self::direction_of(node) == Some(direction))
    }

    /// Where a change of the chosen device's volume goes (as WirePlumber's mixer and
    /// `pipewire-pulse` decide it): its device's route when it has one, else the node.
    pub fn volume_target(&self, direction: Direction) -> Option<Target> {
        let (id, node) = self.default_node(direction)?;
        if let (Some((device, _)), Some(route)) = (self.device_of(node), self.route_of(node)) {
            return Some(Target::Route {
                device,
                index: route.index,
                route_device: route.device,
            });
        }
        Some(Target::Node(id))
    }

    /// One node as a person sees it; `None` for what is not an output or input, or a route
    /// with nothing plugged into it (a display's port with no display).
    pub fn device(&self, node: &Node) -> Option<(Direction, Device)> {
        let direction = Self::direction_of(node)?;
        let id = node.props.get("node.name")?.clone();
        let route = self.route_of(node);
        if route.is_some_and(|route| route.available == Availability::No) {
            return None;
        }
        let device = self.device_of(node).map(|(_, device)| &device.props);
        fn get<'a>(props: Option<&'a Props>, key: &str) -> Option<&'a str> {
            props
                .and_then(|props| props.get(key))
                .map(String::as_str)
                .filter(|value| !value.is_empty())
        }
        let port_type = route.and_then(|route| get(Some(&route.info), "port.type"));
        let route_name = route
            .map(|route| route.name.to_ascii_lowercase())
            .unwrap_or_default();
        // The device's, else the node's own (a node with no device object says them itself).
        let either = |key: &str| get(device, key).or_else(|| get(Some(&node.props), key));
        let transport = if either("device.api") == Some("bluez5")
            || either("device.bus") == Some("bluetooth")
        {
            Transport::Bluetooth
        } else if either("device.bus") == Some("usb") {
            Transport::Usb
        } else if port_type == Some("hdmi") || route_name.contains("hdmi") {
            Transport::Hdmi
        } else if device.is_some() {
            Transport::BuiltIn
        } else if get(Some(&node.props), "node.virtual") == Some("true")
            || get(Some(&node.props), "device.id").is_none()
        {
            Transport::Virtual
        } else {
            Transport::Other
        };
        let form = either("device.form-factor");
        let kind = match direction {
            Direction::Output => match (port_type, form) {
                (Some("headphones"), _) => Kind::Headphones,
                (Some("headset" | "handset"), _) => Kind::Headset,
                (Some("speaker"), _) => Kind::Speakers,
                (Some("hdmi"), _) => Kind::Hdmi,
                (Some("line"), _) => Kind::Line,
                _ if route_name.contains("headphone") => Kind::Headphones,
                _ if route_name.contains("headset") => Kind::Headset,
                _ if route_name.contains("speaker") => Kind::Speakers,
                _ if route_name.contains("hdmi") => Kind::Hdmi,
                _ if route_name.contains("line") => Kind::Line,
                (_, Some("headphone")) => Kind::Headphones,
                (_, Some("headset" | "hands-free" | "handset")) => Kind::Headset,
                (_, Some("speaker" | "internal" | "hifi" | "portable" | "car" | "computer")) => {
                    Kind::Speakers
                }
                _ if transport == Transport::Hdmi => Kind::Hdmi,
                _ if transport == Transport::BuiltIn => Kind::Speakers,
                _ => Kind::Other,
            },
            Direction::Input => match (port_type, form) {
                (Some("mic"), _) => Kind::Microphone,
                (Some("headset" | "handset"), _) => Kind::Headset,
                (Some("line"), _) => Kind::Line,
                _ if route_name.contains("headset") => Kind::Headset,
                _ if route_name.contains("mic") => Kind::Microphone,
                _ if route_name.contains("line") => Kind::Line,
                (_, Some("headset" | "hands-free" | "handset")) => Kind::Headset,
                (_, Some("webcam" | "microphone" | "internal" | "computer")) => Kind::Microphone,
                _ if transport == Transport::BuiltIn => Kind::Microphone,
                _ => Kind::Other,
            },
        };
        let node_name = get(Some(&node.props), "node.description")
            .or_else(|| get(Some(&node.props), "node.nick"))
            .unwrap_or(id.as_str());
        let name = match transport {
            Transport::Hdmi => route
                .and_then(|route| get(Some(&route.info), "device.product.name"))
                .or_else(|| {
                    route
                        .map(|route| route.description.as_str())
                        .filter(|d| !d.is_empty())
                })
                .unwrap_or(node_name),
            Transport::Bluetooth | Transport::Usb => get(device, "device.description")
                .or_else(|| get(device, "device.product.name"))
                .unwrap_or(node_name),
            Transport::BuiltIn => route
                .map(|route| route.description.as_str())
                .filter(|d| !d.is_empty())
                .unwrap_or(node_name),
            Transport::Virtual | Transport::Other => node_name,
        };
        let name = name.to_string();
        let selected = self.defaults.get(&direction) == Some(&id);
        Some((
            direction,
            Device {
                id,
                name,
                kind,
                transport,
                selected,
            },
        ))
    }

    /// What a person sees, read on connection `generation`.
    pub fn state(&self, generation: u64) -> State {
        let mut state = State {
            available: true,
            generation,
            audible: self
                .streams
                .values()
                .filter(|stream| stream.running)
                .filter_map(|stream| stream.pid)
                .collect(),
            ..State::default()
        };
        for node in self.nodes.values() {
            if let Some((direction, device)) = self.device(node) {
                state.side_mut(direction).devices.push(device);
            }
        }
        for direction in [Direction::Output, Direction::Input] {
            let side: &mut Side = state.side_mut(direction);
            side.devices.sort_by(|a, b| {
                (a.transport, a.name.to_lowercase(), &a.id).cmp(&(
                    b.transport,
                    b.name.to_lowercase(),
                    &b.id,
                ))
            });
            if let Some(volumes) = self
                .default_node(direction)
                .and_then(|(_, node)| node.volumes.as_ref())
            {
                let (volume, balance) = volumes.seen();
                side.volume = Some(volume);
                side.muted = volumes.mute;
                // An input's balance is not a person's to set (a Mac's has none).
                side.balance = balance.filter(|_| direction == Direction::Output);
            }
        }
        state
    }
}
