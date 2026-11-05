//! Plays system sounds through PipeWire as the session does (docs/adr/0044-system-sounds.mdx), from
//! the sound theme in the data directories: `play [NAME...]`, each by its sound naming name
//! (`file-trash`, `trash-empty`, `message-new-instant`, `screen-capture`), a second apart; all of them by default.

use oxyos_sound::{name, PipeWire, Player, SOUNDS};
use std::time::Duration;

fn main() -> std::io::Result<()> {
    let asked: Vec<String> = std::env::args().skip(1).collect();
    let mut player = PipeWire::spawn(oxyos_sound::data_dirs())?;
    for sound in SOUNDS {
        if asked.is_empty() || asked.iter().any(|a| a == name(sound)) {
            println!("{}", name(sound));
            player.play(sound);
            std::thread::sleep(Duration::from_secs(4));
        }
    }
    Ok(())
}
