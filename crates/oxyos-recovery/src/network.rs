//! What the recovery needs of NetworkManager to get online ([ADR 0054](../../../docs/adr/0054-recovery-and-erasing.mdx),
//! decision 5): whether it is, the Wi-Fi networks around, and joining one. Through `nmcli`'s terse
//! output, which escapes `:` and `\` in its fields.

use crate::host::{Command, Failed, Host};

/// A Wi-Fi network in range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    /// 0 to 100.
    pub signal: u8,
    /// Whether joining it asks for a password.
    pub secured: bool,
}

/// Splits a line of `nmcli --terse` into its fields, undoing its escapes.
pub fn fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut characters = line.chars();
    while let Some(c) = characters.next() {
        match c {
            '\\' => {
                if let Some(next) = characters.next() {
                    fields.last_mut().expect("one field at least").push(next);
                }
            }
            ':' => fields.push(String::new()),
            c => fields.last_mut().expect("one field at least").push(c),
        }
    }
    fields
}

/// The networks in `nmcli --terse --fields SSID,SIGNAL,SECURITY device wifi list`'s answer: hidden
/// ones (no name) left out, each name once with its strongest signal, strongest first.
pub fn parse_networks(text: &str) -> Vec<Network> {
    let mut networks: Vec<Network> = Vec::new();
    for line in text.lines() {
        let fields = fields(line);
        let [ssid, signal, security] = fields.as_slice() else {
            continue;
        };
        if ssid.is_empty() {
            continue;
        }
        let signal = signal.parse().unwrap_or(0);
        let secured = !security.is_empty() && security != "--";
        match networks.iter_mut().find(|n| &n.ssid == ssid) {
            Some(known) if known.signal >= signal => {}
            Some(known) => {
                *known = Network {
                    ssid: ssid.clone(),
                    signal,
                    secured,
                }
            }
            None => networks.push(Network {
                ssid: ssid.clone(),
                signal,
                secured,
            }),
        }
    }
    networks.sort_by(|a, b| b.signal.cmp(&a.signal).then_with(|| a.ssid.cmp(&b.ssid)));
    networks
}

/// Whether NetworkManager says the machine is online (`full` connectivity).
pub fn online(host: &mut dyn Host) -> bool {
    host.run(&Command::new("nmcli").args(["--terse", "--fields", "CONNECTIVITY", "general"]))
        .is_ok_and(|said| said.trim() == "full")
}

pub fn networks(host: &mut dyn Host) -> Result<Vec<Network>, Failed> {
    let said = host.run(&Command::new("nmcli").args([
        "--terse",
        "--fields",
        "SSID,SIGNAL,SECURITY",
        "device",
        "wifi",
        "list",
        "--rescan",
        "yes",
    ]))?;
    Ok(parse_networks(&said))
}

/// Joins `ssid`; the password goes to NetworkManager on nmcli's standard input (`--ask`), never on
/// a command line others can read.
pub fn join(host: &mut dyn Host, ssid: &str, password: Option<&str>) -> Result<(), Failed> {
    let mut command = Command::new("nmcli").args(["--ask", "device", "wifi", "connect", ssid]);
    if let Some(password) = password {
        command = command.input(format!("{password}\n").as_bytes());
    }
    host.run(&command).map(drop)
}
