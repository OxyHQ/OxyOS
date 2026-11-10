//! Memory pressure (docs/adr/0066-quality-of-service.mdx, decision 7): the program running the
//! compositor follows the kernel's pressure stall information and says how short of memory the
//! machine is; the applications are told, so they give back what they can make again before the
//! kernel must take it. Those the person does not see first: at a warning, only an application
//! none of whose windows is shown (minimized, hidden, on another desktop, the session locked, or
//! running without one); at critical, every application and the shell. Each is told a level once,
//! a higher one when it comes, and normal again once it eases.

use super::{ClientId, Scene};
use oxyos_protocol::{Event, MemoryPressureLevel, MEMORY_PRESSURE_VERSION};
use std::collections::BTreeMap;

/// The level the program said, and what each client was last told.
#[derive(Debug, Default)]
pub(super) struct MemoryPressure {
    level: Option<MemoryPressureLevel>,
    told: BTreeMap<ClientId, MemoryPressureLevel>,
}

impl Scene {
    /// How short of memory the machine is now: the clients are told as `tell_memory_pressure`
    /// says.
    pub fn set_memory_pressure(&mut self, level: MemoryPressureLevel) {
        self.memory_pressure.level = Some(level);
        self.tell_memory_pressure();
    }

    /// The level the program last said (normal before it said one).
    pub fn memory_pressure(&self) -> MemoryPressureLevel {
        self.memory_pressure
            .level
            .unwrap_or(MemoryPressureLevel::Normal)
    }

    /// Tells each client what the level asks of it now, if it was not told it: the program calls
    /// it every turn, so an application hidden while the machine is short of memory is told then.
    /// Nothing to do, and nothing walked, while the machine is not and nobody was told.
    pub fn tell_memory_pressure(&mut self) {
        let level = self.memory_pressure();
        if level == MemoryPressureLevel::Normal && self.memory_pressure.told.is_empty() {
            return;
        }
        // Those not shown before those shown, so the hidden give back first.
        let mut clients: Vec<(bool, ClientId)> = self
            .clients
            .iter()
            .filter(|(client, grants)| self.application(**client).is_some() || grants.shell)
            .filter(|(client, _)| {
                self.version_of(**client)
                    .is_none_or(|version| version >= MEMORY_PRESSURE_VERSION)
            })
            .map(|(client, _)| (self.client_shown(*client), *client))
            .collect();
        clients.sort();
        for (shown, client) in clients {
            let told = self
                .memory_pressure
                .told
                .get(&client)
                .copied()
                .unwrap_or(MemoryPressureLevel::Normal);
            let application = self.application(client).is_some();
            let wanted = match level {
                MemoryPressureLevel::Normal => MemoryPressureLevel::Normal,
                MemoryPressureLevel::Warning if application && !shown => {
                    MemoryPressureLevel::Warning
                }
                MemoryPressureLevel::Warning => told,
                MemoryPressureLevel::Critical => MemoryPressureLevel::Critical,
            };
            let changed = match level {
                // Eased: each told a level is told it is over, once.
                MemoryPressureLevel::Normal => told != MemoryPressureLevel::Normal,
                // Only ever higher while it lasts: what was given back stays given back.
                _ => wanted > told,
            };
            if !changed {
                continue;
            }
            if wanted == MemoryPressureLevel::Normal {
                self.memory_pressure.told.remove(&client);
            } else {
                self.memory_pressure.told.insert(client, wanted);
            }
            self.emit(client, Event::MemoryPressure { level: wanted });
        }
    }

    pub(super) fn forget_memory_pressure_client(&mut self, client: ClientId) {
        self.memory_pressure.told.remove(&client);
    }
}
