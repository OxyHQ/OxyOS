use super::*;
use crate::{Loaded, Loader};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicUsize;

/// A 2 × 2 opaque PNG of one colour.
fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&[10, 20, 30, 255].repeat(4))
        .unwrap();
    bytes
}

/// A 2 × 1 GIF of three frames, red, green and blue, looping forever.
fn gif() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let palette = [255, 0, 0, 0, 255, 0, 0, 0, 255];
        let mut encoder = gif::Encoder::new(&mut bytes, 2, 1, &palette).unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
        for index in 0..3 {
            let mut frame = gif::Frame::from_indexed_pixels(2, 1, vec![index; 2], None);
            frame.delay = 5;
            encoder.write_frame(&frame).unwrap();
        }
    }
    bytes
}

/// A server of images: its base URL and how many requests it answered. `/slow.png` sends its
/// body a byte every 50 ms, and counts how many of its answers were cut short.
fn server() -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (asked, cut) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (counted, cuts) = (asked.clone(), cut.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (counted, cuts) = (counted.clone(), cuts.clone());
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let target = line.split_whitespace().nth(1).unwrap_or("");
                let path = target.split('?').next().unwrap_or("").to_string();
                let mut token = String::new();
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header.trim_end().is_empty() {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        if name.eq_ignore_ascii_case("x-token") {
                            token = value.trim().to_string();
                        }
                    }
                }
                counted.fetch_add(1, Ordering::SeqCst);
                let (status, kind, body) = match path.as_str() {
                    "/a.png" | "/slow.png" => ("200 OK", "image/png", png()),
                    "/anim.gif" => ("200 OK", "image/gif", gif()),
                    "/private.png" if token == "secret" => ("200 OK", "image/png", png()),
                    "/icon.svg" => (
                        "200 OK",
                        "image/svg+xml",
                        b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec(),
                    ),
                    _ => ("404 Not Found", "text/plain", b"missing".to_vec()),
                };
                let mut stream = stream;
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: max-age=600\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).unwrap();
                if path == "/slow.png" {
                    for byte in &body {
                        std::thread::sleep(Duration::from_millis(50));
                        if stream
                            .write_all(&[*byte])
                            .and_then(|()| stream.flush())
                            .is_err()
                        {
                            cuts.fetch_add(1, Ordering::SeqCst);
                            return;
                        }
                    }
                } else {
                    let _ = stream.write_all(&body);
                }
            });
        }
    });
    (format!("http://{address}"), asked, cut)
}

fn loader(cache: Option<&str>) -> Loader {
    let mut loader = Loader::new().unwrap();
    let cache = cache.map(|name| {
        let place =
            std::env::temp_dir().join(format!("oxyos-image-remote-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&place);
        Arc::new(Cache::open(&place, 1 << 20).unwrap())
    });
    loader.connect(Arc::new(Network::new().unwrap()), cache);
    loader
}

fn fetch(url: &str, want: Want) -> Fetch {
    Fetch {
        url: url.into(),
        headers: Vec::new(),
        policy: Policy::Default,
        memory: true,
        disk: true,
        want,
        key: None,
        animated: false,
    }
}

/// The answers until each of `ids` ended (an answer other than progress), in order.
fn answers(loader: &Loader, ids: &[u64]) -> Vec<Loaded> {
    let mut got: Vec<Loaded> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    let ended = |got: &[Loaded], id: u64| {
        got.iter()
            .any(|answer| answer.id == id && !matches!(answer.outcome, Outcome::Progress { .. }))
    };
    while !ids.iter().all(|id| ended(&got, *id)) {
        assert!(Instant::now() < deadline, "answered only {got:?}");
        match loader.answer() {
            Some(answer) => got.push(answer),
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    got
}

fn ended(answers: &[Loaded], id: u64) -> &Outcome {
    &answers
        .iter()
        .rfind(|answer| answer.id == id)
        .expect("answered")
        .outcome
}

#[test]
fn a_remote_image_is_fetched_with_its_progress_decoded_and_kept_in_memory() {
    let (base, asked, _) = server();
    let loader = loader(Some("memory"));
    let url = format!("{base}/a.png");
    loader.fetch(1, fetch(&url, Want::Picture(Fit::Whole)));
    let got = answers(&loader, &[1]);
    let length = png().len() as u64;
    assert!(
        matches!(got[0].outcome, Outcome::Progress { loaded, total } if loaded == length && total == length)
    );
    assert!(
        matches!(ended(&got, 1), Outcome::Image(image, Origin::Source) if image.pixels == [10, 20, 30, 255].repeat(4))
    );
    assert_eq!(loader.cached(&url), (true, true));
    // Again, at another size and for its size: from memory, the network not asked.
    loader.fetch(2, fetch(&url, Want::Picture(Fit::Within(1, 1))));
    loader.fetch(3, fetch(&url, Want::Size));
    let got = answers(&loader, &[2, 3]);
    assert!(matches!(ended(&got, 2), Outcome::Image(image, Origin::Memory) if image.width == 1));
    assert!(matches!(ended(&got, 3), Outcome::Size(2, 2)));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
}

#[test]
fn headers_are_sent_and_failures_and_svg_documents_are_answered() {
    let (base, _, _) = server();
    let loader = loader(None);
    let mut private = fetch(&format!("{base}/private.png"), Want::Picture(Fit::Whole));
    loader.fetch(1, private.clone());
    private.headers.push(("X-Token".into(), "secret".into()));
    loader.fetch(2, private);
    loader.fetch(
        3,
        fetch(&format!("{base}/icon.svg"), Want::Picture(Fit::Whole)),
    );
    let mut only = fetch(&format!("{base}/a.png"), Want::Picture(Fit::Whole));
    only.policy = Policy::OnlyIfCached;
    loader.fetch(4, only);
    let got = answers(&loader, &[1, 2, 3, 4]);
    assert!(
        matches!(ended(&got, 1), Outcome::Failed(Error::Fetch(error)) if error.ends_with("/private.png answered 404"))
    );
    assert!(matches!(ended(&got, 2), Outcome::Image(..)));
    assert!(
        matches!(ended(&got, 3), Outcome::Undecoded { content_type: Some(kind), .. } if kind == "image/svg+xml")
    );
    assert!(
        matches!(ended(&got, 4), Outcome::Failed(Error::Fetch(error)) if error.contains("only-if-cached"))
    );
}

#[test]
fn a_prefetch_fills_the_disk_cache_which_serves_only_if_cached() {
    let (base, asked, _) = server();
    let loader = loader(Some("prefetch"));
    let url = format!("{base}/a.png");
    // React Native's prefetch uses the disk cache alone, as Android's prefetchToDiskCache.
    let mut prefetch = fetch(&url, Want::Prefetch);
    prefetch.memory = false;
    loader.fetch(1, prefetch);
    assert!(matches!(
        ended(&answers(&loader, &[1]), 1),
        Outcome::Fetched
    ));
    assert_eq!(loader.cached(&url), (false, true));
    let mut cached = fetch(&url, Want::Picture(Fit::Whole));
    cached.policy = Policy::OnlyIfCached;
    loader.fetch(2, cached);
    assert!(matches!(
        ended(&answers(&loader, &[2]), 2),
        Outcome::Image(_, Origin::Disk)
    ));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
}

#[test]
fn loads_at_once_share_a_fetch_which_stops_once_none_waits() {
    let (base, asked, cut) = server();
    let loader = loader(None);
    let url = format!("{base}/slow.png");
    loader.fetch(1, fetch(&url, Want::Picture(Fit::Whole)));
    loader.fetch(2, fetch(&url, Want::Picture(Fit::Within(1, 1))));
    std::thread::sleep(Duration::from_millis(300));
    // One cancelled: the other still gets it, from the one request.
    loader.cancel(1);
    let got = answers(&loader, &[2]);
    assert!(got.iter().all(|answer| answer.id == 2));
    assert!(matches!(ended(&got, 2), Outcome::Image(image, _) if image.width == 1));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    // Both cancelled: the request stops, and nothing is answered.
    let fresh = format!("{base}/slow.png?again");
    loader.fetch(3, fetch(&fresh, Want::Picture(Fit::Whole)));
    std::thread::sleep(Duration::from_millis(300));
    loader.cancel(3);
    let deadline = Instant::now() + Duration::from_secs(10);
    while cut.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline, "the request went on");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(loader.answer().is_none());
}

#[test]
fn a_prefetch_fills_both_caches_when_it_uses_both() {
    let (base, asked, _) = server();
    let loader = loader(Some("prefetch-both"));
    let url = format!("{base}/a.png");
    // expo-image's prefetch with memory-disk.
    loader.fetch(1, fetch(&url, Want::Prefetch));
    assert!(matches!(
        ended(&answers(&loader, &[1]), 1),
        Outcome::Fetched
    ));
    assert_eq!(loader.cached(&url), (true, true));
    // Again: from memory, nothing asked.
    loader.fetch(2, fetch(&url, Want::Prefetch));
    loader.fetch(3, fetch(&url, Want::Picture(Fit::Whole)));
    let got = answers(&loader, &[2, 3]);
    assert!(matches!(ended(&got, 2), Outcome::Fetched));
    assert!(matches!(ended(&got, 3), Outcome::Image(_, Origin::Memory)));
    // A load using neither cache leaves both as they were and asks the network.
    let mut neither = fetch(&format!("{base}/a.png?none"), Want::Picture(Fit::Whole));
    (neither.memory, neither.disk) = (false, false);
    loader.fetch(4, neither);
    assert!(matches!(
        ended(&answers(&loader, &[4]), 4),
        Outcome::Image(_, Origin::Source)
    ));
    assert_eq!(loader.cached(&format!("{base}/a.png?none")), (false, false));
    assert_eq!(asked.load(Ordering::SeqCst), 2);
}

#[test]
fn a_cache_key_holds_an_image_in_both_caches_which_clear_empties() {
    let (base, asked, _) = server();
    let loader = loader(Some("key"));
    let keyed = |url: &str, memory: bool| Fetch {
        key: Some("avatar".into()),
        memory,
        ..fetch(url, Want::Picture(Fit::Whole))
    };
    // Two signed URLs of one image.
    loader.fetch(1, keyed(&format!("{base}/a.png?sig=1"), true));
    assert!(matches!(
        ended(&answers(&loader, &[1]), 1),
        Outcome::Image(_, Origin::Source)
    ));
    assert_eq!(loader.cached("avatar"), (true, true));
    assert_eq!(
        loader.cached(&format!("{base}/a.png?sig=1")),
        (false, false)
    );
    loader.fetch(2, keyed(&format!("{base}/a.png?sig=2"), true));
    loader.fetch(3, keyed(&format!("{base}/a.png?sig=3"), false));
    let got = answers(&loader, &[2, 3]);
    assert!(matches!(ended(&got, 2), Outcome::Image(_, Origin::Memory)));
    assert!(matches!(ended(&got, 3), Outcome::Image(_, Origin::Disk)));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    // Its bytes, exactly as fetched, in a file of their own.
    let path = loader.cache_path("avatar").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), png());
    assert_eq!(loader.cache_path(&format!("{base}/a.png?sig=1")), None);

    loader.clear(true, false);
    assert_eq!(loader.cached("avatar"), (false, true));
    loader.clear(false, true);
    assert_eq!(loader.cached("avatar"), (false, false));
    assert_eq!(loader.cache_path("avatar"), None);
    assert!(!path.exists());
}

#[test]
fn loads_sharing_a_fetch_each_get_the_frames_they_ask_for_from_where_it_came() {
    let (base, asked, _) = server();
    let loader = loader(None);
    let url = format!("{base}/anim.gif");
    let animated = Fetch {
        animated: true,
        ..fetch(&url, Want::Picture(Fit::Whole))
    };
    loader.fetch(1, animated.clone());
    loader.fetch(2, fetch(&url, Want::Picture(Fit::Whole)));
    let got = answers(&loader, &[1, 2]);
    let Outcome::Image(frames, Origin::Source) = ended(&got, 1) else {
        panic!("{got:?}");
    };
    assert_eq!((frames.frames(), frames.loops), (3, 0));
    assert_eq!(frames.delays, [50, 50, 50]);
    assert_eq!(frames.pixels.len(), 3 * 2 * 4);
    assert_eq!(&frames.pixels[8..12], [0, 255, 0, 255]);
    let Outcome::Image(still, Origin::Source) = ended(&got, 2) else {
        panic!("{got:?}");
    };
    assert_eq!((still.frames(), still.delays.as_slice()), (1, &[0][..]));
    assert_eq!(still.pixels, [255, 0, 0, 255].repeat(2));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    // From memory, animated as asked.
    loader.fetch(3, animated);
    assert!(matches!(
        ended(&answers(&loader, &[3]), 3),
        Outcome::Image(image, Origin::Memory) if image.frames() == 3
    ));
}
