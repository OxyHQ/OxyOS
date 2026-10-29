//! Linux-PAM's application interface (`security/pam_appl.h`), as much of it as checking a
//! password needs: start a transaction, authenticate, check the account, end it. The conversation
//! answers PAM's prompts from what the caller gave.

use libc::{c_char, c_int, c_void};
use std::ffi::{CStr, CString};
use std::ptr;

pub(crate) const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
pub(crate) const PAM_PERM_DENIED: c_int = 6;
pub(crate) const PAM_AUTH_ERR: c_int = 7;
pub(crate) const PAM_USER_UNKNOWN: c_int = 10;
pub(crate) const PAM_MAXTRIES: c_int = 11;
pub(crate) const PAM_NEW_AUTHTOK_REQD: c_int = 12;

const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;

/// An empty password never authenticates, even for an account without one.
const PAM_DISALLOW_NULL_AUTHTOK: c_int = 0x0001;

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
    fn pam_end(pamh: *mut PamHandle, pam_status: c_int) -> c_int;
    fn pam_strerror(pamh: *mut PamHandle, errnum: c_int) -> *const c_char;
}

/// What the conversation answers with: the user's name to a visible prompt, the password to the
/// first hidden one. A second hidden prompt (a one-time code, a new password) is not this
/// password's to answer: the conversation fails.
struct Answers<'a> {
    user: &'a CStr,
    password: &'a CStr,
    password_given: bool,
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
    // SAFETY: appdata_ptr is the `Answers` authenticate() passed, alive for the transaction and
    // used by nothing else while PAM converses.
    let answers = unsafe { &mut *appdata_ptr.cast::<Answers<'_>>() };
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
            PAM_PROMPT_ECHO_ON => malloc_copy(answers.user),
            PAM_PROMPT_ECHO_OFF if !answers.password_given => {
                answers.password_given = true;
                malloc_copy(answers.password)
            }
            PAM_PROMPT_ECHO_OFF => {
                free_responses(responses, count);
                return PAM_CONV_ERR;
            }
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

/// The end of one PAM transaction: authentication's code, then the account's when it got that far.
pub(crate) struct Codes {
    pub(crate) authenticate: c_int,
    pub(crate) account: Option<c_int>,
    /// PAM's words for a code that is no answer (a broken configuration, an error).
    pub(crate) message: String,
}

/// One transaction of `service` (read from `confdir` instead of `/etc/pam.d` when given) for
/// `user`: authenticate with `password`, then check the account.
#[allow(unsafe_code)]
pub(crate) fn check(
    service: &CStr,
    confdir: Option<&CString>,
    user: &CStr,
    password: &CStr,
) -> Result<Codes, String> {
    let mut answers = Answers {
        user,
        password,
        password_given: false,
    };
    let conversation = PamConv {
        conv: converse,
        appdata_ptr: ptr::addr_of_mut!(answers).cast(),
    };
    let mut handle: *mut PamHandle = ptr::null_mut();
    // SAFETY: every pointer is valid for the call: the C strings outlive the transaction, and
    // `conversation` and the `answers` it points to live until pam_end below.
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
        return Err(format!("PAM could not start ({started})"));
    }
    // SAFETY: `handle` is the transaction pam_start opened, ended once by pam_end and used by
    // nothing after it.
    unsafe {
        let authenticate = pam_authenticate(handle, PAM_DISALLOW_NULL_AUTHTOK);
        let account = (authenticate == PAM_SUCCESS).then(|| pam_acct_mgmt(handle, 0));
        let last = account.unwrap_or(authenticate);
        let text = pam_strerror(handle, last);
        let message = if text.is_null() {
            String::new()
        } else {
            CStr::from_ptr(text).to_string_lossy().into_owned()
        };
        pam_end(handle, last);
        Ok(Codes {
            authenticate,
            account,
            message,
        })
    }
}
