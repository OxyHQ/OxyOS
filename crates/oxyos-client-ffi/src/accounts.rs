//! C ABI over the accounts' service's `list` (`oxyos-account`'s `ipc::list`,
//! docs/adr/0020-oxy-account-login.mdx): the enrolled accounts, for the Bloom host run as the login
//! screen, which shows them. The service answers only the login screen's user (and root): for
//! anyone else this fails. Nothing secret is listed.

use super::fail;
use crate::apps::json_string;
use std::ffi::c_char;
use std::path::PathBuf;

/// The service's socket: `OXYOS_ACCOUNTD_SOCKET`, else the system's.
fn socket() -> PathBuf {
    std::env::var_os("OXYOS_ACCOUNTD_SOCKET")
        .map_or_else(|| oxyos_account::ipc::SOCKET.into(), PathBuf::from)
}

/// The enrolled accounts as JSON, `[{"user", "name", "picture" (a file, or null)}]` in the
/// service's order, or why they could not be read.
pub fn accounts_json() -> Result<String, String> {
    accounts_json_at(&socket())
}

fn accounts_json_at(socket: &std::path::Path) -> Result<String, String> {
    let accounts = oxyos_account::ipc::list(socket)
        .map_err(|error| format!("the accounts at {}: {error}", socket.display()))?;
    let items: Vec<String> = accounts
        .iter()
        .map(|account| {
            format!(
                "{{\"user\":{},\"name\":{},\"picture\":{}}}",
                json_string(&account.user),
                json_string(&account.name),
                account
                    .picture
                    .as_deref()
                    .map_or("null".into(), json_string)
            )
        })
        .collect();
    Ok(format!("[{}]", items.join(",")))
}

/// The enrolled accounts as JSON (see `accounts_json`), written into `out` (`capacity` bytes,
/// NUL-terminated when it fits). Returns the JSON's length in bytes (larger than `capacity`:
/// call again with more room; the accounts may have changed meanwhile), or -1 when the service
/// could not be asked or refused (`oxyos_last_error` says why).
///
/// # Safety
/// `out` points to `capacity` writable bytes, or is null with `capacity` 0.
#[allow(unsafe_code)]
#[no_mangle]
pub unsafe extern "C" fn oxyos_accounts(out: *mut c_char, capacity: usize) -> i64 {
    if out.is_null() && capacity != 0 {
        return i64::from(fail("null buffer"));
    }
    let json = match accounts_json() {
        Ok(json) => json,
        Err(error) => return i64::from(fail(error)),
    };
    let bytes = json.as_bytes();
    if bytes.len() < capacity {
        // SAFETY: `out` has `capacity` bytes, more than the JSON and its NUL.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast::<u8>(), bytes.len());
            *out.add(bytes.len()) = 0;
        }
    }
    bytes.len() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyos_account::ipc::{Account, Answer, Listener, Request};

    #[test]
    fn the_accounts_are_the_services_and_none_without_it() {
        let dir = std::env::temp_dir().join(format!("oxyos-ffi-accounts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("accountd");
        assert!(
            accounts_json_at(&socket).is_err(),
            "no service, no accounts"
        );
        let listener = Listener::bind(&socket).unwrap();
        let service = std::thread::spawn(move || {
            let connection = listener.accept().unwrap();
            assert!(matches!(
                connection.receive::<Request>().unwrap(),
                Some(Some(Request::List))
            ));
            connection
                .send(&Answer::Accounts(vec![
                    Account {
                        account_id: "a".into(),
                        user: "oxy-a".into(),
                        uid: 1001,
                        name: "Ada \"Countess\" Lovelace".into(),
                        picture: Some("/var/lib/oxyos/pictures/oxy-a".into()),
                    },
                    Account {
                        account_id: "b".into(),
                        user: "oxy-b".into(),
                        uid: 1002,
                        name: "Bea".into(),
                        picture: None,
                    },
                ]))
                .unwrap();
        });
        assert_eq!(
            accounts_json_at(&socket).unwrap(),
            r#"[{"user":"oxy-a","name":"Ada \"Countess\" Lovelace","picture":"/var/lib/oxyos/pictures/oxy-a"},{"user":"oxy-b","name":"Bea","picture":null}]"#
        );
        service.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
