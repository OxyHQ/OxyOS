//! Linux-PAM's application interface (`security/pam_appl.h`), as much of it as checking a
//! password and opening a login's session need: start a transaction, set its terminal and
//! environment, authenticate, check the account, establish credentials, open and close the
//! session, read the environment the modules set, end it. The conversation answers PAM's prompts
//! from what the caller gave.

use libc::{c_char, c_int, c_void};
use std::ffi::{CStr, CString};
use std::ptr;
use zeroize::Zeroizing;

pub(crate) const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
pub(crate) const PAM_PERM_DENIED: c_int = 6;
pub(crate) const PAM_AUTH_ERR: c_int = 7;
pub(crate) const PAM_USER_UNKNOWN: c_int = 10;
pub(crate) const PAM_MAXTRIES: c_int = 11;
pub(crate) const PAM_NEW_AUTHTOK_REQD: c_int = 12;
pub(crate) const PAM_ACCT_EXPIRED: c_int = 13;

const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;

/// An empty password never authenticates, even for an account without one.
pub(crate) const PAM_DISALLOW_NULL_AUTHTOK: c_int = 0x0001;
pub(crate) const PAM_ESTABLISH_CRED: c_int = 0x0002;
pub(crate) const PAM_DELETE_CRED: c_int = 0x0004;

/// The item naming the transaction's terminal.
pub(crate) const PAM_TTY: c_int = 3;

#[repr(C)]
struct PamHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type Conversation = unsafe extern "C" fn(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata_ptr: *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv: Conversation,
    appdata_ptr: *mut c_void,
}

#[allow(unsafe_code)]
// SAFETY: these are Linux-PAM's declarations, from security/pam_appl.h; libpam.so.0 is linked by
// build.rs.
unsafe extern "C" {
    fn pam_start(
        service_name: *const c_char,
        user: *const c_char,
        pam_conversation: *const PamConv,
        pamh: *mut *mut PamHandle,
    ) -> c_int;
    fn pam_start_confdir(
        service_name: *const c_char,
        user: *const c_char,
        pam_conversation: *const PamConv,
        confdir: *const c_char,
        pamh: *mut *mut PamHandle,
    ) -> c_int;
    fn pam_authenticate(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_acct_mgmt(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_setcred(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_open_session(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_close_session(pamh: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_set_item(pamh: *mut PamHandle, item_type: c_int, item: *const c_void) -> c_int;
    fn pam_putenv(pamh: *mut PamHandle, name_value: *const c_char) -> c_int;
    fn pam_getenvlist(pamh: *mut PamHandle) -> *mut *mut c_char;
    fn pam_end(pamh: *mut PamHandle, pam_status: c_int) -> c_int;
    fn pam_strerror(pamh: *mut PamHandle, errnum: c_int) -> *const c_char;
}

/// What the conversation answers with: the user's name to a visible prompt, the password (with
/// its NUL) to the first hidden one. A second hidden prompt (a one-time code, a new password) is
/// not this password's to answer: the conversation fails. Without a password, every hidden
/// prompt fails.
struct Answers {
    user: CString,
    password: Option<Zeroizing<Vec<u8>>>,
}

/// A copy of `text` in memory from malloc, which PAM frees; null if none could be had.
#[allow(unsafe_code)]
fn malloc_copy(text: &CStr) -> *mut c_char {
    let bytes = text.to_bytes_with_nul();
    // SAFETY: calloc returns either null or a block of bytes.len() bytes; copying that many bytes
    // from `bytes` into it stays inside both.
    unsafe {
        let copy = libc::calloc(bytes.len(), 1).cast::<c_char>();
        if !copy.is_null() {
            ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), copy, bytes.len());
        }
        copy
    }
}

/// Frees `count` responses PAM would have taken, wiping the answers first.
#[allow(unsafe_code)]
fn free_responses(responses: *mut PamResponse, count: usize) {
    if responses.is_null() {
        return;
    }
    // SAFETY: `responses` is calloc's block of `count` responses, each answer null or a
    // NUL-terminated copy from malloc_copy; each is wiped to its length and freed once, then the
    // block.
    unsafe {
        for index in 0..count {
            let answer = (*responses.add(index)).resp;
            if !answer.is_null() {
                let length = libc::strlen(answer);
                ptr::write_bytes(answer, 0, length);
                libc::free(answer.cast());
            }
        }
        libc::free(responses.cast());
    }
}

/// PAM's conversation: answers each prompt from the `Answers` behind `appdata_ptr`.
///
/// # Safety
///
/// Called by PAM only, with Linux-PAM's arguments: `msg` points to `num_msg` message pointers,
/// `resp` to where the responses go, and `appdata_ptr` is the `Answers` given to `pam_start`.
#[allow(unsafe_code)]
unsafe extern "C" fn converse(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata_ptr: *mut c_void,
) -> c_int {
    let Ok(count) = usize::try_from(num_msg) else {
        return PAM_CONV_ERR;
    };
    if count == 0 || msg.is_null() || resp.is_null() || appdata_ptr.is_null() {
        return PAM_CONV_ERR;
    }
    // SAFETY: appdata_ptr is the `Answers` the transaction owns, alive for the transaction and
    // used by nothing else while PAM converses.
    let answers = unsafe { &mut *appdata_ptr.cast::<Answers>() };
    // SAFETY: calloc returns null or a zeroed block of `count` responses.
    let block = unsafe { libc::calloc(count, std::mem::size_of::<PamResponse>()) };
    let responses = block.cast::<PamResponse>();
    if responses.is_null() {
        return PAM_BUF_ERR;
    }
    for index in 0..count {
        // SAFETY: Linux-PAM passes `count` valid message pointers in `msg` (an array of pointers,
        // not a pointer to an array of messages).
        let style = unsafe { (**msg.add(index)).msg_style };
        let answer = match style {
            PAM_PROMPT_ECHO_ON => malloc_copy(&answers.user),
            PAM_PROMPT_ECHO_OFF => match answers.password.take() {
                Some(password) => match CStr::from_bytes_with_nul(&password) {
                    Ok(password) => malloc_copy(password),
                    Err(_) => ptr::null_mut(),
                },
                None => {
                    free_responses(responses, count);
                    return PAM_CONV_ERR;
                }
            },
            // Information and error messages need no answer.
            _ => continue,
        };
        if answer.is_null() {
            free_responses(responses, count);
            return PAM_BUF_ERR;
        }
        // SAFETY: `index` is within the `count` responses calloc gave.
        unsafe { (*responses.add(index)).resp = answer };
    }
    // SAFETY: `resp` is where PAM takes the responses, which it then owns and frees.
    unsafe { *resp = responses };
    PAM_SUCCESS
}

/// One PAM transaction, from `pam_start` to `pam_end` (when dropped).
pub(crate) struct Transaction {
    handle: *mut PamHandle,
    /// What the conversation answers with, owned here (`Box::into_raw`) and freed after
    /// `pam_end`: PAM keeps a pointer to it for the whole transaction.
    answers: *mut Answers,
    /// The last code a call returned, which `pam_end` is given.
    last: c_int,
}

impl Transaction {
    /// Starts a transaction of `service` (read from `confdir` instead of `/etc/pam.d` when given)
    /// for `user`.
    #[allow(unsafe_code)]
    pub(crate) fn start(
        service: &CStr,
        confdir: Option<&CStr>,
        user: &CStr,
    ) -> Result<Self, String> {
        let answers = Box::into_raw(Box::new(Answers {
            user: user.to_owned(),
            password: None,
        }));
        let conversation = PamConv {
            conv: converse,
            appdata_ptr: answers.cast(),
        };
        let mut handle: *mut PamHandle = ptr::null_mut();
        // SAFETY: every pointer is valid for the call: the C strings outlive it, PAM copies the
        // conversation structure, and the `answers` it points to live until after pam_end.
        let started = unsafe {
            match confdir {
                Some(dir) => pam_start_confdir(
                    service.as_ptr(),
                    user.as_ptr(),
                    &conversation,
                    dir.as_ptr(),
                    &mut handle,
                ),
                None => pam_start(service.as_ptr(), user.as_ptr(), &conversation, &mut handle),
            }
        };
        if started != PAM_SUCCESS || handle.is_null() {
            // SAFETY: `answers` came from Box::into_raw above and PAM holds no transaction.
            drop(unsafe { Box::from_raw(answers) });
            return Err(format!("PAM could not start ({started})"));
        }
        Ok(Self {
            handle,
            answers,
            last: PAM_SUCCESS,
        })
    }

    /// The password the next hidden prompt is answered with (once).
    #[allow(unsafe_code)]
    pub(crate) fn give_password(&mut self, password: &CStr) {
        let mut bytes = Zeroizing::new(Vec::with_capacity(password.to_bytes_with_nul().len()));
        bytes.extend_from_slice(password.to_bytes_with_nul());
        // SAFETY: `answers` is this transaction's, and PAM is not conversing: no call is running.
        unsafe { (*self.answers).password = Some(bytes) };
    }

    /// Drops a password no prompt asked for.
    #[allow(unsafe_code)]
    pub(crate) fn forget_password(&mut self) {
        // SAFETY: as in give_password.
        unsafe { (*self.answers).password = None };
    }

    fn record(&mut self, code: c_int) -> c_int {
        self.last = code;
        code
    }

    #[allow(unsafe_code)]
    pub(crate) fn authenticate(&mut self, flags: c_int) -> c_int {
        // SAFETY: `handle` is the live transaction pam_start opened.
        let code = unsafe { pam_authenticate(self.handle, flags) };
        self.forget_password();
        self.record(code)
    }

    #[allow(unsafe_code)]
    pub(crate) fn account(&mut self, flags: c_int) -> c_int {
        // SAFETY: `handle` is the live transaction pam_start opened.
        let code = unsafe { pam_acct_mgmt(self.handle, flags) };
        self.record(code)
    }

    #[allow(unsafe_code)]
    pub(crate) fn credentials(&mut self, flags: c_int) -> c_int {
        // SAFETY: `handle` is the live transaction pam_start opened.
        let code = unsafe { pam_setcred(self.handle, flags) };
        self.record(code)
    }

    #[allow(unsafe_code)]
    pub(crate) fn open_session(&mut self) -> c_int {
        // SAFETY: `handle` is the live transaction pam_start opened.
        let code = unsafe { pam_open_session(self.handle, 0) };
        self.record(code)
    }

    #[allow(unsafe_code)]
    pub(crate) fn close_session(&mut self) -> c_int {
        // SAFETY: `handle` is the live transaction pam_start opened.
        let code = unsafe { pam_close_session(self.handle, 0) };
        self.record(code)
    }

    /// Sets a text item (`PAM_TTY`).
    #[allow(unsafe_code)]
    pub(crate) fn set_item(&mut self, item: c_int, value: &CStr) -> c_int {
        // SAFETY: `handle` is live; PAM copies a text item's string.
        let code = unsafe { pam_set_item(self.handle, item, value.as_ptr().cast()) };
        self.record(code)
    }

    /// Sets `NAME=value` in the transaction's environment, which the modules read and add to.
    #[allow(unsafe_code)]
    pub(crate) fn putenv(&mut self, name_value: &CStr) -> c_int {
        // SAFETY: `handle` is live; PAM copies the string.
        let code = unsafe { pam_putenv(self.handle, name_value.as_ptr()) };
        self.record(code)
    }

    /// The transaction's environment, `NAME=value` each.
    #[allow(unsafe_code)]
    pub(crate) fn environment(&self) -> Vec<String> {
        let mut variables = Vec::new();
        // SAFETY: `handle` is live; pam_getenvlist returns null or a null-terminated array of
        // strings from malloc, each and the array the caller's to free.
        unsafe {
            let list = pam_getenvlist(self.handle);
            if list.is_null() {
                return variables;
            }
            let mut at = list;
            while !(*at).is_null() {
                variables.push(CStr::from_ptr(*at).to_string_lossy().into_owned());
                libc::free((*at).cast());
                at = at.add(1);
            }
            libc::free(list.cast());
        }
        variables
    }

    /// PAM's words for `code`.
    #[allow(unsafe_code)]
    pub(crate) fn message(&self, code: c_int) -> String {
        // SAFETY: `handle` is live; pam_strerror returns a static string or null.
        unsafe {
            let text = pam_strerror(self.handle, code);
            if text.is_null() {
                String::new()
            } else {
                CStr::from_ptr(text).to_string_lossy().into_owned()
            }
        }
    }
}

impl Drop for Transaction {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: `handle` is the transaction pam_start opened, ended once here; `answers` came
        // from Box::into_raw in start and PAM uses it no more once the transaction has ended.
        unsafe {
            pam_end(self.handle, self.last);
            drop(Box::from_raw(self.answers));
        }
    }
}

/// The end of one PAM transaction: authentication's code, then the account's when it got that far.
pub(crate) struct Codes {
    pub(crate) authenticate: c_int,
    pub(crate) account: Option<c_int>,
    /// PAM's words for a code that is no answer (a broken configuration, an error).
    pub(crate) message: String,
}

/// One transaction of `service` (read from `confdir` instead of `/etc/pam.d` when given) for
/// `user`: authenticate with `password`, then check the account.
pub(crate) fn check(
    service: &CStr,
    confdir: Option<&CString>,
    user: &CStr,
    password: &CStr,
) -> Result<Codes, String> {
    let mut transaction = Transaction::start(service, confdir.map(CString::as_c_str), user)?;
    transaction.give_password(password);
    let authenticate = transaction.authenticate(PAM_DISALLOW_NULL_AUTHTOK);
    let account = (authenticate == PAM_SUCCESS).then(|| transaction.account(0));
    let message = transaction.message(account.unwrap_or(authenticate));
    Ok(Codes {
        authenticate,
        account,
        message,
    })
}
