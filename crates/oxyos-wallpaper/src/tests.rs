use super::*;
use oxyos_preferences::wallpaper::Choice;

/// A scratch folder of the test's own.
fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("oxyos-wallpaper-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

const SVG: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 10'><rect width='16' height='10' fill='#123456'/></svg>";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A catalog of a default that follows the appearance, a wallpaper that follows the sun, two
/// still gradients, and colours.
fn catalog(name: &str) -> Catalog {
    let root = scratch(name);
    write(
        &root.join("catalog.json"),
        r##"{"version": 1, "default": "oxyos",
            "categories": [{"id": "dynamic", "name": {"en": "Dynamic", "es": "Dinámicos"}},
                           {"id": "gradients", "name": {"en": "Gradients"}}],
            "colors": [{"color": "#1F4FD8", "name": {"en": "Blue", "es": "Azul"}},
                       {"color": "#000000", "name": {"en": "Black"}}]}"##,
    );
    write(
        &root.join("oxyos/wallpaper.json"),
        r#"{"name": {"en": "OxyOS"}, "category": "dynamic", "automatic": "appearance", "preview": "light",
            "variants": [{"id": "light", "name": {"en": "Light"}, "file": "light.svg", "appearance": "light"},
                         {"id": "dark", "name": {"en": "Dark"}, "file": "dark.svg", "appearance": "dark"}]}"#,
    );
    write(
        &root.join("horizon/wallpaper.json"),
        r#"{"name": {"en": "Horizon", "es": "Horizonte"}, "category": "dynamic", "automatic": "sun", "preview": "day",
            "variants": [{"id": "night", "name": {"en": "Night"}, "file": "night.svg", "sun": {"elevation": -30, "azimuth": 0}},
                         {"id": "dawn", "name": {"en": "Dawn"}, "file": "dawn.svg", "sun": {"elevation": -12, "azimuth": 90}},
                         {"id": "day", "name": {"en": "Day"}, "file": "day.svg", "sun": {"elevation": 60, "azimuth": 180}},
                         {"id": "dusk", "name": {"en": "Dusk"}, "file": "dusk.svg", "sun": {"elevation": -12, "azimuth": 270}}]}"#,
    );
    for id in ["ocean", "iris"] {
        write(
            &root.join(id).join("wallpaper.json"),
            &format!(
                r#"{{"name": {{"en": "{id}"}}, "category": "gradients", "automatic": "none", "preview": "only",
                    "variants": [{{"id": "only", "name": {{"en": "Only"}}, "file": "only.svg"}}]}}"#
            ),
        );
        write(&root.join(id).join("only.svg"), SVG);
    }
    // Not wallpapers: an unknown key, a file outside its folder, a category not the catalog's.
    write(
        &root.join("bad-key/wallpaper.json"),
        r#"{"name": {"en": "x"}, "category": "gradients", "automatic": "none", "preview": "a", "extra": 1,
            "variants": [{"id": "a", "name": {"en": "A"}, "file": "a.svg"}]}"#,
    );
    write(
        &root.join("outside/wallpaper.json"),
        r#"{"name": {"en": "x"}, "category": "gradients", "automatic": "none", "preview": "a",
            "variants": [{"id": "a", "name": {"en": "A"}, "file": "../oxyos/light.svg"}]}"#,
    );
    for file in [
        "oxyos/light.svg",
        "oxyos/dark.svg",
        "horizon/night.svg",
        "horizon/dawn.svg",
        "horizon/day.svg",
        "horizon/dusk.svg",
    ] {
        write(&root.join(file), SVG);
    }
    Catalog::read(&root)
}

fn moment(now: i64) -> Moment {
    Moment {
        now,
        login: 1_000,
        dark: false,
        output: 1,
        place: None,
    }
}

const MADRID: oxyos_solar::Place = oxyos_solar::Place {
    latitude: 40.4,
    longitude: -3.68,
};

#[test]
fn the_catalog_is_read_strictly_in_its_categories_order() {
    let catalog = catalog("read");
    let ids: Vec<&str> = catalog.wallpapers.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(ids, ["horizon", "oxyos", "iris", "ocean"]);
    assert_eq!(catalog.default.as_deref(), Some("oxyos"));
    assert_eq!(catalog.colors[0].color, Rgb([0x1f, 0x4f, 0xd8]));
    assert_eq!(catalog.colors[0].name.get("es-ES"), "Azul");
    assert_eq!(catalog.colors[1].name.get("es"), "Black");
    assert_eq!(
        catalog.wallpaper("horizon").unwrap().name.get("es"),
        "Horizonte"
    );
    // No catalog: nothing, and the shell's blue shown.
    let none = Catalog::read(Path::new("/nonexistent"));
    assert!(none.wallpapers.is_empty());
    assert_eq!(show(&none, &Choice::Default, &moment(0)).color, FALLBACK);
}

#[test]
fn automatic_follows_the_appearance_or_the_sun() {
    let catalog = catalog("automatic");
    let light = show(&catalog, &Choice::Default, &moment(0));
    assert_eq!(light.key, "oxyos/light");
    assert!(light.uri().unwrap().ends_with("/oxyos/light.svg"));
    let dark = show(
        &catalog,
        &Choice::Default,
        &Moment {
            dark: true,
            ..moment(0)
        },
    );
    assert_eq!(dark.key, "oxyos/dark");
    // A fixed variant, whatever the appearance.
    let fixed = Choice::parse("oxyos/light").unwrap();
    assert_eq!(
        show(
            &catalog,
            &fixed,
            &Moment {
                dark: true,
                ..moment(0)
            }
        )
        .key,
        "oxyos/light"
    );
    // The sun: Madrid at 03:00 on the June solstice is night, noon is day; no place, the preview.
    let horizon = Choice::parse("horizon").unwrap();
    let night = show(
        &catalog,
        &horizon,
        &Moment {
            place: Some(MADRID),
            ..moment(1_782_003_600)
        },
    );
    assert_eq!(night.key, "horizon/night");
    assert!(night.next.is_some_and(|next| next > 1_782_003_600));
    let noon = show(
        &catalog,
        &horizon,
        &Moment {
            place: Some(MADRID),
            ..moment(1_782_043_200)
        },
    );
    assert_eq!(noon.key, "horizon/day");
    assert_eq!(
        show(&catalog, &horizon, &moment(1_782_003_600)).key,
        "horizon/day"
    );
}

#[test]
fn what_names_nothing_shows_the_default() {
    let catalog = catalog("nothing");
    for text in [
        "gone",
        "photo:fill:000000:/nonexistent/a.jpg",
        "folder:fill:000000:5:/nonexistent",
        "shuffle:landscapes:5",
    ] {
        let choice = Choice::parse(text).unwrap();
        assert_eq!(
            show(&catalog, &choice, &moment(0)).key,
            "oxyos/light",
            "{text}"
        );
    }
    // An unknown variant: Automatic's.
    let choice = Choice::parse("oxyos/sepia").unwrap();
    assert_eq!(show(&catalog, &choice, &moment(0)).key, "oxyos/light");
}

#[test]
fn photos_fit_over_their_colour_and_colours_are_themselves() {
    let catalog = catalog("photos");
    let folder = scratch("photos-folder");
    let photo = folder.join("Día de playa.svg");
    write(&photo, SVG);
    let choice = Choice::parse(&format!("photo:tile:ffffff:{}", photo.display())).unwrap();
    let shown = show(&catalog, &choice, &moment(0));
    assert_eq!(shown.fit, Fit::Tile);
    assert_eq!(shown.color, Rgb([255, 255, 255]));
    assert!(shown.uri().unwrap().ends_with("/D%C3%ADa%20de%20playa.svg"));
    assert_eq!(path_of(&shown.uri().unwrap()).unwrap(), photo);
    let color = show(
        &catalog,
        &Choice::parse("color:1f4fd8").unwrap(),
        &moment(0),
    );
    assert_eq!((color.file, color.color), (None, Rgb([0x1f, 0x4f, 0xd8])));
}

#[test]
fn a_shuffle_changes_each_slot_never_to_the_same_and_each_display_on_its_own() {
    let catalog = catalog("shuffle");
    let folder = scratch("shuffle-folder");
    for name in ["a.jpg", "b.png", "c.svg", ".hidden.jpg", "notes.txt"] {
        write(&folder.join(name), SVG);
    }
    assert_eq!(
        folder_images(&folder)
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        ["a.jpg", "b.png", "c.svg"]
    );
    let choice = Choice::parse(&format!("folder:fill:000000:5:{}", folder.display())).unwrap();
    let mut last = None;
    for slot in 0..50 {
        let now = 1_782_000_000 + slot * 300;
        let shown = show(&catalog, &choice, &moment(now));
        assert_eq!(shown.next, Some((now / 300 + 1) * 300));
        // The same within the slot.
        assert_eq!(
            show(&catalog, &choice, &moment(now + 299 - now % 300)).key,
            shown.key
        );
        assert_ne!(Some(shown.key.clone()), last, "slot {slot}");
        last = Some(shown.key);
    }
    // Each display its own: over many slots, two displays differ somewhere.
    let differ = (0..20).any(|slot| {
        let now = slot * 300;
        show(&catalog, &choice, &moment(now)).key
            != show(
                &catalog,
                &choice,
                &Moment {
                    output: 2,
                    ..moment(now)
                },
            )
            .key
    });
    assert!(differ);
    // At login: the session's, never changing by the clock.
    let login = Choice::parse("shuffle:gradients:login").unwrap();
    let shown = show(&catalog, &login, &moment(0));
    assert_eq!(shown.next, None);
    assert_eq!(show(&catalog, &login, &moment(1_000_000)).key, shown.key);
    assert!(["iris/only", "ocean/only"].contains(&shown.key.as_str()));
    // The colours' shuffle.
    let colors = Choice::parse("shuffle:colors:60").unwrap();
    assert!(show(&catalog, &colors, &moment(0))
        .key
        .starts_with("color:"));
}

#[test]
fn a_thumbnail_is_made_once_at_its_size() {
    let folder = scratch("thumbnail");
    let photo = folder.join("photo.png");
    // A 40 × 20 PNG, red on its left half and blue on its right.
    let mut rgba = Vec::new();
    for _ in 0..20 {
        for x in 0..40 {
            rgba.extend_from_slice(if x < 20 {
                &[255, 0, 0, 255]
            } else {
                &[0, 0, 255, 255]
            });
        }
    }
    thumbnail::write_png_for_tests(&photo, &rgba, 40, 20);
    let cache = folder.join("cache");
    let made = thumbnail::thumbnail(&photo, 10, 10, &cache).unwrap();
    let image = oxyos_image::decode_file(&made, oxyos_image::Fit::Whole).unwrap();
    // Covering 10 × 10: reduced to 20 × 10, its middle 10 × 10 kept.
    assert_eq!((image.width, image.height), (10, 10));
    assert_eq!(&image.pixels[..4], &[255, 0, 0, 255]);
    assert_eq!(&image.pixels[image.pixels.len() - 4..], &[0, 0, 255, 255]);
    // Kept: the same file again.
    assert_eq!(thumbnail::thumbnail(&photo, 10, 10, &cache).unwrap(), made);
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);
    // An SVG is its own; too large a size is refused.
    let svg = folder.join("a.svg");
    write(&svg, SVG);
    assert_eq!(thumbnail::thumbnail(&svg, 10, 10, &cache).unwrap(), svg);
    assert!(thumbnail::thumbnail(&photo, 0, 10, &cache).is_err());
    assert!(thumbnail::thumbnail(&photo, 2000, 10, &cache).is_err());
}

#[test]
fn file_uris_round_trip() {
    let path = Path::new("/home/ana/Fotos/año 2026/#1 100%.jpg");
    let uri = file_uri(path);
    assert_eq!(
        uri,
        "file:///home/ana/Fotos/a%C3%B1o%202026/%231%20100%25.jpg"
    );
    assert_eq!(path_of(&uri).unwrap(), path);
    assert_eq!(path_of("/a/b").unwrap(), Path::new("/a/b"));
    assert_eq!(path_of("http://x/a"), None);
    assert_eq!(path_of("file://relative"), None);
}

#[test]
fn the_images_catalog_is_whole() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../config/includes.chroot_after_packages/usr/share/oxyos/wallpapers");
    let catalog = Catalog::read(&root);
    let on_disk = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .filter(|e| e.path().join("wallpaper.json").is_file())
        .count();
    // Every folder read, none refused.
    assert_eq!(catalog.wallpapers.len(), on_disk);
    // The default follows the sun through its sixteen variants (Solar Gradients).
    assert_eq!(catalog.default.as_deref(), Some("solar-gradients"));
    let solar = catalog.wallpaper("solar-gradients").unwrap();
    assert_eq!(
        (solar.automatic, solar.variants.len()),
        (Automatic::Sun, 16)
    );
    assert!(!catalog.colors.is_empty());
    for wallpaper in &catalog.wallpapers {
        assert!(
            wallpaper.name.es.is_some(),
            "{} has no Spanish name",
            wallpaper.id
        );
        for variant in &wallpaper.variants {
            assert!(variant.name.es.is_some(), "{}/{}", wallpaper.id, variant.id);
        }
    }
    for color in &catalog.colors {
        assert!(color.name.es.is_some());
    }
}

#[test]
fn a_place_is_two_degrees() {
    assert_eq!(
        parse_place("40.4, -3.68"),
        Some(oxyos_solar::Place {
            latitude: 40.4,
            longitude: -3.68
        })
    );
    for text in ["", "40.4", "91,0", "0,181", "a,b"] {
        assert_eq!(parse_place(text), None, "{text}");
    }
}
