//! Files over the network (docs/adr/0028-file-access.mdx): `expo-file-system`'s downloads into a
//! file and uploads of one, as its Android implementation makes them (`FileSystemDownload`,
//! `FileSystemDownloadTask`, `FileSystemUploadTask`), through `oxyos-net`, reading and writing only
//! what the scope allows.
//!
//! - **A download** asks first, then writes: a response that is not a success (2xx) fails with its
//!   status and makes no file. Into a folder, the file is named as Android's `URLUtil` names it:
//!   the response's `Content-Disposition`, else the URL's last segment, else `downloadfile`, with
//!   the extension of its type when it has none. A file already there is refused unless the
//!   download may replace it. The body is written into the file as it arrives (a download that
//!   fails part way leaves what arrived, as Android's does), its progress said as it goes. A
//!   paused download stops and keeps what it wrote: it resumes from there with `Range`, and a
//!   server that answers the whole file again (200, not 206) writes it again from the start.
//! - **An upload** sends the file as it is read, never wholly in memory: as the body itself, or in
//!   a `multipart/form-data` body with the parameters, its length always known, its progress said
//!   as it goes. Its answer is the response's status, headers and body, whatever the status.
//! - **Cancelled**, either stops at its next step and fails as cancelled.

use crate::scope::Access;
use crate::{mime, uri, Error, Scope};
use oxyos_net::{Answer, Body, Network, Part, Request};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How often progress is said at most, as Android throttles it.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// A transfer while it runs: what stops it, and where a download writes.
#[derive(Default)]
pub struct Live {
    pub cancel: Arc<AtomicBool>,
    paused: AtomicBool,
    /// The file a download writes, once it knows it.
    destination: Mutex<Option<PathBuf>>,
}

impl Live {
    /// Stops the transfer at its next step; it fails as cancelled.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Stops a download at its next step, keeping what it wrote; what it can resume from: the
    /// bytes in its file (none before it wrote any).
    pub fn pause(&self) -> Option<u64> {
        self.paused.store(true, Ordering::Relaxed);
        self.cancel.store(true, Ordering::Relaxed);
        let destination = self.destination.lock().expect("lock").clone()?;
        std::fs::metadata(destination)
            .ok()
            .map(|metadata| metadata.len())
            .filter(|length| *length > 0)
    }
}

/// A request's headers, from `{"headers": {NAME: VALUE}}`.
fn headers(request: &Value) -> Vec<(String, String)> {
    request
        .get("headers")
        .and_then(Value::as_object)
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(name, value)| {
                    value.as_str().map(|value| (name.clone(), value.into()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn text<'a>(request: &'a Value, key: &str) -> Result<&'a str, Error> {
    request
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Invalid(format!("`{key}` is missing")))
}

fn flag(request: &Value, key: &str) -> bool {
    request.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(given, _)| given.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// The name a downloaded file takes, as Android's `URLUtil.guessFileName` gives it: the
/// `Content-Disposition`'s (`filename*` in UTF-8 first), else the URL's last segment, else
/// `downloadfile`; with the extension of `content_type` (`.bin` when unknown) if it has none.
pub fn file_name(url: &str, disposition: Option<&str>, content_type: Option<&str>) -> String {
    let decode = |text: &str| {
        uri::path_of(&format!("file:///{text}"))
            .ok()
            .map(|path| path.trim_start_matches('/').to_string())
    };
    let from_disposition = disposition.and_then(|disposition| {
        let mut plain = None;
        let mut extended = None;
        for parameter in disposition.split(';').map(str::trim) {
            let Some((key, value)) = parameter.split_once('=') else {
                continue;
            };
            match key.trim().to_ascii_lowercase().as_str() {
                "filename*" => {
                    let value = value.trim().trim_matches('"');
                    if let Some(encoded) = value
                        .strip_prefix("UTF-8''")
                        .or_else(|| value.strip_prefix("utf-8''"))
                    {
                        extended = decode(encoded);
                    }
                }
                "filename" => plain = Some(value.trim().trim_matches('"').to_string()),
                _ => {}
            }
        }
        extended.or(plain)
    });
    let from_url = || {
        let path = url.split(['?', '#']).next().unwrap_or("");
        let path = path.split_once("://").map_or(path, |(_, rest)| {
            rest.find('/').map_or("", |slash| &rest[slash..])
        });
        path.rsplit('/')
            .next()
            .filter(|segment| !segment.is_empty())
            .and_then(decode)
    };
    let name = from_disposition
        .or_else(from_url)
        .map(|name| {
            name.rsplit(['/', '\\'])
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .filter(|name| crate::valid_name(name).is_ok())
        .unwrap_or_else(|| "downloadfile".to_string());
    if name.rfind('.').is_some_and(|dot| dot > 0) {
        return name;
    }
    let extension = content_type.and_then(mime::extension_of).unwrap_or("bin");
    format!("{name}.{extension}")
}

/// What a download writes: its file, how much it wrote and of how much.
struct Writing {
    file: File,
    written: u64,
    total: Option<u64>,
    told: Option<Instant>,
}

/// Downloads `{url, to, toDirectory, headers, idempotent, resume}` into a file; its URI, or null
/// if it was paused. `progress` is told `{bytesWritten, totalBytes}` (-1 when unknown) as it goes.
pub fn download(
    network: &Network,
    scope: &Scope,
    live: &Live,
    request: &Value,
    progress: &dyn Fn(Value),
) -> Result<Value, Error> {
    let url = text(request, "url")?;
    let to = text(request, "to")?;
    let to_directory = flag(request, "toDirectory");
    let idempotent = flag(request, "idempotent");
    let resume = request.get("resume").and_then(Value::as_u64);
    let to_path = uri::path_of(to)?;
    let reached = scope.reach(&to_path, Access::Write)?;
    match std::fs::metadata(&reached) {
        Ok(found) if to_directory && !found.is_dir() => return Err(Error::NotAFolder),
        Ok(found) if !to_directory && found.is_dir() => return Err(Error::NotAFile),
        Ok(_) => {}
        Err(_) if to_directory => return Err(Error::NoDestination),
        Err(_) if !reached.parent().is_some_and(|parent| parent.is_dir()) => {
            return Err(Error::NoDestination)
        }
        Err(_) => {}
    }
    let mut asked = headers(request);
    if let Some(offset) = resume {
        asked.push(("Range".into(), format!("bytes={offset}-")));
    }
    let fetched = Request {
        method: "GET".into(),
        url: url.to_string(),
        headers: asked,
        incremental: true,
        cancel: live.cancel.clone(),
        ..Request::default()
    };
    let refused: RefCell<Option<Error>> = RefCell::new(None);
    let writing: RefCell<Option<Writing>> = RefCell::new(None);
    let made: RefCell<String> = RefCell::new(to.to_string());
    let tell = |writing: &mut Writing, last: bool| {
        if last
            || writing
                .told
                .is_none_or(|told| told.elapsed() >= PROGRESS_EVERY)
        {
            writing.told = Some(Instant::now());
            let total = writing
                .total
                .and_then(|total| i64::try_from(total).ok())
                .unwrap_or(-1);
            progress(json!({ "bytesWritten": writing.written, "totalBytes": total }));
        }
    };
    let fail = |error: Error| {
        let message = error.to_string();
        *refused.borrow_mut() = Some(error);
        Err(message)
    };
    let fetched = network.fetch(fetched, &|answer| match answer {
        Answer::Response {
            status, headers, ..
        } => {
            if !(200..300).contains(&status) {
                return fail(Error::Network(format!(
                    "Unable to download a file: response has status: {status}"
                )));
            }
            let partial = resume.is_some() && status == 206;
            let path = if to_directory {
                let name = file_name(
                    url,
                    header(&headers, "content-disposition"),
                    header(&headers, "content-type"),
                );
                *made.borrow_mut() = uri::child(to, &name, false);
                // Links followed: a link there is written through only if it stays in the scope.
                match scope.reach(&format!("{to_path}/{name}"), Access::Write) {
                    Ok(path) => path,
                    Err(error) => return fail(error),
                }
            } else {
                reached.clone()
            };
            if resume.is_none() && !idempotent && std::fs::symlink_metadata(&path).is_ok() {
                return fail(Error::Exists);
            }
            let opened = if partial {
                let offset = resume.unwrap_or(0);
                OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .and_then(|mut file| {
                        file.set_len(offset)?;
                        file.seek(SeekFrom::End(0))?;
                        Ok(file)
                    })
            } else {
                File::create(&path)
            };
            let file = match opened {
                Ok(file) => file,
                Err(error) => return fail(error.into()),
            };
            *live.destination.lock().expect("lock") = Some(path);
            let offset = if partial { resume.unwrap_or(0) } else { 0 };
            let length = header(&headers, "content-length").and_then(|n| n.parse::<u64>().ok());
            *writing.borrow_mut() = Some(Writing {
                file,
                written: offset,
                total: length.map(|length| length + offset),
                told: None,
            });
            Ok(())
        }
        Answer::Chunk { bytes, .. } => {
            let mut writing = writing.borrow_mut();
            let Some(writing) = writing.as_mut() else {
                return Ok(());
            };
            if let Err(error) = writing.file.write_all(&bytes) {
                *refused.borrow_mut() = Some(error.into());
                return Err("the file could not be written".into());
            }
            writing.written += bytes.len() as u64;
            tell(writing, false);
            Ok(())
        }
        _ => Ok(()),
    });
    if let Err(failure) = fetched {
        if live.paused.load(Ordering::Relaxed) {
            return Ok(Value::Null);
        }
        if let Some(error) = refused.into_inner() {
            return Err(error);
        }
        if live.cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        return Err(Error::Network(format!(
            "Unable to download a file: {}",
            failure.error
        )));
    }
    if let Some(writing) = writing.borrow_mut().as_mut() {
        writing.file.flush()?;
        // Its end said, at what it wrote.
        writing.total = Some(writing.written);
        tell(writing, true);
    }
    Ok(made.into_inner().into())
}

/// A multipart body's boundary no part is likely to contain: random, as OkHttp's.
fn boundary() -> String {
    let mut random = [0u8; 16];
    let filled = File::open("/dev/urandom").and_then(|mut source| {
        use std::io::Read;
        source.read_exact(&mut random)
    });
    if filled.is_err() {
        // A boundary that differs per upload is enough; the time gives one.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        random.copy_from_slice(&now.to_le_bytes());
    }
    random
        .iter()
        .fold(String::from("OxyOSFormBoundary"), |mut boundary, byte| {
            use std::fmt::Write;
            let _ = write!(boundary, "{byte:02x}");
            boundary
        })
}

/// A form field's name or file name as a `Content-Disposition` writes it (quotes and line ends
/// escaped, as OkHttp escapes them).
fn quoted(text: &str) -> String {
    text.replace('\n', "%0A")
        .replace('\r', "%0D")
        .replace('"', "%22")
}

/// Uploads a file: `{uri, url, method, headers, uploadType (0: the file as the body, 1: a
/// multipart form), fieldName, mimeType, parameters}`; `{body, status, headers}` whatever the
/// status. `progress` is told `{bytesSent, totalBytes}` as it goes.
pub fn upload(
    network: &Network,
    scope: &Scope,
    live: &Live,
    request: &Value,
    progress: &dyn Fn(Value),
) -> Result<Value, Error> {
    let file_uri = text(request, "uri")?;
    let path = scope.reach(&uri::path_of(file_uri)?, Access::Read)?;
    let found = std::fs::metadata(&path).map_err(|_| Error::NotFound)?;
    if found.is_dir() {
        return Err(Error::NotAFile);
    }
    let file = Arc::new(File::open(&path)?);
    let size = file.metadata()?.len();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file")
        .to_string();
    let mut sent_headers = headers(request);
    let multipart = request.get("uploadType").and_then(Value::as_u64) == Some(1);
    let body = if multipart {
        let boundary = boundary();
        let mut head = String::new();
        if let Some(parameters) = request.get("parameters").and_then(Value::as_object) {
            for (key, value) in parameters {
                let value = value
                    .as_str()
                    .map_or_else(|| value.to_string(), String::from);
                head.push_str(&format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\
                     Content-Length: {}\r\n\r\n{value}\r\n",
                    quoted(key),
                    value.len()
                ));
            }
        }
        let field = request
            .get("fieldName")
            .and_then(Value::as_str)
            .unwrap_or("file");
        let media = request
            .get("mimeType")
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| mime::of_name(&name).map(String::from))
            .unwrap_or_else(|| "application/octet-stream".into());
        head.push_str(&format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n\
             Content-Type: {media}\r\nContent-Length: {size}\r\n\r\n",
            quoted(field),
            quoted(&name)
        ));
        if header(&sent_headers, "content-type").is_none() {
            sent_headers.push((
                "Content-Type".into(),
                format!("multipart/form-data; boundary={boundary}"),
            ));
        }
        Body {
            parts: vec![
                Part::Bytes(head.into_bytes().into()),
                Part::File {
                    file,
                    offset: 0,
                    length: size,
                },
                Part::Bytes(format!("\r\n--{boundary}--\r\n").into_bytes().into()),
            ],
        }
    } else {
        Body {
            parts: vec![Part::File {
                file,
                offset: 0,
                length: size,
            }],
        }
    };
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("POST")
        .to_ascii_uppercase();
    let sent = Request {
        method,
        url: text(request, "url")?.to_string(),
        headers: sent_headers,
        body: Some(body),
        upload_progress: true,
        cancel: live.cancel.clone(),
        ..Request::default()
    };
    let answered: RefCell<(u16, Map<String, Value>, Vec<u8>)> = RefCell::new(Default::default());
    let fetched = network.fetch(sent, &|answer| {
        match answer {
            Answer::Sent { sent, total, .. } => {
                progress(json!({ "bytesSent": sent, "totalBytes": total }));
            }
            Answer::Response {
                status, headers, ..
            } => {
                let mut answered = answered.borrow_mut();
                answered.0 = status;
                answered.1 = headers
                    .into_iter()
                    .map(|(name, value)| (name, Value::from(value)))
                    .collect();
            }
            Answer::Data { bytes, .. } | Answer::Chunk { bytes, .. } => {
                answered.borrow_mut().2.extend_from_slice(&bytes);
            }
            Answer::Complete { .. } => {}
        }
        Ok(())
    });
    if let Err(failure) = fetched {
        if live.cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        return Err(Error::Network(format!(
            "Unable to upload a file: {}",
            failure.error
        )));
    }
    let (status, headers, body) = answered.into_inner();
    Ok(json!({
        "body": String::from_utf8_lossy(&body),
        "status": status,
        "headers": headers,
    }))
}
