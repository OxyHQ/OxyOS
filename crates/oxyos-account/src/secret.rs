//! A secret held in memory (a password, a device secret, an access token, a second-factor code or
//! challenge) is `oxyos-secret`'s [`Secret`], and JSON that may hold one is written into a buffer
//! that is overwritten.

use serde::Serialize;
use zeroize::Zeroizing;

/// The one secret type of OxyOS (`oxyos-secret`): overwritten when dropped, never printed.
pub use oxyos_secret::Secret;

/// JSON that may hold secrets: written into a buffer that is overwritten when dropped. The buffer
/// is sized beforehand so that growing it leaves no copy behind.
pub(crate) fn json(value: &impl Serialize) -> Result<Zeroizing<Vec<u8>>, serde_json::Error> {
    let mut buffer = Zeroizing::new(Vec::with_capacity(16 * 1024));
    serde_json::to_writer(&mut *buffer, value)?;
    Ok(buffer)
}
