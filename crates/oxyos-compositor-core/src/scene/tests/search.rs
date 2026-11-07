//! Search (docs/adr/0051-search.mdx): any client asks, in chunks, its ask handed whole to the
//! program; the answers reach their asker alone, and only one that speaks search; the program is
//! told the askers that went.

use super::*;

fn ask(h: &mut Harness, client: ClientId, serial: u32, text: &str) -> Result<(), ErrorCode> {
    let chunks = TextChunk::split(text);
    let count = chunks.len();
    for (index, chunk) in chunks.iter().enumerate() {
        h.request(
            client,
            Request::SearchRequest {
                serial,
                index: index as u16,
                last: index + 1 == count,
                text: PreeditText::new(chunk.as_str()).unwrap(),
            },
        )?;
    }
    Ok(())
}

fn texts(events: &[Event]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            Event::SearchText { text, .. } => Some(text.as_str().to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn any_client_asks_and_its_answer_reaches_it_alone() {
    let mut h = Harness::new();
    let long = format!(r#"{{"query":{{"text":"{}"}}}}"#, "ñ".repeat(100));
    ask(&mut h, APP, 1, &long).unwrap();
    ask(&mut h, SHELL, 7, r#"{"status":{}}"#).unwrap();
    assert_eq!(
        h.scene.take_search_asks(),
        [
            SearchAsk {
                client: APP,
                serial: 1,
                text: long.clone(),
            },
            SearchAsk {
                client: SHELL,
                serial: 7,
                text: r#"{"status":{}}"#.into(),
            },
        ]
    );
    assert!(h.scene.take_search_asks().is_empty());
    h.clear_log();
    let answer = format!(r#"{{"results":{{"items":"{}"}}}}"#, "x".repeat(300));
    h.scene.search_text(APP, 1, &answer);
    h.scene.search_text(APP, 0, r#"{"changed":{}}"#);
    h.pump();
    let events = h.events_for(APP);
    assert_eq!(texts(&events), format!(r#"{answer}{{"changed":{{}}}}"#));
    assert!(events.iter().any(|event| matches!(
        event,
        Event::SearchText {
            serial: 0,
            last: true,
            ..
        }
    )));
    assert!(texts(&h.events_for(OTHER)).is_empty());
    assert!(texts(&h.events_for(SHELL)).is_empty());
}

#[test]
fn chunks_out_of_order_or_past_the_quota_are_refused() {
    let mut h = Harness::new();
    let chunk = |index, last| Request::SearchRequest {
        serial: 1,
        index,
        last,
        text: PreeditText::new("{").unwrap(),
    };
    h.request(APP, chunk(0, false)).unwrap();
    assert_eq!(h.request(APP, chunk(2, true)), Err(ErrorCode::InvalidOrder));
    assert!(h.scene.take_search_asks().is_empty());
    // A request past MAX_SEARCH_TEXT_BYTES is refused before it grows further.
    let mut refused = None;
    let piece = "y".repeat(oxyos_protocol::TEXT_CHUNK_BYTES);
    for index in
        0..=(oxyos_protocol::MAX_SEARCH_TEXT_BYTES / oxyos_protocol::TEXT_CHUNK_BYTES) as u16
    {
        if let Err(error) = h.request(
            APP,
            Request::SearchRequest {
                serial: 2,
                index,
                last: false,
                text: PreeditText::new(&piece).unwrap(),
            },
        ) {
            refused = Some(error);
            break;
        }
    }
    assert_eq!(refused, Some(ErrorCode::QuotaExceeded));
}

#[test]
fn asks_past_the_limit_are_answered_busy_and_no_one_is_cut_off() {
    let mut h = Harness::new();
    for serial in 1..=MAX_SEARCH_ASKS as u32 {
        ask(&mut h, APP, serial, r#"{"query":{"text":"a"}}"#).unwrap();
    }
    h.clear_log();
    ask(&mut h, OTHER, 1, r#"{"query":{"text":"b"}}"#).unwrap();
    h.pump();
    assert_eq!(
        texts(&h.events_for(OTHER)),
        r#"{"refused":{"reason":"busy"}}"#
    );
    assert_eq!(h.scene.take_search_asks().len(), MAX_SEARCH_ASKS);
    ask(&mut h, OTHER, 2, r#"{"query":{"text":"b"}}"#).unwrap();
    assert_eq!(h.scene.take_search_asks().len(), 1);
}

#[test]
fn a_client_of_an_older_version_is_never_told_an_answer() {
    let mut h = Harness::new();
    h.scene.set_version(OTHER, SEARCH_VERSION - 1);
    h.scene.set_version(APP, SEARCH_VERSION);
    h.clear_log();
    h.scene.search_text(OTHER, 1, r#"{"results":{}}"#);
    h.scene.search_text(APP, 1, r#"{"results":{}}"#);
    h.pump();
    assert!(h.events_for(OTHER).is_empty());
    assert_eq!(texts(&h.events_for(APP)), r#"{"results":{}}"#);
}

#[test]
fn the_askers_that_go_are_told_once_and_their_asks_forgotten() {
    let mut h = Harness::new();
    ask(&mut h, APP, 1, r#"{"watch":{}}"#).unwrap();
    assert_eq!(h.scene.take_search_asks().len(), 1);
    ask(&mut h, APP, 2, r#"{"query":{"text":"a"}}"#).unwrap();
    // Half an ask of the other client's, and a client that never asked.
    h.request(
        OTHER,
        Request::SearchRequest {
            serial: 1,
            index: 0,
            last: false,
            text: PreeditText::new("{").unwrap(),
        },
    )
    .unwrap();
    h.scene.disconnect(APP);
    h.scene.disconnect(OTHER);
    assert!(h.scene.take_search_asks().is_empty());
    assert_eq!(h.scene.take_gone_searchers(), [APP]);
    assert!(h.scene.take_gone_searchers().is_empty());
    // An answer for a client gone reaches no one.
    h.clear_log();
    h.scene.search_text(APP, 2, r#"{"results":{}}"#);
    h.pump();
    assert!(h.events_for(APP).is_empty());
}
