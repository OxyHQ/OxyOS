//! The names OxyOS gives things, each rule in one place. A leaf with no dependencies, so the
//! registry, the package format, the install service, the preferences and the files service
//! all take an application's id by the same rule.

#[cfg(test)]
mod tests;

/// The most bytes an application id has.
pub const MAX_APP_ID: usize = 128;

/// An application id (docs/adr/0030-installing-applications.mdx, decision 1: the package's id,
/// which is reverse-DNS, is the registry's id): at least two labels separated by `.`, each
/// non-empty, of lowercase ASCII letters, digits, `-` and `_`, at most [`MAX_APP_ID`] bytes, as
/// `so.oxy.allo`. The registry's entries (`so.oxy.allo.desktop`), the image's policy of system
/// applications, a package's manifest and signature, the install service's directories, an
/// application's own data and the person's lists of applications all take this rule, so an id
/// one of them accepts is one every other accepts. It names directories and files: it never has
/// a `/`, never starts with `.` and is never `.` or `..`.
pub fn valid_app_id(id: &str) -> bool {
    id.len() <= MAX_APP_ID
        && id.split('.').count() >= 2
        && id.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        })
}
