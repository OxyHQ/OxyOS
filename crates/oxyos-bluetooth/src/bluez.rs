//! The session's connection to BlueZ ([`run`]): one connection, its executor run by the calling
//! thread as it waits (no thread of zbus's own, which would keep async-io's reactor waking, as
//! docs/performance.mdx says), waiting on BlueZ's signals, the caller's commands, the agent's
//! questions, the calls under way and the order to stop, and on nothing else.

use crate::agent::{Agent, Heard};
use crate::model::{Interfaces, Objects, ADAPTER, BATTERY, DEVICE};
use crate::{Answer, Ask, Busy, Command, Done, Event, Failure, Question, State, BLUEZ};
use futures_lite::StreamExt;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::os::fd::BorrowedFd;
use std::time::Duration;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// Where the session serves its pairing agent.
pub const AGENT_PATH: &str = "/so/oxy/OxyOS/BluetoothAgent";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const AGENT_MANAGER: &str = "org.bluez.AgentManager1";
/// How long BlueZ is given, as the session ends, to stop discovering and forget the agent.
const PARTING: Duration = Duration::from_secs(1);

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

/// A BlueZ error's name and message.
fn error_of(error: &zbus::Error) -> (String, String) {
    match error {
        zbus::Error::MethodError(name, message, _) => {
            (name.to_string(), message.clone().unwrap_or_default())
        }
        zbus::Error::FDO(error) => {
            let name = zbus::DBusError::name(&**error).to_string();
            (name, error.to_string())
        }
        other => (String::new(), other.to_string()),
    }
}

fn failure(error: &zbus::Error) -> Failure {
    let (name, message) = error_of(error);
    Failure::of(&name, &message)
}

/// Connect found nothing of the device to connect (a phone, a computer): paired is all it is.
fn no_profiles(error: &zbus::Error) -> bool {
    let (name, message) = error_of(error);
    name.ends_with(".NotAvailable") || message.contains("profile-unavailable")
}

/// A method of BlueZ's at `path`, its answer read as `R`.
async fn call<R>(
    bus: &zbus::Connection,
    path: &str,
    interface: &str,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> zbus::Result<R>
where
    R: for<'d> zbus::zvariant::DynamicDeserialize<'d>,
{
    let reply = bus
        .call_method(Some(BLUEZ), path, Some(interface), method, body)
        .await?;
    reply.body().deserialize()
}

/// Sets a property of BlueZ's.
async fn set(
    bus: &zbus::Connection,
    path: &str,
    interface: &str,
    name: &str,
    value: Value<'_>,
) -> zbus::Result<()> {
    bus.call_method(
        Some(BLUEZ),
        path,
        Some(PROPERTIES),
        "Set",
        &(interface, name, value),
    )
    .await
    .map(|_| ())
}

/// A match on BlueZ's signals: of `interface`'s `member`, at `path` if given, and for
/// `PropertiesChanged`, of the properties of `of`.
fn rule(
    path: Option<&'static str>,
    interface: &'static str,
    member: &'static str,
    of: Option<&'static str>,
) -> zbus::Result<zbus::MatchRule<'static>> {
    let mut builder = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(BLUEZ)?
        .interface(interface)?
        .member(member)?;
    if let Some(path) = path {
        builder = builder.path(path)?;
    }
    if let Some(of) = of {
        builder = builder.arg(0, of)?;
    }
    Ok(builder.build())
}

/// `future`, with `executor` ticked as it is waited for.
async fn ticked<T>(
    executor: &zbus::Executor<'static>,
    future: impl std::future::Future<Output = T>,
) -> T {
    futures_lite::future::or(future, async {
        loop {
            executor.tick().await;
        }
    })
    .await
}

/// A call of the session's ended.
struct Finished {
    call: u64,
    address: String,
    path: String,
    outcome: Result<Done, Failure>,
}

enum Next {
    Signal(zbus::Message),
    Command(Command),
    Heard(Heard),
    Finished(Finished),
    Stop,
    Closed,
}

/// The next message of any of `streams`; `None` once one ends (the bus closed).
async fn next_of(streams: &mut [zbus::MessageStream]) -> Option<zbus::Message> {
    futures_lite::future::poll_fn(|cx| {
        for stream in streams.iter_mut() {
            match stream.poll_next(cx) {
                std::task::Poll::Ready(Some(Ok(message))) => {
                    return std::task::Poll::Ready(Some(message))
                }
                std::task::Poll::Ready(Some(Err(_))) => {}
                std::task::Poll::Ready(None) => return std::task::Poll::Ready(None),
                std::task::Poll::Pending => {}
            }
        }
        std::task::Poll::Pending
    })
    .await
}

/// A question standing: its number, its device's object, and the code it shows, if it only shows.
struct Standing {
    id: u64,
    path: String,
    shows: Option<Ask>,
}

/// What the loop knows and holds.
struct Link {
    bus: zbus::Connection,
    executor: zbus::Executor<'static>,
    objects: Objects,
    available: bool,
    busy: BTreeMap<String, Busy>,
    /// Discovery wanted by the caller.
    want: bool,
    /// This connection discovers.
    discovering: bool,
    /// A start that failed for the wish as it is: not tried again until it changes.
    start_failed: bool,
    /// The adapter made discoverable by this connection, and a change asked not yet seen.
    discoverable: bool,
    asking_discoverable: bool,
    registered: bool,
    next_question: u64,
    standing: Vec<Standing>,
    /// Devices whose question the person refused: their pairing failing is a cancel.
    refused: HashSet<String>,
    told: Option<State>,
    finished: async_channel::Sender<Finished>,
}

impl Link {
    fn state(&self) -> State {
        self.objects.state(self.available, &self.busy)
    }

    fn tell(&mut self, emit: &mut dyn FnMut(Event)) {
        let state = self.state();
        if self.told.as_ref() != Some(&state) {
            self.told = Some(state.clone());
            emit(Event::State(state));
        }
    }

    /// Reads BlueZ whole and registers the agent: at start and when BlueZ (re)starts.
    async fn read(&mut self, emit: &mut dyn FnMut(Event)) {
        self.discovering = false;
        self.discoverable = false;
        self.asking_discoverable = false;
        self.start_failed = false;
        self.registered = false;
        let read: zbus::Result<HashMap<OwnedObjectPath, Interfaces>> =
            call(&self.bus, "/", OBJECT_MANAGER, "GetManagedObjects", &()).await;
        match read {
            Ok(objects) => {
                self.objects = Objects::read(objects);
                self.available = true;
            }
            Err(error) => {
                if failure(&error) != Failure::Unavailable {
                    eprintln!("[bluetooth] reading BlueZ: {error}");
                }
                self.objects = Objects::default();
                self.available = false;
                self.end_questions(emit);
                return;
            }
        }
        let path = zbus::zvariant::ObjectPath::from_static_str_unchecked(AGENT_PATH);
        let registered: zbus::Result<()> = call(
            &self.bus,
            "/org/bluez",
            AGENT_MANAGER,
            "RegisterAgent",
            &(&path, "KeyboardDisplay"),
        )
        .await;
        match registered {
            Ok(()) => self.registered = true,
            Err(error) if error_of(&error).0.ends_with(".AlreadyExists") => self.registered = true,
            Err(error) => eprintln!("[bluetooth] registering the pairing agent: {error}"),
        }
        if self.registered {
            let default: zbus::Result<()> = call(
                &self.bus,
                "/org/bluez",
                AGENT_MANAGER,
                "RequestDefaultAgent",
                &(&path,),
            )
            .await;
            if let Err(error) = default {
                eprintln!("[bluetooth] the default pairing agent: {error}");
            }
        }
    }

    fn end_questions(&mut self, emit: &mut dyn FnMut(Event)) {
        for standing in self.standing.drain(..) {
            emit(Event::QuestionEnded(standing.id));
        }
    }

    /// Ends the questions that only show a code for the device at `path`: its pairing ended.
    fn end_shown(&mut self, path: &str, emit: &mut dyn FnMut(Event)) {
        self.standing.retain(|standing| {
            let ended = standing.path == path && standing.shows.is_some();
            if ended {
                emit(Event::QuestionEnded(standing.id));
            }
            !ended
        });
    }

    async fn signal(&mut self, message: zbus::Message, emit: &mut dyn FnMut(Event)) {
        let header = message.header();
        let member = header.member().map(|m| m.to_string()).unwrap_or_default();
        let path = header.path().map(|p| p.to_string()).unwrap_or_default();
        let body = message.body();
        match member.as_str() {
            "NameOwnerChanged" => {
                let Ok((_, _, owner)) = body.deserialize::<(String, String, String)>() else {
                    return;
                };
                if owner.is_empty() {
                    self.available = false;
                    self.objects = Objects::default();
                    self.registered = false;
                    self.discovering = false;
                    self.discoverable = false;
                    self.busy.clear();
                    self.end_questions(emit);
                } else {
                    self.read(emit).await;
                }
            }
            "InterfacesAdded" => {
                if let Ok((object, interfaces)) =
                    body.deserialize::<(OwnedObjectPath, Interfaces)>()
                {
                    self.objects.added(object.as_str(), &interfaces);
                }
            }
            "InterfacesRemoved" => {
                if let Ok((object, interfaces)) =
                    body.deserialize::<(OwnedObjectPath, Vec<String>)>()
                {
                    self.objects.removed(object.as_str(), &interfaces);
                    if interfaces.iter().any(|i| i == DEVICE) {
                        self.end_shown(object.as_str(), emit);
                    }
                }
            }
            "PropertiesChanged" => {
                let Ok((interface, changed, invalidated)) =
                    body.deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                else {
                    return;
                };
                let was_powered = self.objects.powered();
                self.objects
                    .changed(&path, &interface, &changed, &invalidated);
                if interface == ADAPTER {
                    if was_powered && !self.objects.powered() {
                        // BlueZ ends discovery as the adapter goes off.
                        self.discovering = false;
                        self.discoverable = false;
                    }
                    if changed.contains_key("Discoverable") {
                        self.asking_discoverable = false;
                    }
                }
                if interface == DEVICE && self.objects.paired(&path) {
                    self.end_shown(&path, emit);
                }
            }
            _ => {}
        }
    }

    /// Discovery as wanted: on while the caller wants it and the adapter is on, discoverable while
    /// it discovers; else off, and not discoverable if this connection made it so.
    async fn discover(&mut self) {
        let wanted = self.want && self.available && self.objects.powered();
        let Some(adapter) = self.objects.adapter_path().map(str::to_string) else {
            return;
        };
        if wanted && !self.discovering && !self.start_failed {
            let mut filter: HashMap<&str, Value> = HashMap::new();
            // A device's advertisement seen again is no change: nothing is signalled for it.
            filter.insert("DuplicateData", Value::from(false));
            let _: zbus::Result<()> = call(
                &self.bus,
                &adapter,
                ADAPTER,
                "SetDiscoveryFilter",
                &(filter,),
            )
            .await;
            let started: zbus::Result<()> =
                call(&self.bus, &adapter, ADAPTER, "StartDiscovery", &()).await;
            match started {
                Ok(()) => self.discovering = true,
                Err(error) => {
                    eprintln!("[bluetooth] discovering: {error}");
                    self.start_failed = true;
                }
            }
        }
        if !wanted {
            self.start_failed = false;
            if self.discovering {
                let _: zbus::Result<()> =
                    call(&self.bus, &adapter, ADAPTER, "StopDiscovery", &()).await;
                self.discovering = false;
            }
        }
        if wanted && self.discovering && !self.objects.discoverable() && !self.asking_discoverable {
            // Made discoverable as a Mac is while its Bluetooth settings are open; BlueZ ends it
            // after its timeout, and it is asked again as that is signalled.
            self.asking_discoverable = true;
            match set(&self.bus, &adapter, ADAPTER, "Discoverable", true.into()).await {
                Ok(()) => self.discoverable = true,
                Err(error) => eprintln!("[bluetooth] discoverable: {error}"),
            }
        }
        if !wanted && self.discoverable {
            self.discoverable = false;
            self.asking_discoverable = false;
            if self.objects.discoverable() {
                let _ = set(&self.bus, &adapter, ADAPTER, "Discoverable", false.into()).await;
            }
        }
    }

    /// Runs a call off the loop, its outcome told back as `Finished`.
    fn spawn(
        &self,
        call: u64,
        address: String,
        path: String,
        work: impl std::future::Future<Output = Result<Done, Failure>> + Send + 'static,
    ) {
        let finished = self.finished.clone();
        self.executor
            .spawn(
                async move {
                    let outcome = work.await;
                    let _ = finished
                        .send(Finished {
                            call,
                            address,
                            path,
                            outcome,
                        })
                        .await;
                },
                "bluetooth-call",
            )
            .detach();
    }

    fn command(&mut self, command: Command, emit: &mut dyn FnMut(Event)) {
        if let Command::Discover(want) = command {
            self.want = want;
            return;
        }
        if let Command::Power { on, call } = command {
            let Some(adapter) = self.objects.adapter_path().map(str::to_string) else {
                emit(Event::Done {
                    call,
                    outcome: Err(Failure::Unavailable),
                });
                return;
            };
            let bus = self.bus.clone();
            self.spawn(call, String::new(), adapter.clone(), async move {
                set(&bus, &adapter, ADAPTER, "Powered", on.into())
                    .await
                    .map(|()| Done::Powered(on))
                    .map_err(|error| failure(&error))
            });
            return;
        }
        let id = command.device().unwrap_or_default().to_string();
        let path = self.objects.device_path(&id).map(str::to_string);
        let answered = match &command {
            Command::Connect { call, .. }
            | Command::Disconnect { call, .. }
            | Command::Forget { call, .. }
            | Command::Rename { call, .. } => Some(*call),
            _ => None,
        };
        let Some(path) = path else {
            if let Some(call) = answered {
                emit(Event::Done {
                    call,
                    outcome: Err(Failure::NotFound),
                });
            }
            return;
        };
        let bus = self.bus.clone();
        match command {
            Command::Connect { call, .. } | Command::Disconnect { call, .. }
                if self.busy.contains_key(&id) =>
            {
                emit(Event::Done {
                    call,
                    outcome: Err(Failure::Busy),
                });
            }
            Command::Connect { call, .. } => {
                if !self.objects.powered() {
                    emit(Event::Done {
                        call,
                        outcome: Err(Failure::Unavailable),
                    });
                    return;
                }
                self.refused.remove(&path);
                self.busy.insert(id.clone(), Busy::Connecting);
                let (paired, trusted) = (self.objects.paired(&path), self.objects.trusted(&path));
                let at = path.clone();
                self.spawn(call, id, path, async move {
                    connect(&bus, &at, paired, trusted).await
                });
            }
            Command::Disconnect { call, .. } => {
                self.busy.insert(id.clone(), Busy::Disconnecting);
                let at = path.clone();
                self.spawn(call, id, path, async move {
                    let done: zbus::Result<()> =
                        self::call(&bus, &at, DEVICE, "Disconnect", &()).await;
                    done.map(|()| Done::Disconnected)
                        .map_err(|error| failure(&error))
                });
            }
            Command::Forget { call, .. } => {
                let Some(adapter) = self.objects.adapter_path().map(str::to_string) else {
                    return;
                };
                let at = path.clone();
                self.spawn(call, id, path, async move {
                    let object = zbus::zvariant::ObjectPath::try_from(at.as_str())
                        .map_err(|_| Failure::NotFound)?;
                    let done: zbus::Result<()> =
                        self::call(&bus, &adapter, ADAPTER, "RemoveDevice", &(object,)).await;
                    done.map(|()| Done::Forgotten)
                        .map_err(|error| failure(&error))
                });
            }
            Command::Rename { name, call, .. } => {
                let name = crate::name_given(&name);
                let at = path.clone();
                self.spawn(call, id, path, async move {
                    set(&bus, &at, DEVICE, "Alias", Value::from(name.as_str()))
                        .await
                        .map(|()| Done::Renamed(name))
                        .map_err(|error| failure(&error))
                });
            }
            Command::CancelPairing { .. } => {
                self.executor
                    .spawn(
                        async move {
                            let _: zbus::Result<()> =
                                call(&bus, &path, DEVICE, "CancelPairing", &()).await;
                        },
                        "bluetooth-cancel",
                    )
                    .detach();
            }
            Command::Power { .. } | Command::Discover(_) => {}
        }
    }

    fn heard(&mut self, heard: Heard, emit: &mut dyn FnMut(Event)) {
        match heard {
            Heard::Asked { path, ask, reply } => {
                // A code shown again (a passkey as its digits are typed) is the same question.
                if ask.shows()
                    && self
                        .standing
                        .iter()
                        .any(|s| s.path == path && s.shows.as_ref() == Some(&ask))
                {
                    return;
                }
                let Some((address, name, kind)) = self.objects.describe(&path) else {
                    let _ = reply.try_send(Answer::Refuse);
                    return;
                };
                self.next_question += 1;
                let id = self.next_question;
                self.standing.push(Standing {
                    id,
                    path,
                    shows: ask.shows().then(|| ask.clone()),
                });
                emit(Event::Question(Question {
                    id,
                    ask,
                    address,
                    name,
                    kind,
                    reply: Some(reply),
                }));
            }
            Heard::Refused(path) => {
                self.refused.insert(path);
            }
            Heard::Cancelled => self.end_questions(emit),
            Heard::Released => self.registered = false,
        }
    }

    fn finished(&mut self, finished: Finished, emit: &mut dyn FnMut(Event)) {
        if !finished.address.is_empty() {
            self.busy.remove(&finished.address);
        }
        self.end_shown(&finished.path, emit);
        let refused = self.refused.remove(&finished.path);
        let outcome = match finished.outcome {
            // The person said no to what the pairing asked: theirs, not a failure to say.
            Err(Failure::Rejected | Failure::Cancelled) if refused => Err(Failure::Cancelled),
            outcome => outcome,
        };
        emit(Event::Done {
            call: finished.call,
            outcome,
        });
    }

    /// As the session ends: no discovery of its own left, the adapter not discoverable because of
    /// it, and the agent forgotten.
    async fn part(&mut self) {
        let adapter = self.objects.adapter_path().map(str::to_string);
        if let Some(adapter) = &adapter {
            if self.discovering {
                let _: zbus::Result<()> =
                    call(&self.bus, adapter, ADAPTER, "StopDiscovery", &()).await;
            }
            if self.discoverable && self.objects.discoverable() {
                let _ = set(&self.bus, adapter, ADAPTER, "Discoverable", false.into()).await;
            }
        }
        if self.registered {
            let path = zbus::zvariant::ObjectPath::from_static_str_unchecked(AGENT_PATH);
            let _: zbus::Result<()> = call(
                &self.bus,
                "/org/bluez",
                AGENT_MANAGER,
                "UnregisterAgent",
                &(&path,),
            )
            .await;
        }
    }
}

/// Connects the device at `path`: paired first (the agent asking the person what it must) and
/// trusted, so it may connect by itself again, as a Mac's paired devices do.
async fn connect(
    bus: &zbus::Connection,
    path: &str,
    paired: bool,
    trusted: bool,
) -> Result<Done, Failure> {
    let mut fresh = false;
    if !paired {
        let pairing: zbus::Result<()> = call(bus, path, DEVICE, "Pair", &()).await;
        match pairing {
            Ok(()) => fresh = true,
            Err(error) if error_of(&error).0.ends_with(".AlreadyExists") => {}
            Err(error) => return Err(failure(&error)),
        }
    }
    if !trusted || fresh {
        if let Err(error) = set(bus, path, DEVICE, "Trusted", true.into()).await {
            eprintln!("[bluetooth] trusting {path}: {error}");
        }
    }
    let connecting: zbus::Result<()> = call(bus, path, DEVICE, "Connect", &()).await;
    match connecting {
        Ok(()) => Ok(Done::Connected),
        Err(error) if no_profiles(&error) => Ok(Done::Paired),
        Err(error) => Err(failure(&error)),
    }
}

/// Follows BlueZ on the system bus (or at `address`, a played one's), serves the pairing agent and
/// does what `commands` ask, telling `emit` (module doc), until `stop` is readable or `commands`
/// closes; returns when the bus closes, the caller following it again once it is back. Only
/// BlueZ's signals that can change what is shown are delivered: its objects added and removed, the
/// properties of its adapters, devices and batteries, and its name's owner; of those, a change of
/// nothing shown (a device's signal strength while discovering) tells nothing.
pub fn run(
    address: Option<&str>,
    stop: BorrowedFd<'_>,
    commands: &async_channel::Receiver<Command>,
    mut emit: impl FnMut(Event),
) -> io::Result<()> {
    let emit: &mut dyn FnMut(Event) = &mut emit;
    let stop = async_io::Async::new(stop.try_clone_to_owned()?)?;
    let (heard, hearing) = async_channel::unbounded();
    let builder = match address {
        Some(address) => zbus::connection::Builder::address(address),
        None => zbus::connection::Builder::system(),
    }
    .map_err(failed)?
    .internal_executor(false)
    .serve_at(AGENT_PATH, Agent { heard })
    .map_err(failed)?;
    let bus = futures_lite::future::block_on(builder.build()).map_err(failed)?;
    let executor = bus.executor().clone();
    let wait = |rule| {
        futures_lite::future::block_on(ticked(
            &executor,
            zbus::MessageStream::for_match_rule(rule, &bus, Some(64)),
        ))
    };
    let rules = [
        rule(Some("/"), OBJECT_MANAGER, "InterfacesAdded", None),
        rule(Some("/"), OBJECT_MANAGER, "InterfacesRemoved", None),
        rule(None, PROPERTIES, "PropertiesChanged", Some(ADAPTER)),
        rule(None, PROPERTIES, "PropertiesChanged", Some(DEVICE)),
        rule(None, PROPERTIES, "PropertiesChanged", Some(BATTERY)),
    ];
    let owner = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")
        .map_err(failed)?
        .member("NameOwnerChanged")
        .map_err(failed)?
        .arg(0, BLUEZ)
        .map_err(failed)?
        .build();
    let mut streams = Vec::new();
    for rule in rules {
        let rule = rule.map_err(failed)?;
        streams.push(wait(rule).map_err(failed)?);
    }
    streams.push(wait(owner).map_err(failed)?);
    let (finished, finishing) = async_channel::unbounded();
    let mut link = Link {
        bus: bus.clone(),
        executor: executor.clone(),
        objects: Objects::default(),
        available: false,
        busy: BTreeMap::new(),
        want: false,
        discovering: false,
        start_failed: false,
        discoverable: false,
        asking_discoverable: false,
        registered: false,
        next_question: 0,
        standing: Vec::new(),
        refused: HashSet::new(),
        told: None,
        finished,
    };
    futures_lite::future::block_on(ticked(&executor, link.read(emit)));
    link.tell(emit);
    let ended = loop {
        let next = futures_lite::future::block_on(ticked(&executor, async {
            futures_lite::future::or(
                async {
                    match next_of(&mut streams).await {
                        Some(message) => Next::Signal(message),
                        None => Next::Closed,
                    }
                },
                futures_lite::future::or(
                    async {
                        match commands.recv().await {
                            Ok(command) => Next::Command(command),
                            Err(_) => Next::Stop,
                        }
                    },
                    futures_lite::future::or(
                        async {
                            match hearing.recv().await {
                                Ok(heard) => Next::Heard(heard),
                                Err(_) => Next::Closed,
                            }
                        },
                        futures_lite::future::or(
                            async {
                                match finishing.recv().await {
                                    Ok(done) => Next::Finished(done),
                                    Err(_) => Next::Closed,
                                }
                            },
                            async {
                                let _ = stop.readable().await;
                                Next::Stop
                            },
                        ),
                    ),
                ),
            )
            .await
        }));
        match next {
            Next::Stop => break false,
            Next::Closed => break true,
            Next::Signal(message) => {
                futures_lite::future::block_on(ticked(&executor, link.signal(message, emit)))
            }
            Next::Command(command) => link.command(command, emit),
            Next::Heard(heard) => link.heard(heard, emit),
            Next::Finished(done) => link.finished(done, emit),
        }
        futures_lite::future::block_on(ticked(&executor, link.discover()));
        link.tell(emit);
    };
    if !ended {
        futures_lite::future::block_on(ticked(
            &executor,
            futures_lite::future::or(link.part(), async {
                async_io::Timer::after(PARTING).await;
            }),
        ));
    }
    link.end_questions(emit);
    drop(streams);
    drop(link);
    drop(bus);
    // The connection's tasks (its socket's reader, the object server, its matches removed) end as
    // it closes; one that does not within the parting's time is dropped with the executor.
    futures_lite::future::block_on(futures_lite::future::or(
        async {
            while !executor.is_empty() {
                executor.tick().await;
            }
        },
        async {
            async_io::Timer::after(PARTING).await;
        },
    ));
    Ok(())
}
