//! The file service through the C ABI, for a home and an application the test makes. The service
//! takes the user's directories from the environment (`HOME`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`,
//! `XDG_CACHE_HOME`, `USER`), as a host's process has them.
//!
//! One test in a binary of its own: the environment is set before any thread reads it, and no
//! other test sees it. Set among the crate's own tests, the home was every test's there: the
//! painter's Vulkan device keeps Mesa's shader cache in `$XDG_CACHE_HOME`, and its writes landed in
//! the home as it was removed.

#![allow(unsafe_code)]

/// A reply's id, JSON and bytes, freed.
///
/// # Safety
/// `reply` is a live reply.
unsafe fn files_reply(
    reply: *mut oxyos_client_ffi::files::OxyOSFilesReply,
) -> (u64, String, Vec<u8>) {
    use oxyos_client_ffi::files::*;
    assert!(!reply.is_null());
    // SAFETY: a live reply, read before it is freed.
    unsafe {
        let mut length = 0;
        let json = oxyos_files_reply_json(reply, &mut length);
        let text = std::slice::from_raw_parts(json.cast::<u8>(), length + 1);
        assert_eq!(text[length], 0, "NUL-terminated");
        let text = String::from_utf8(text[..length].to_vec()).unwrap();
        let mut count = 0;
        let bytes = oxyos_files_reply_bytes(reply, &mut count);
        let bytes = std::slice::from_raw_parts(bytes, count).to_vec();
        let id = oxyos_files_reply_id(reply);
        oxyos_files_reply_free(reply);
        (id, text, bytes)
    }
}

/// The file service through the C ABI, for a home and an application the test makes.
#[test]
fn files_are_read_written_trashed_and_watched_through_the_c_abi() {
    use oxyos_client_ffi::files::*;
    use oxyos_client_ffi::OXYOS_ERROR;
    let home = std::env::temp_dir().join(format!("oxyos-ffi-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("Documents")).unwrap();
    std::fs::write(home.join("Documents/notes.txt"), "notes").unwrap();
    let home = std::fs::canonicalize(&home).unwrap();
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));
    std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));
    std::env::set_var("XDG_CACHE_HOME", home.join(".cache"));
    std::env::set_var("USER", "ffi-test");
    let id = "so.oxy.ffi";
    // SAFETY: a live service, and requests and buffers that outlive each call.
    unsafe {
        assert!(oxyos_files_new("../x".as_ptr().cast(), 4, std::ptr::null(), 0).is_null());
        let files = oxyos_files_new(id.as_ptr().cast(), id.len(), std::ptr::null(), 0);
        assert!(!files.is_null());
        assert!(home.join(".local/share/oxyos/apps/so.oxy.ffi").is_dir());
        assert!(home.join(".cache/oxyos/apps/so.oxy.ffi").is_dir());
        let fd = oxyos_files_fd(files);
        assert!(fd >= 0);
        let call = |json: String, bytes: &[u8]| {
            files_reply(oxyos_files_call(
                files,
                json.as_ptr().cast(),
                json.len(),
                bytes.as_ptr(),
                bytes.len(),
            ))
        };
        let ask = |id: u64, json: String| {
            assert_eq!(
                oxyos_files_request(
                    files,
                    id,
                    json.as_ptr().cast(),
                    json.len(),
                    std::ptr::null(),
                    0
                ),
                0
            );
        };
        // A reply, waited for on the descriptor as the host waits.
        let take = || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let reply = oxyos_files_take(files);
                if !reply.is_null() {
                    return files_reply(reply);
                }
                assert!(std::time::Instant::now() < deadline, "no reply");
                let mut ready = [rustix::event::PollFd::from_borrowed_fd(
                    std::os::fd::BorrowedFd::borrow_raw(fd),
                    rustix::event::PollFlags::IN,
                )];
                let wait = rustix::event::Timespec {
                    tv_sec: 0,
                    tv_nsec: 50_000_000,
                };
                let _ = rustix::event::poll(&mut ready, Some(&wait));
            }
        };
        let documents = format!(
            "file://{}/",
            home.join(".local/share/oxyos/apps/so.oxy.ffi").display()
        );
        let (_, paths, _) = call(r#"{"op":"paths"}"#.into(), &[]);
        assert!(
            paths.contains(&format!(r#""documentDirectory":"{documents}""#)),
            "{paths}"
        );
        // Bytes in and out, at once.
        let data = format!("{documents}data.bin");
        let (_, written, _) = call(
            format!(r#"{{"op":"write","uri":"{data}","encoding":"bytes"}}"#),
            &[0, 1, 2, 255],
        );
        assert_eq!(written, r#"{"ok":true,"value":null}"#);
        let (id0, read, bytes) = call(
            format!(r#"{{"op":"read","uri":"{data}","encoding":"bytes"}}"#),
            &[],
        );
        assert_eq!(
            (id0, read.as_str(), bytes),
            (0, r#"{"ok":true,"value":null}"#, vec![0, 1, 2, 255])
        );
        let (_, refused, _) = call(r#"{"op":"read","uri":"file:///etc/passwd"}"#.into(), &[]);
        assert!(refused.contains(r#""error":"refused""#), "{refused}");
        // In order, through the queue, by id.
        let notes = format!("file://{}/Documents/notes.txt", home.display());
        ask(7, r#"{"op":"watchTrash"}"#.into());
        let (seven, watch, _) = take();
        assert_eq!(seven, 7);
        assert!(watch.contains(r#""value":1"#), "{watch}");
        ask(8, format!(r#"{{"op":"trash","uris":["{notes}"]}}"#));
        let mut heard = [take(), take()];
        heard.sort();
        assert_eq!(heard[0].0, 0);
        assert_eq!(heard[0].1, r#"{"watch":1}"#);
        assert_eq!(heard[1].0, 8);
        assert!(heard[1].1.contains(r#""ok":true"#), "{heard:?}");
        assert!(home.join(".local/share/Trash/files/notes.txt").exists());
        assert!(
            oxyos_files_call(files, [0xffu8].as_ptr().cast(), 1, std::ptr::null(), 0).is_null()
        );
        assert_eq!(
            oxyos_files_request(files, 9, [0xffu8].as_ptr().cast(), 1, std::ptr::null(), 0),
            OXYOS_ERROR
        );
        assert_eq!(
            oxyos_files_request(files, 0, "{}".as_ptr().cast(), 2, std::ptr::null(), 0),
            OXYOS_ERROR
        );
        assert!(oxyos_files_take(files).is_null());
        // A file opened for a request's body: its descriptor and size; nothing outside.
        let mut size = 0;
        let fd = oxyos_files_open_read(files, data.as_ptr().cast(), data.len(), &mut size);
        assert!(fd >= 0 && size == 4);
        let mut read = Vec::new();
        std::io::Read::read_to_end(
            &mut std::fs::File::from(
                <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd),
            ),
            &mut read,
        )
        .unwrap();
        assert_eq!(read, [0, 1, 2, 255]);
        let outside = "file:///etc/passwd";
        assert_eq!(
            oxyos_files_open_read(files, outside.as_ptr().cast(), outside.len(), &mut size),
            OXYOS_ERROR
        );
        // Tags (docs/adr/0049-file-tags.mdx): the index started by a watch and asked, an item
        // tagged, read, found and its change heard; where the temporary directory keeps no
        // `user.` attributes, the change says so and the rest is skipped.
        std::fs::write(home.join("Documents/report.txt"), "report").unwrap();
        let report = format!("file://{}/Documents/report.txt", home.display());
        let folder = format!("file://{}/Documents/", home.display());
        // A reply with this id, the events taken meanwhile kept.
        let mut events = Vec::new();
        let reply_to = |id: u64, events: &mut Vec<String>| loop {
            let (got, json, _) = take();
            if got == id {
                return json;
            }
            events.push(json);
        };
        ask(10, r#"{"op":"watchTags"}"#.into());
        let watch = reply_to(10, &mut events);
        let watch: u64 = serde_json::from_str::<serde_json::Value>(&watch).unwrap()["value"]
            .as_u64()
            .expect("a watch");
        ask(11, r#"{"op":"tagNames"}"#.into());
        assert_eq!(reply_to(11, &mut events), r#"{"ok":true,"value":[]}"#);
        let (_, called, _) = call(r#"{"op":"tagged","name":"Work"}"#.into(), &[]);
        assert!(called.contains(r#""error":"invalid""#), "{called}");
        let (_, invalid, _) = call(
            format!(r#"{{"op":"changeTags","uris":["{report}"],"add":[{{"name":"a,b"}}]}}"#),
            &[],
        );
        assert!(invalid.contains(r#""error":"invalid""#), "{invalid}");
        let (_, changed, _) = call(
            format!(
                r#"{{"op":"changeTags","uris":["{report}","file:///etc/passwd"],"add":[{{"name":"Work","color":"red"}},{{"name":"Q3"}}],"remove":[]}}"#
            ),
            &[],
        );
        if changed.contains(r#""error":"unsupported""#) {
            eprintln!("skipped: the temporary directory keeps no user. extended attributes");
        } else {
            assert_eq!(
                changed,
                format!(
                    r#"{{"ok":true,"value":[{{"ok":true,"result":"{report}","uri":"{report}"}},{{"error":"refused","message":"refused: /etc/passwd is outside the home, the mounted volumes and the application's folders","ok":false,"uri":"file:///etc/passwd"}}]}}"#
                )
            );
            // No Mac's attribute on it: the names alone are kept.
            let tags = r#"[{"color":null,"name":"Work"},{"color":null,"name":"Q3"}]"#;
            let (_, held, _) = call(format!(r#"{{"op":"tagsIn","uri":"{folder}"}}"#), &[]);
            assert_eq!(
                held,
                format!(r#"{{"ok":true,"value":{{"{report}":{tags}}}}}"#)
            );
            let (_, of, _) = call(
                format!(r#"{{"op":"tagsOf","uris":["{report}","{folder}gone.txt"]}}"#),
                &[],
            );
            assert_eq!(of, format!(r#"{{"ok":true,"value":[{tags},null]}}"#));
            // The index heard it.
            let heard = format!(r#"{{"watch":{watch}}}"#);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !events.contains(&heard) {
                assert!(std::time::Instant::now() < deadline, "{events:?}");
                let (id, json, _) = take();
                assert_eq!(id, 0);
                events.push(json);
            }
            ask(12, r#"{"op":"tagged","name":"work"}"#.into());
            let found = reply_to(12, &mut events);
            assert!(
                found.contains(&format!(r#""uri":"{report}""#))
                    && found.contains(r#""isDirectory":false"#)
                    && found.contains(r#""size":6"#)
                    && found.contains(tags),
                "{found}"
            );
            ask(13, r#"{"op":"tagNames"}"#.into());
            assert_eq!(
                reply_to(13, &mut events),
                r#"{"ok":true,"value":[{"count":1,"name":"Q3"},{"count":1,"name":"Work"}]}"#
            );
            // In the Trash, an item's tags come with it.
            let (_, trashed, _) = call(format!(r#"{{"op":"trash","uris":["{report}"]}}"#), &[]);
            assert!(trashed.contains(r#""ok":true"#), "{trashed}");
            let (_, held, _) = call(r#"{"op":"trashed"}"#.into(), &[]);
            assert!(held.contains(&format!(r#""tags":{tags}"#)), "{held}");
        }
        let (_, unwatched, _) = call(format!(r#"{{"op":"unwatch","watch":{watch}}}"#), &[]);
        assert_eq!(unwatched, r#"{"ok":true,"value":null}"#);
        oxyos_files_free(files);
    }
    std::fs::remove_dir_all(home).unwrap();
}
