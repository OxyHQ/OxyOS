//! The process's default locale, set and unset: a test binary of its own, since it is the whole
//! process's.

#[test]
fn the_default_locale_is_the_one_set_until_it_is_unset() {
    let system = oxyos_intl::system_locale();
    assert_eq!(oxyos_intl::default_locale(), system);
    // Kept with its Unicode extension keywords, as resolve keeps them; canonicalized.
    assert_eq!(
        oxyos_intl::set_default_locale("ES-mx-u-hc-h12").unwrap(),
        "es-MX-u-hc-h12"
    );
    assert_eq!(oxyos_intl::default_locale(), "es-MX-u-hc-h12");
    // What formats default to: a locale with no data resolves to it.
    assert_eq!(oxyos_intl::resolve("xx").unwrap(), "es-MX-u-hc-h12");
    assert_eq!(oxyos_intl::system_locale(), system);
    // Seen from every thread.
    let other = std::thread::spawn(oxyos_intl::default_locale)
        .join()
        .unwrap();
    assert_eq!(other, "es-MX-u-hc-h12");
    // An invalid tag, or a list, changes nothing.
    assert!(oxyos_intl::set_default_locale("en_US").is_err());
    assert!(oxyos_intl::set_default_locale("en,es").is_err());
    assert_eq!(oxyos_intl::default_locale(), "es-MX-u-hc-h12");
    // A tag without data is the system's.
    assert_eq!(oxyos_intl::set_default_locale("xx").unwrap(), system);
    assert_eq!(oxyos_intl::set_default_locale("en-ES").unwrap(), "en-ES");
    // Empty: the system's again.
    assert_eq!(oxyos_intl::set_default_locale("").unwrap(), system);
    assert_eq!(oxyos_intl::default_locale(), system);
}
