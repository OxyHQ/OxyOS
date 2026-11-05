use super::*;

/// The image's sound theme, as the repository keeps it.
fn image_share() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../config/includes.chroot_after_packages/usr/share")
}

#[test]
fn every_sound_has_its_freedesktop_name_and_is_in_the_images_theme() {
    assert_eq!(name(SystemSound::MoveToTrash), "file-trash");
    assert_eq!(name(SystemSound::EmptyTrash), "trash-empty");
    assert_eq!(name(SystemSound::Notification), "message-new-instant");
    assert!(is_interface(SystemSound::MoveToTrash));
    assert!(is_interface(SystemSound::EmptyTrash));
    assert!(!is_interface(SystemSound::Notification));
    let share = image_share();
    for sound in SOUNDS {
        let file = find(std::slice::from_ref(&share), sound).expect("in the image");
        assert!(
            file.starts_with(share.join("sounds/oxyos/stereo")),
            "{file:?}"
        );
    }
}

#[test]
fn the_theme_is_looked_for_in_order_oxyos_first() {
    let root = std::env::temp_dir().join(format!("oxyos-sound-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (first, second) = (root.join("first"), root.join("second"));
    let make = |dir: &Path, theme: &str, file: &str| {
        let folder = dir.join("sounds").join(theme).join("stereo");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(file), b"").unwrap();
        folder.join(file)
    };
    assert_eq!(
        find(&[first.clone(), second.clone()], SystemSound::EmptyTrash),
        None
    );
    let freedesktop = make(&second, "freedesktop", "trash-empty.oga");
    assert_eq!(
        find(&[first.clone(), second.clone()], SystemSound::EmptyTrash),
        Some(freedesktop)
    );
    let ours = make(&second, "oxyos", "trash-empty.ogg");
    assert_eq!(
        find(&[first.clone(), second.clone()], SystemSound::EmptyTrash),
        Some(ours)
    );
    // An earlier data directory's comes first, as the XDG specification orders them.
    let earlier = make(&first, "freedesktop", "trash-empty.oga");
    assert_eq!(
        find(&[first, second], SystemSound::EmptyTrash),
        Some(earlier)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_images_sounds_decode_whole_starting_at_once() {
    let share = image_share();
    let decoded = |sound| decode(&find(std::slice::from_ref(&share), sound).unwrap()).unwrap();
    // The owner's trash sound: 44.1 kHz stereo, 0.66 s once its silences were cut (assets'
    // provenance, usr/share/doc/oxyos-sounds/copyright); both Trash names are the one file.
    let trash = decoded(SystemSound::EmptyTrash);
    assert_eq!((trash.rate, trash.channels), (44_100, 2));
    assert_eq!(trash.frames(), 29_021);
    assert_eq!(decoded(SystemSound::MoveToTrash), trash);
    // The owner's notification sound: 48 kHz stereo, 3.4 s.
    let notification = decoded(SystemSound::Notification);
    assert_eq!((notification.rate, notification.channels), (48_000, 2));
    assert_eq!(notification.frames(), 163_172);
    for pcm in [&trash, &notification] {
        // Sound from the first hundredth of a second: no silence before it.
        let first = &pcm.samples[..pcm.rate as usize / 100 * 2];
        assert!(
            first.iter().any(|sample| sample.unsigned_abs() > 300),
            "{:?}",
            &first[..8]
        );
    }
}

#[test]
fn a_recorder_keeps_what_it_was_asked_in_order() {
    let recorder = Recorder::default();
    let mut player: Box<dyn Player> = Box::new(recorder.clone());
    player.play(SystemSound::MoveToTrash);
    player.play(SystemSound::Notification);
    assert_eq!(
        recorder.take(),
        [SystemSound::MoveToTrash, SystemSound::Notification]
    );
    assert!(recorder.take().is_empty());
}
