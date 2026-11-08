//! The search service doing nothing while nothing happens (docs/performance.mdx's idle rule: no
//! thread wakes on a clock), in a process of its own so that no other test's threads are counted.

use oxyos_files::Scope;
use oxyos_search::service::{serve, Paths};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// Context switches of every thread of this process but the calling one.
fn switches() -> u64 {
    let me = rustix::thread::gettid().as_raw_nonzero().get().to_string();
    let mut total = 0;
    for task in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
        if task.file_name().to_string_lossy() == me {
            continue;
        }
        let status = std::fs::read_to_string(task.path().join("status")).unwrap_or_default();
        for line in status.lines() {
            if let Some(count) = line
                .strip_prefix("voluntary_ctxt_switches:")
                .or_else(|| line.strip_prefix("nonvoluntary_ctxt_switches:"))
            {
                total += count.trim().parse::<u64>().unwrap_or(0);
            }
        }
    }
    total
}

fn threads() -> Vec<String> {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .flatten()
        .map(|task| std::fs::read_to_string(task.path().join("comm")).unwrap_or_default())
        .map(|name| name.trim().to_string())
        .collect()
}

#[test]
fn nothing_wakes_while_nothing_happens() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    for n in 0..200 {
        let path = home.join(format!("notes/{}/{n}.txt", n % 10));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("note number {n}")).unwrap();
    }
    let mountinfo = dir.path().join("mountinfo");
    std::fs::write(&mountinfo, "").unwrap();
    let scope = Scope::new(&home, &home, &home, Vec::new(), mountinfo, 0).unwrap();
    let paths = Paths {
        cache: dir.path().join("cache"),
        data: dir.path().join("data"),
        config: dir.path().join("search.json"),
    };
    let (ours, theirs) = UnixStream::pair().unwrap();
    let service = std::thread::Builder::new()
        .name("search-serve".into())
        .spawn(move || serve(theirs, scope, Some(paths)))
        .unwrap();
    ours.set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let control = ours.try_clone().unwrap();
    let mut writer = ours.try_clone().unwrap();
    let mut reader = BufReader::new(ours);
    let mut serial = 0;
    let mut ask = |ask: Value| -> Value {
        serial += 1;
        let line = json!({"ask": {"client": 1, "serial": serial, "asker": {"shell": true, "settings": true}, "text": ask.to_string()}});
        writer.write_all(format!("{line}\n").as_bytes()).unwrap();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["answer"]["serial"] == serial {
                return serde_json::from_str(message["answer"]["text"].as_str().unwrap()).unwrap();
            }
        }
    };
    let settle = |ask: &mut dyn FnMut(Value) -> Value| {
        let start = Instant::now();
        loop {
            let status = ask(json!({"status": {}}));
            if status["status"]["indexing"] == false
                && status["status"]["rebuilding"] == false
                && status["status"]["items"].as_u64() >= Some(210)
            {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "never settled: {status}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let idle = || {
        std::thread::sleep(Duration::from_millis(300));
        let before = switches();
        std::thread::sleep(Duration::from_secs(4));
        let woke = switches() - before;
        assert_eq!(
            woke,
            0,
            "the service's threads woke {woke} times in 4 s idle: {:?}",
            threads()
        );
        // The tantivy writer is closed while idle: its threads are gone, the service's three
        // remain (answering, crawling, writing) beside the test harness's two.
        let named: Vec<String> = threads();
        assert_eq!(named.len(), 5, "{named:?}");
        assert!(named.contains(&"search-crawler".to_string()), "{named:?}");
        assert!(named.contains(&"search-writer".to_string()), "{named:?}");
    };
    settle(&mut ask);
    idle();
    // Rebuilt (docs/adr/0051-search.mdx, decision 14): once the new index answers, idle as before,
    // the old one's directory gone.
    assert_eq!(ask(json!({"rebuild": {}})), json!({"rebuilding": {}}));
    settle(&mut ask);
    idle();
    let mut cache: Vec<String> = std::fs::read_dir(dir.path().join("cache"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    cache.sort();
    assert_eq!(cache, ["index-1.1", "index-1.json"]);
    // The program closing its end ends the service.
    control.shutdown(std::net::Shutdown::Both).unwrap();
    service.join().unwrap().unwrap();
}
