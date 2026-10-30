//! The lock's password through the compositor (docs/adr/0010-lock-screen.mdx): a lock surface
//! submits it, the authentication path answers off the compositor's turn, and only its
//! `Accepted` unlocks.

use oxyos_compositor::{Authentication, Compositor};
use oxyos_compositor_core::scene::Secret;
use oxyos_compositor_core::{Grants, Output, Scene};
use oxyos_protocol::{AuthOutcome, Event, Rect, Request, SecretChunk};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};

const LOCKER: u32 = 7;

/// Accepts "right" only, answering a turn after it was asked; counts what it was given.
struct Fixture {
    asked: Option<bool>,
    ready: OwnedFd,
    _signal: OwnedFd,
    attempts: std::rc::Rc<std::cell::Cell<u32>>,
}

impl Authentication for Fixture {
    fn attempt(&mut self, secret: Secret) -> Option<(AuthOutcome, u32)> {
        self.attempts.set(self.attempts.get() + 1);
        self.asked = Some(&secret[..] == b"right");
        None
    }
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.ready.as_fd()
    }
    fn answer(&mut self) -> Option<(AuthOutcome, u32)> {
        self.asked.take().map(|right| {
            if right {
                (AuthOutcome::Accepted, 0)
            } else {
                (AuthOutcome::Rejected, 0)
            }
        })
    }
}

fn compositor() -> (Compositor, std::rc::Rc<std::cell::Cell<u32>>) {
    let output = Rect::new(0, 0, 640, 480);
    let mut scene = Scene::new();
    scene
        .add_output(Output {
            id: 1,
            rect: output,
            scale_120: 120,
            refresh_mhz: 60_000,
        })
        .unwrap();
    scene
        .connect(
            LOCKER,
            Grants {
                lock: true,
                ..Grants::default()
            },
        )
        .unwrap();
    let mut compositor = Compositor::new(scene, 0);
    let (ready, signal) = rustix::pipe::pipe().unwrap();
    let attempts = std::rc::Rc::new(std::cell::Cell::new(0));
    compositor.authentication = Some(Box::new(Fixture {
        asked: None,
        ready,
        _signal: signal,
        attempts: attempts.clone(),
    }));
    (compositor, attempts)
}

fn submit(compositor: &mut Compositor, password: &str) {
    for secret in SecretChunk::split(password) {
        compositor
            .scene
            .handle(LOCKER, Request::AuthenticateSecret { surface: 1, secret })
            .unwrap();
    }
    compositor
        .scene
        .handle(LOCKER, Request::Authenticate { surface: 1 })
        .unwrap();
}

fn results(compositor: &mut Compositor) -> Vec<AuthOutcome> {
    compositor
        .scene
        .drain_events()
        .into_iter()
        .filter_map(|(_, event)| match event {
            Event::AuthenticationResult { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect()
}

#[test]
fn only_the_authentication_paths_acceptance_unlocks() {
    let (mut compositor, attempts) = compositor();
    compositor.scene.lock();
    compositor
        .scene
        .handle(
            LOCKER,
            Request::CreateLockSurface {
                surface: 1,
                output: 1,
            },
        )
        .unwrap();
    submit(&mut compositor, "wrong");
    // The turn that hands it over answers nothing yet; the next answers.
    compositor.authenticate();
    assert_eq!(attempts.get(), 1);
    assert!(results(&mut compositor).is_empty());
    compositor.authenticate();
    assert_eq!(results(&mut compositor), [AuthOutcome::Rejected]);
    assert!(compositor.scene.is_locked());
    submit(&mut compositor, "right");
    compositor.authenticate();
    assert!(compositor.scene.is_locked(), "locked until the answer");
    compositor.authenticate();
    assert_eq!(results(&mut compositor), [AuthOutcome::Accepted]);
    assert!(!compositor.scene.is_locked());
}

#[test]
fn without_an_authentication_path_nothing_unlocks() {
    let (mut compositor, _) = compositor();
    compositor.authentication = None;
    compositor.scene.lock();
    compositor
        .scene
        .handle(
            LOCKER,
            Request::CreateLockSurface {
                surface: 1,
                output: 1,
            },
        )
        .unwrap();
    submit(&mut compositor, "right");
    compositor.authenticate();
    assert_eq!(results(&mut compositor), [AuthOutcome::Rejected]);
    assert!(compositor.scene.is_locked());
}
