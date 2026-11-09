use super::*;

#[test]
fn debug_hides_the_text() {
    let secret = Secret::new("hunter2-correct-horse");
    let shown = format!("{secret:?} {:?}", Some(&secret));
    assert_eq!(shown, "Secret(…) Some(Secret(…))");
}

#[test]
fn the_text_is_read_only_through_expose_and_as_bytes() {
    let secret = Secret::new(String::from("correct horse"));
    assert_eq!(secret.expose(), "correct horse");
    assert_eq!(secret.as_bytes(), b"correct horse");
    assert_eq!(secret.len(), 13);
    assert!(!secret.is_empty());
    assert!(Secret::default().is_empty());
    assert!(Secret::new("").is_empty());
}

#[test]
fn a_string_moves_in_and_its_bytes_move_out_without_a_copy() {
    let text = String::from("correct horse battery");
    let address = text.as_ptr();
    let secret = Secret::new(text);
    assert_eq!(secret.expose().as_ptr(), address);
    let bytes = secret.into_bytes();
    assert_eq!(bytes.as_ptr(), address);
    assert_eq!(&**bytes, b"correct horse battery");

    let bytes = b"from a wire".to_vec();
    let address = bytes.as_ptr();
    let secret = Secret::from_utf8(bytes).unwrap();
    assert_eq!(secret.as_bytes().as_ptr(), address);
}

#[test]
fn bytes_that_are_not_text_are_refused() {
    assert_eq!(Secret::from_utf8(vec![b'a', 0xff, b'b']), Err(NotText));
    // NUL is text: refusing it is for whoever hands the secret to C (PAM).
    assert_eq!(
        Secret::from_utf8(b"a\0b".to_vec()).unwrap().as_bytes(),
        b"a\0b"
    );
}

#[test]
fn equal_secrets_compare_equal() {
    assert_eq!(Secret::new("a"), Secret::new(String::from("a")));
    assert_ne!(Secret::new("a"), Secret::new("b"));
    let secret = Secret::new("clone me");
    assert_eq!(secret.clone(), secret);
}

#[test]
fn zeroize_empties_it() {
    let mut secret = Secret::new("hunter2");
    secret.zeroize();
    assert!(secret.is_empty());
}

#[test]
fn clear_empties_a_plain_string_and_keeps_its_buffer() {
    let mut text = String::with_capacity(32);
    text.push_str("hunter2");
    let address = text.as_ptr();
    clear(&mut text);
    // Overwritten in place (zeroize writes every byte of the capacity), never moved elsewhere.
    assert!(text.is_empty());
    assert_eq!(text.as_ptr(), address);
}

#[cfg(feature = "serde")]
#[test]
fn json_holds_the_text() {
    let secret: Secret = serde_json::from_str(r#""hunter2""#).unwrap();
    assert_eq!(secret.expose(), "hunter2");
    assert_eq!(serde_json::to_string(&secret).unwrap(), r#""hunter2""#);
}
