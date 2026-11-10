//! The person's user manager played on a bus of a test's own (never the person's): systemd's
//! `org.freedesktop.systemd1.Manager` as far as placing and weighing scopes goes. Each scope asked
//! for is recorded; its job ends `done` with `JobRemoved`, sent to the bus after the call's answer,
//! as a real manager's ends after it; `SetUnitProperties` on a scope it never made fails as
//! systemd's does (`NoSuchUnit`).

use std::sync::{Arc, Mutex};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

/// What the played manager was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    /// `StartTransientUnit` of a scope: its name, slice, processes and CPU weight.
    Start {
        name: String,
        slice: String,
        pids: Vec<u32>,
        cpu_weight: Option<u64>,
    },
    /// `SetUnitProperties`: the scope, whether for this boot only, and its CPU and disk weights.
    Set {
        name: String,
        runtime: bool,
        cpu_weight: Option<u64>,
        io_weight: Option<u64>,
        memory_low: Option<u64>,
    },
}

#[derive(Default)]
struct Model {
    calls: Vec<Call>,
    units: Vec<String>,
    next_job: u32,
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.systemd1")]
enum SystemdError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NoSuchUnit(String),
    UnitExists(String),
}

type Bus = Arc<Mutex<Option<zbus::blocking::Connection>>>;

struct Systemd {
    model: Arc<Mutex<Model>>,
    bus: Bus,
}

fn property<T: TryFrom<OwnedValue>>(properties: &[(String, OwnedValue)], name: &str) -> Option<T> {
    properties
        .iter()
        .find(|(key, _)| key == name)
        .and_then(|(_, value)| value.try_clone().ok())
        .and_then(|value| T::try_from(value).ok())
}

#[zbus::interface(name = "org.freedesktop.systemd1.Manager")]
impl Systemd {
    fn subscribe(&self) {}

    fn start_transient_unit(
        &self,
        name: String,
        _mode: String,
        properties: Vec<(String, OwnedValue)>,
        _auxiliary: Vec<(String, Vec<(String, OwnedValue)>)>,
    ) -> Result<OwnedObjectPath, SystemdError> {
        let job = {
            let mut model = self.model.lock().unwrap();
            if model.units.contains(&name) {
                return Err(SystemdError::UnitExists(name));
            }
            model.calls.push(Call::Start {
                name: name.clone(),
                slice: property::<String>(&properties, "Slice").unwrap_or_default(),
                pids: property::<Vec<u32>>(&properties, "PIDs").unwrap_or_default(),
                cpu_weight: property::<u64>(&properties, "CPUWeight"),
            });
            model.units.push(name.clone());
            model.next_job += 1;
            model.next_job
        };
        let path = OwnedObjectPath::try_from(format!("/org/freedesktop/systemd1/job/{job}"))
            .map_err(|error| SystemdError::ZBus(error.into()))?;
        // The job ends after the answer, from a thread of its own (never inside the call).
        let (bus, ended) = (self.bus.clone(), path.clone());
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            if let Some(bus) = bus.lock().unwrap().clone() {
                let _ = bus.emit_signal(
                    None::<()>,
                    "/org/freedesktop/systemd1",
                    "org.freedesktop.systemd1.Manager",
                    "JobRemoved",
                    &(job, ObjectPath::from(&ended), name.as_str(), "done"),
                );
            }
        });
        Ok(path)
    }

    fn set_unit_properties(
        &self,
        name: String,
        runtime: bool,
        properties: Vec<(String, OwnedValue)>,
    ) -> Result<(), SystemdError> {
        let mut model = self.model.lock().unwrap();
        // The user manager's own slices are always there.
        if !model.units.contains(&name) && !name.ends_with(".slice") {
            return Err(SystemdError::NoSuchUnit(format!("Unit {name} not loaded.")));
        }
        model.calls.push(Call::Set {
            name,
            runtime,
            cpu_weight: property::<u64>(&properties, "CPUWeight"),
            io_weight: property::<u64>(&properties, "IOWeight"),
            memory_low: property::<u64>(&properties, "MemoryLow"),
        });
        Ok(())
    }
}

/// The played user manager, serving until dropped.
pub struct Played {
    model: Arc<Mutex<Model>>,
    _connection: zbus::blocking::Connection,
}

impl Played {
    /// Serves on the bus at `address`, under systemd's name.
    pub fn serve(address: &str) -> zbus::Result<Self> {
        let model = Arc::new(Mutex::new(Model::default()));
        let bus: Bus = Arc::new(Mutex::new(None));
        let connection = zbus::blocking::connection::Builder::address(address)?
            .name("org.freedesktop.systemd1")?
            .serve_at(
                "/org/freedesktop/systemd1",
                Systemd {
                    model: model.clone(),
                    bus: bus.clone(),
                },
            )?
            .build()?;
        *bus.lock().unwrap() = Some(connection.clone());
        Ok(Self {
            model,
            _connection: connection,
        })
    }

    /// Every call it answered, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.model.lock().unwrap().calls.clone()
    }
}
