use super::*;

/// A data directory for one test, removed when dropped.
struct Directory(PathBuf);

impl Directory {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("oxyos-app-storage-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Self(dir.join("so.oxy.stash"))
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.0.parent().expect("parent"));
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn pairs(values: &[(&str, &str)]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn what_is_set_is_kept_in_the_file_and_read_back_by_another_process() {
    let dir = Directory::new("kept");
    let storage = Storage::new(&dir.0);
    // Reading what was never kept creates nothing.
    assert_eq!(storage.get(&strings(&["a"])).expect("get"), vec![None]);
    assert!(!dir.0.exists());
    storage
        .set(&pairs(&[("a", "1"), ("b", "two"), ("a", "3")]))
        .expect("set");
    assert_eq!(
        storage.get(&strings(&["a", "b", "c"])).expect("get"),
        vec![Some("3".into()), Some("two".into()), None]
    );
    assert_eq!(storage.keys().expect("keys"), strings(&["a", "b"]));
    let text = fs::read_to_string(dir.0.join(FILE)).expect("file");
    assert_eq!(
        serde_json::from_str::<Value>(&text).expect("json"),
        serde_json::json!({"a": "3", "b": "two"})
    );
    // Private to the user: the directory and the file.
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&dir.0).expect("dir").permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(dir.0.join(FILE))
            .expect("file")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    // Nothing staged is left beside it.
    let names: Vec<_> = fs::read_dir(&dir.0)
        .expect("list")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .into_string()
                .expect("name")
        })
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");

    // Another process's change (the file replaced) is read again, never overwritten with old data.
    fs::write(dir.0.join("other"), r#"{"a":"from another","z":"z"}"#).expect("write");
    fs::rename(dir.0.join("other"), dir.0.join(FILE)).expect("rename");
    assert_eq!(
        storage.get(&strings(&["a"])).expect("get"),
        vec![Some("from another".into())]
    );
    storage.set(&pairs(&[("b", "mine")])).expect("set");
    assert_eq!(storage.keys().expect("keys"), strings(&["a", "b", "z"]));
}

#[test]
fn remove_and_clear_forget_what_they_name() {
    let dir = Directory::new("remove");
    let storage = Storage::new(&dir.0);
    storage
        .set(&pairs(&[("a", "1"), ("b", "2"), ("c", "3")]))
        .expect("set");
    storage.remove(&strings(&["a", "missing"])).expect("remove");
    assert_eq!(storage.keys().expect("keys"), strings(&["b", "c"]));
    storage.clear().expect("clear");
    assert_eq!(storage.keys().expect("keys"), Vec::<String>::new());
    assert_eq!(fs::read_to_string(dir.0.join(FILE)).expect("file"), "{}");
}

#[test]
fn merge_deep_merges_json_objects_as_android_does() {
    let dir = Directory::new("merge");
    let storage = Storage::new(&dir.0);
    storage
        .set(&pairs(&[(
            "user",
            r#"{"name":"Ada","prefs":{"dark":true,"size":1},"tags":[1]}"#,
        )]))
        .expect("set");
    storage
        .merge(&pairs(&[
            ("user", r#"{"prefs":{"size":2},"tags":[2]}"#),
            ("new", r#"{"x":1}"#),
            ("user", r#"{"name":"Grace"}"#),
        ]))
        .expect("merge");
    let got = storage.get(&strings(&["user", "new"])).expect("get");
    let value = |text: &Option<String>| {
        serde_json::from_str::<Value>(text.as_deref().expect("kept")).expect("json")
    };
    assert_eq!(
        value(&got[0]),
        serde_json::json!({"name": "Grace", "prefs": {"dark": true, "size": 2}, "tags": [2]})
    );
    assert_eq!(value(&got[1]), serde_json::json!({"x": 1}));
    // A value that is not an object fails the whole merge, and changes nothing.
    storage.set(&pairs(&[("text", "plain")])).expect("set");
    let before = storage.get(&strings(&["user", "text"])).expect("get");
    assert!(matches!(
        storage.merge(&pairs(&[
            ("user", r#"{"name":"X"}"#),
            ("text", r#"{"a":1}"#)
        ])),
        Err(Error::Merge(_))
    ));
    assert!(matches!(
        storage.merge(&pairs(&[("user", "[1]")])),
        Err(Error::Merge(_))
    ));
    assert_eq!(
        storage.get(&strings(&["user", "text"])).expect("get"),
        before
    );
}

#[test]
fn a_file_that_is_not_the_storage_is_never_overwritten() {
    let dir = Directory::new("corrupt");
    fs::create_dir_all(&dir.0).expect("dir");
    fs::write(dir.0.join(FILE), "not json").expect("write");
    let storage = Storage::new(&dir.0);
    assert!(matches!(
        storage.get(&strings(&["a"])),
        Err(Error::Corrupt(_))
    ));
    assert!(matches!(
        storage.set(&pairs(&[("a", "1")])),
        Err(Error::Corrupt(_))
    ));
    assert_eq!(
        fs::read_to_string(dir.0.join(FILE)).expect("file"),
        "not json"
    );
    fs::write(dir.0.join(FILE), r#"{"a":1}"#).expect("write");
    assert!(matches!(storage.keys(), Err(Error::Corrupt(_))));
}

#[test]
fn writers_in_parallel_lose_nothing() {
    let dir = Directory::new("parallel");
    let threads: Vec<_> = (0..8)
        .map(|thread| {
            let path = dir.0.clone();
            std::thread::spawn(move || {
                let storage = Storage::new(path);
                for i in 0..20 {
                    storage
                        .set(&pairs(&[(&format!("{thread}-{i}"), "v")]))
                        .expect("set");
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("join");
    }
    assert_eq!(Storage::new(&dir.0).keys().expect("keys").len(), 160);
}
