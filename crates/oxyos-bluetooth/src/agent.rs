//! The machine's pairing agent (`org.bluez.Agent1`), served on the session's connection to BlueZ
//! and registered as its default, with the `KeyboardDisplay` capability: it can show a code and
//! have one typed, so BlueZ uses the safest pairing both devices can (numeric comparison, passkey
//! entry), and a legacy device's PIN. Every question goes to the connection's loop, which hands it
//! to the session (a system sheet); its answer comes back here. A question shown only (a code to
//! type on the other device) is answered at once, as BlueZ expects, and ends with the pairing.

use crate::{Answer, Ask};
use zbus::zvariant::OwnedObjectPath;

/// What the agent tells the connection's loop.
pub(crate) enum Heard {
    Asked {
        path: String,
        ask: Ask,
        reply: async_channel::Sender<Answer>,
    },
    /// The person refused a question about this device: its pairing failing is theirs.
    Refused(String),
    /// BlueZ cancelled the question standing.
    Cancelled,
    /// BlueZ no longer uses this agent.
    Released,
}

#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "org.bluez.Error")]
pub(crate) enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
    Canceled(String),
}

pub(crate) struct Agent {
    pub(crate) heard: async_channel::Sender<Heard>,
}

impl Agent {
    /// Asks the person, waiting for the answer; refused when nothing answers.
    async fn ask(&self, device: &OwnedObjectPath, ask: Ask) -> Answer {
        let (reply, answered) = async_channel::bounded(1);
        let path = device.to_string();
        if self
            .heard
            .send(Heard::Asked {
                path: path.clone(),
                ask,
                reply,
            })
            .await
            .is_err()
        {
            return Answer::Refuse;
        }
        let answer = answered.recv().await.unwrap_or(Answer::Refuse);
        if answer == Answer::Refuse {
            let _ = self.heard.send(Heard::Refused(path)).await;
        }
        answer
    }

    /// Shows a code; nothing waits for an answer.
    async fn show(&self, device: &OwnedObjectPath, ask: Ask) {
        let (reply, _) = async_channel::bounded(1);
        let _ = self
            .heard
            .send(Heard::Asked {
                path: device.to_string(),
                ask,
                reply,
            })
            .await;
    }

    fn agreed(answer: Answer) -> Result<(), AgentError> {
        match answer {
            Answer::Accept => Ok(()),
            _ => Err(AgentError::Rejected("refused by the person".into())),
        }
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    async fn release(&self) {
        let _ = self.heard.send(Heard::Released).await;
    }

    async fn request_pin_code(&self, device: OwnedObjectPath) -> Result<String, AgentError> {
        match self.ask(&device, Ask::PinCode).await {
            Answer::Code(code) if crate::code_valid(&Ask::PinCode, &code) => Ok(code),
            _ => Err(AgentError::Rejected("no PIN given".into())),
        }
    }

    async fn display_pin_code(
        &self,
        device: OwnedObjectPath,
        pincode: String,
    ) -> Result<(), AgentError> {
        self.show(&device, Ask::ShowPinCode(pincode)).await;
        Ok(())
    }

    async fn request_passkey(&self, device: OwnedObjectPath) -> Result<u32, AgentError> {
        match self.ask(&device, Ask::Passkey).await {
            Answer::Code(code) if crate::code_valid(&Ask::Passkey, &code) => code
                .parse()
                .map_err(|_| AgentError::Rejected("not a passkey".into())),
            _ => Err(AgentError::Rejected("no passkey given".into())),
        }
    }

    async fn display_passkey(&self, device: OwnedObjectPath, passkey: u32, _entered: u16) {
        self.show(&device, Ask::ShowPasskey(passkey)).await;
    }

    async fn request_confirmation(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
    ) -> Result<(), AgentError> {
        Self::agreed(self.ask(&device, Ask::Confirm(passkey)).await)
    }

    async fn request_authorization(&self, device: OwnedObjectPath) -> Result<(), AgentError> {
        Self::agreed(self.ask(&device, Ask::Authorize).await)
    }

    async fn authorize_service(
        &self,
        device: OwnedObjectPath,
        uuid: String,
    ) -> Result<(), AgentError> {
        Self::agreed(self.ask(&device, Ask::Service(uuid)).await)
    }

    async fn cancel(&self) {
        let _ = self.heard.send(Heard::Cancelled).await;
    }
}
