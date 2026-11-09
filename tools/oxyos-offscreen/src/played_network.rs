//! The network behind a played NetworkManager (`--network-bus`, docs/adr/0038-networks.mdx), as
//! Wi-Fi Diagnostics probe it: the router's echoes and ARP, and the resolver, answered by the
//! played daemon's control interface (`Probe`), which counts each, so a scenario says exactly what
//! diagnostics asked and that nothing asked again. The session program probes the machine's own
//! (`oxyos_networks::SystemProbe`).

use oxyos_networks::{Ping, Probe, Resolved};
use serde_json::{json, Value};
use std::net::Ipv4Addr;
use std::time::Duration;

pub struct PlayedProbe {
    pub bus: zbus::blocking::Connection,
}

impl PlayedProbe {
    fn ask(&self, what: Value) -> Option<Value> {
        let reply = self
            .bus
            .call_method(
                Some("org.freedesktop.NetworkManager"),
                "/so/oxy/OxyOS/FakeNetworkManager",
                Some("so.oxy.OxyOS.FakeNetworkManager"),
                "Probe",
                &(what.to_string(),),
            )
            .ok()?;
        let text: String = reply.body().deserialize().ok()?;
        serde_json::from_str(&text).ok()
    }
}

/// A played time in ms as a duration.
fn ms(value: &Value) -> Option<Duration> {
    value
        .as_f64()
        .map(|ms| Duration::from_secs_f64(ms / 1000.0))
}

impl Probe for PlayedProbe {
    fn ping(&self, address: Ipv4Addr, count: u32, _: Duration) -> Ping {
        let Some(answer) = self.ask(json!({"ping": address.to_string(), "count": count})) else {
            return Ping::Failed;
        };
        if answer["not_allowed"] == true {
            return Ping::NotAllowed;
        }
        let answered = answer["rtts_ms"]
            .as_array()
            .map(|times| times.iter().filter_map(ms).collect())
            .unwrap_or_default();
        Ping::Sent {
            sent: count,
            answered,
        }
    }

    fn neighbour(&self, address: Ipv4Addr) -> bool {
        self.ask(json!({"neighbour": address.to_string()}))
            .is_some_and(|answer| answer["neighbour"] == true)
    }

    fn resolve(&self, name: &str, _: Duration) -> Resolved {
        let Some(answer) = self.ask(json!({"resolve": name})) else {
            return Resolved::TimedOut;
        };
        let took = ms(&answer["ms"]).unwrap_or_default();
        match answer["addresses"].as_u64() {
            Some(count) if count > 0 => Resolved::Addresses {
                count: count as usize,
                took,
            },
            _ if answer["timed_out"] == true => Resolved::TimedOut,
            _ => Resolved::Failed { took },
        }
    }
}
