use super::*;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

/// What the test server was asked: each request's path, and its `If-None-Match` and
/// `If-Modified-Since`.
type Asked = Arc<Mutex<Vec<(String, Option<String>, Option<String>)>>>;

/// A server whose routes answer with caching headers; its base URL and what it was asked.
fn server() -> (String, Asked) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let asked: Asked = Arc::default();
    let log = asked.clone();
    let version = Arc::new(AtomicUsize::new(1));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let log = log.clone();
            let version = version.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let (mut tag, mut since) = (None, None);
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    let (name, value) = header.split_once(':').unwrap();
                    match name.to_ascii_lowercase().as_str() {
                        "if-none-match" => tag = Some(value.trim().to_string()),
                        "if-modified-since" => since = Some(value.trim().to_string()),
                        _ => {}
                    }
                }
                log.lock()
                    .unwrap()
                    .push((path.clone(), tag.clone(), since.clone()));
                let now = jiff::Timestamp::now()
                    .strftime("%a, %d %b %Y %H:%M:%S GMT")
                    .to_string();
                let current = format!("\"v{}\"", version.load(Ordering::SeqCst));
                let (status, headers, body): (&str, String, String) = match path.as_str() {
                    "/fresh" => ("200 OK", "Cache-Control: max-age=600\r\n".into(), "fresh".into()),
                    "/etag" if tag.as_deref() == Some(current.as_str()) => {
                        ("304 Not Modified", format!("ETag: {current}\r\nX-Seen: again\r\n"), String::new())
                    }
                    "/etag" => (
                        "200 OK",
                        format!("Cache-Control: no-cache\r\nETag: {current}\r\nX-Seen: first\r\n"),
                        format!("body {current}"),
                    ),
                    "/change" => {
                        version.fetch_add(1, Ordering::SeqCst);
                        ("200 OK", String::new(), "changed".into())
                    }
                    "/modified" if since.as_deref() == Some("Mon, 02 Nov 2026 10:00:00 GMT") => {
                        ("304 Not Modified", String::new(), String::new())
                    }
                    "/modified" => (
                        "200 OK",
                        "Cache-Control: max-age=0\r\nLast-Modified: Mon, 02 Nov 2026 10:00:00 GMT\r\n".into(),
                        "modified".into(),
                    ),
                    "/heuristic" => (
                        "200 OK",
                        "Last-Modified: Mon, 02 Nov 2026 10:00:00 GMT\r\n".into(),
                        "heuristic".into(),
                    ),
                    "/never" => ("200 OK", "Cache-Control: no-store\r\n".into(), "never".into()),
                    "/missing" => ("404 Not Found", String::new(), "missing".into()),
                    "/vary" => ("200 OK", "Cache-Control: max-age=600\r\nVary: Accept\r\n".into(), "varied".into()),
                    _ => ("404 Not Found", String::new(), String::new()),
                };
                let mut stream = stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nDate: {now}\r\nContent-Type: image/png\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    (format!("http://{address}"), asked)
}

fn get(id: i64, url: &str) -> Request {
    Request {
        id,
        method: "GET".into(),
        url: url.into(),
        ..Request::default()
    }
}

/// A request through the cache: its source, status, a header's value and its body.
fn fetched(
    network: &Network,
    cache: &Cache,
    request: Request,
    policy: Policy,
) -> Result<(Source, u16, Option<String>, String), Failure> {
    let got = RefCell::new((0u16, None, Vec::new()));
    let source = network.fetch_cached(request, Some(cache), policy, &|answer| {
        let mut got = got.borrow_mut();
        match answer {
            Answer::Response {
                status, headers, ..
            } => {
                got.0 = status;
                got.1 = header(&headers, "x-seen").map(str::to_string);
            }
            Answer::Data { bytes, .. } | Answer::Chunk { bytes, .. } => got.2.extend(bytes),
            _ => {}
        }
        Ok(())
    })?;
    let (status, seen, body) = got.into_inner();
    Ok((source, status, seen, String::from_utf8(body).unwrap()))
}

fn directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("oxyos-net-cache-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    directory
}

#[test]
fn a_fresh_response_is_served_from_the_cache_without_the_network() {
    let (base, asked) = server();
    let network = Network::new().unwrap();
    let cache = Cache::open(&directory("fresh"), 1 << 20).unwrap();
    let url = format!("{base}/fresh");
    let first = fetched(&network, &cache, get(1, &url), Policy::Default).unwrap();
    assert_eq!(first, (Source::Network, 200, None, "fresh".into()));
    assert!(cache.contains(&url));
    let second = fetched(&network, &cache, get(2, &url), Policy::Default).unwrap();
    assert_eq!(second, (Source::Cache, 200, None, "fresh".into()));
    assert_eq!(asked.lock().unwrap().len(), 1);
    // Reload asks the network whatever the cache holds.
    let reloaded = fetched(&network, &cache, get(3, &url), Policy::Reload).unwrap();
    assert_eq!(reloaded.0, Source::Network);
    assert_eq!(asked.lock().unwrap().len(), 2);
    // So does a request saying no-cache, conditionally: max-age has no validator, so wholly.
    let mut request = get(4, &url);
    request
        .headers
        .push(("Cache-Control".into(), "no-cache".into()));
    assert_eq!(
        fetched(&network, &cache, request, Policy::Default)
            .unwrap()
            .0,
        Source::Network
    );
    assert_eq!(asked.lock().unwrap().len(), 3);
}

#[test]
fn a_stale_response_is_revalidated_by_its_etag_and_last_modified() {
    let (base, asked) = server();
    let network = Network::new().unwrap();
    let cache = Cache::open(&directory("etag"), 1 << 20).unwrap();
    let url = format!("{base}/etag");
    let first = fetched(&network, &cache, get(1, &url), Policy::Default).unwrap();
    assert_eq!(
        first,
        (
            Source::Network,
            200,
            Some("first".into()),
            "body \"v1\"".into()
        )
    );
    // no-cache: asked again with its tag; the 304 refreshes its headers and serves its body.
    let second = fetched(&network, &cache, get(2, &url), Policy::Default).unwrap();
    assert_eq!(
        second,
        (
            Source::Revalidated,
            200,
            Some("again".into()),
            "body \"v1\"".into()
        )
    );
    assert_eq!(asked.lock().unwrap()[1].1.as_deref(), Some("\"v1\""));
    // Changed on the server: the new one replaces it.
    fetched(
        &network,
        &cache,
        get(3, &format!("{base}/change")),
        Policy::Default,
    )
    .unwrap();
    let third = fetched(&network, &cache, get(4, &url), Policy::Default).unwrap();
    assert_eq!(
        third,
        (
            Source::Network,
            200,
            Some("first".into()),
            "body \"v2\"".into()
        )
    );
    assert_eq!(cache.get(&url, &[]).unwrap().body, b"body \"v2\"");

    let modified = format!("{base}/modified");
    fetched(&network, &cache, get(5, &modified), Policy::Default).unwrap();
    let again = fetched(&network, &cache, get(6, &modified), Policy::Default).unwrap();
    assert_eq!(again, (Source::Revalidated, 200, None, "modified".into()));
    let asked = asked.lock().unwrap();
    assert_eq!(
        asked.last().unwrap().2.as_deref(),
        Some("Mon, 02 Nov 2026 10:00:00 GMT")
    );
}

#[test]
fn force_cache_and_only_if_cached_use_what_is_stored_however_old_and_offline() {
    let (base, asked) = server();
    let network = Network::new().unwrap();
    let cache = Cache::open(&directory("force"), 1 << 20).unwrap();
    let url = format!("{base}/etag");
    let missing = fetched(&network, &cache, get(1, &url), Policy::OnlyIfCached).unwrap_err();
    assert_eq!(
        missing.error,
        format!("{url} is not in the cache (only-if-cached)")
    );
    assert!(asked.lock().unwrap().is_empty());
    fetched(&network, &cache, get(2, &url), Policy::ForceCache).unwrap();
    // Stale (no-cache), and the server gone: what is stored is used all the same.
    // A port nothing listens on any more: connecting is refused at once.
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let offline = url.replace(&base, &format!("http://{closed}"));
    let entry = cache.get(&url, &[]).unwrap();
    cache
        .put(
            &offline,
            &Entry {
                url: offline.clone(),
                ..entry
            },
        )
        .unwrap();
    for policy in [Policy::ForceCache, Policy::OnlyIfCached] {
        let served = fetched(&network, &cache, get(3, &offline), policy).unwrap();
        assert_eq!(
            served,
            (
                Source::Cache,
                200,
                Some("first".into()),
                "body \"v1\"".into()
            )
        );
    }
    // By HTTP's rules it must be asked again, and the network is not there.
    assert!(fetched(&network, &cache, get(4, &offline), Policy::Default).is_err());
}

#[test]
fn what_may_not_be_stored_is_not() {
    let (base, _) = server();
    let network = Network::new().unwrap();
    let cache = Cache::open(&directory("store"), 1 << 20).unwrap();
    for path in ["/never", "/missing"] {
        let url = format!("{base}{path}");
        fetched(&network, &cache, get(1, &url), Policy::Default).unwrap();
        assert!(!cache.contains(&url), "{path} was stored");
    }
    // A request saying no-store neither reads nor writes it.
    let mut request = get(2, &format!("{base}/fresh"));
    request
        .headers
        .push(("Cache-Control".into(), "no-store".into()));
    fetched(&network, &cache, request, Policy::Default).unwrap();
    assert!(!cache.contains(&format!("{base}/fresh")));
    // A response varying by a header serves only requests with its value.
    let url = format!("{base}/vary");
    let mut webp = get(3, &url);
    webp.headers.push(("Accept".into(), "image/webp".into()));
    fetched(&network, &cache, webp.clone(), Policy::Default).unwrap();
    assert_eq!(
        fetched(&network, &cache, webp, Policy::Default).unwrap().0,
        Source::Cache
    );
    assert!(cache.get(&url, &[]).is_none());
}

#[test]
fn freshness_follows_max_age_expires_and_the_heuristic() {
    let received = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let entry = |headers: &[(&str, &str)], url: &str| Entry {
        url: url.into(),
        status: 200,
        headers: headers
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect(),
        varied: Vec::new(),
        requested: received - Duration::from_secs(2),
        received,
        body: Vec::new(),
    };
    let date = "Fri, 15 Jan 2027 08:00:00 GMT"; // 1_800_000_000 - 3200 seconds
    let aged = entry(&[("Cache-Control", "max-age=100"), ("Age", "30")], "u");
    // Its age counts what it said and the two seconds it took.
    assert_eq!(aged.age(received), Duration::from_secs(32));
    assert!(aged.fresh(received + Duration::from_secs(67)));
    assert!(!aged.fresh(received + Duration::from_secs(68)));
    let expires = entry(
        &[("Date", date), ("Expires", "Fri, 15 Jan 2027 09:00:00 GMT")],
        "u",
    );
    assert_eq!(expires.lifetime(), Duration::from_secs(3600));
    assert!(!entry(&[("Expires", "0")], "u").fresh(received));
    let heuristic = [
        ("Date", date),
        ("Last-Modified", "Fri, 15 Jan 2027 07:00:00 GMT"),
    ];
    assert_eq!(entry(&heuristic, "u").lifetime(), Duration::from_secs(360));
    assert_eq!(entry(&heuristic, "u?q=1").lifetime(), Duration::ZERO);
    assert_eq!(
        entry(&[("Cache-Control", "no-cache, max-age=60")], "u").lifetime(),
        Duration::ZERO
    );
}

#[test]
fn the_cache_is_bounded_least_recently_used_first_and_kept_on_disk() {
    let place = directory("lru");
    let entry = |url: &str| Entry {
        url: url.into(),
        status: 200,
        headers: vec![("Cache-Control".into(), "max-age=60".into())],
        varied: vec![("accept".into(), None)],
        requested: UNIX_EPOCH + Duration::from_secs(1),
        received: UNIX_EPOCH + Duration::from_secs(2),
        body: vec![7; 1000],
    };
    let size = entry("https://a/1").encode().len() as u64;
    let cache = Cache::open(&place, size * 5 + 10).unwrap();
    for n in 1..=5 {
        let url = format!("https://a/{n}");
        cache.put(&url, &entry(&url)).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    // The first used again: the second is the least recently used, and goes for the sixth.
    assert_eq!(cache.get("https://a/1", &[]), Some(entry("https://a/1")));
    std::thread::sleep(Duration::from_millis(20));
    cache.put("https://a/6", &entry("https://a/6")).unwrap();
    assert!(!cache.contains("https://a/2"));
    for n in [1, 3, 4, 5, 6] {
        assert!(cache.contains(&format!("https://a/{n}")), "{n} was evicted");
    }
    assert_eq!(cache.size(), size * 5);
    // Opened again, by another process say, smaller: the same order of use (the third goes).
    drop(cache);
    let again = Cache::open(&place, size * 4 + 10).unwrap();
    assert!(!again.contains("https://a/3"));
    for n in [1, 4, 5, 6] {
        assert!(again.contains(&format!("https://a/{n}")), "{n} was evicted");
    }
    // A damaged entry is forgotten; one too large for a quarter of the cache is not stored.
    fs::write(place.join(name("https://a/1")), b"damaged").unwrap();
    assert_eq!(again.get("https://a/1", &[]), None);
    assert!(!again.contains("https://a/1"));
    let mut large = entry("https://a/large");
    large.body = vec![0; size as usize * 2];
    again.put("https://a/large", &large).unwrap();
    assert!(!again.contains("https://a/large"));
    again.clear();
    assert_eq!(again.size(), 0);
    assert_eq!(fs::read_dir(&place).unwrap().count(), 0);
    let _ = fs::remove_dir_all(&place);
}
