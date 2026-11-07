//! power-profiles-daemon followed (docs/adr/0057-battery-and-energy.mdx, decision 3): a hold of
//! Low Power Mode ends with the daemon (restarted, upgraded) and when another program chooses a
//! profile (`ProfileReleased`), and the session must know to hold it again. Two narrow matches
//! (the daemon's name changing owner; its `ProfileReleased`), so the bus daemon delivers nothing
//! else; waited on with the order to stop, never on a clock.

use crate::failed;
use crate::profiles::{DAEMON, PATH};
use futures_lite::StreamExt;
use std::io;
use std::os::fd::BorrowedFd;
use zbus::blocking::Connection;

/// What happened to the daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// It started, stopped or was replaced: every hold of the one before is gone.
    Owner,
    /// It released a hold, by its cookie.
    Released(u32),
}

/// Follows the daemon on `bus` until `stop` is readable, `changed` returns false or the bus
/// closes.
pub fn profiles(
    bus: &Connection,
    stop: BorrowedFd<'_>,
    mut changed: impl FnMut(Change) -> bool,
) -> io::Result<()> {
    let stop = async_io::Async::new(stop.try_clone_to_owned()?)?;
    let inner = bus.inner();
    let owner = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.DBus")
        .and_then(|rule| rule.member("NameOwnerChanged"))
        .and_then(|rule| rule.arg(0, DAEMON))
        .map_err(failed)?
        .build();
    let released = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(DAEMON)
        .and_then(|rule| rule.path(PATH))
        .and_then(|rule| rule.interface(DAEMON))
        .and_then(|rule| rule.member("ProfileReleased"))
        .map_err(failed)?
        .build();
    let mut owners =
        futures_lite::future::block_on(zbus::MessageStream::for_match_rule(owner, inner, Some(8)))
            .map_err(failed)?;
    let mut releases = futures_lite::future::block_on(zbus::MessageStream::for_match_rule(
        released,
        inner,
        Some(8),
    ))
    .map_err(failed)?;
    loop {
        let next = futures_lite::future::block_on(futures_lite::future::or(
            futures_lite::future::or(
                async { owners.next().await.map(|message| (true, message)) },
                async { releases.next().await.map(|message| (false, message)) },
            ),
            async {
                let _ = stop.readable().await;
                None
            },
        ));
        let Some((owner, message)) = next else {
            return Ok(());
        };
        let Ok(message) = message else {
            continue;
        };
        let change = if owner {
            Change::Owner
        } else {
            match message.body().deserialize::<u32>() {
                Ok(cookie) => Change::Released(cookie),
                Err(_) => continue,
            }
        };
        if !changed(change) {
            return Ok(());
        }
    }
}
