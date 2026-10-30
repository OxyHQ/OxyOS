use super::*;

/// Solar Gradients' anchors, as its manifest gives them
/// (assets/wallpapers/solar-gradients/manifest.json).
const ANCHORS: &[Anchor] = &[
    Anchor {
        frame: 0,
        elevation: -87.61464321640851,
        azimuth: 260.5898004255673,
    },
    Anchor {
        frame: 1,
        elevation: -17.107833232320647,
        azimuth: 90.19641045280235,
    },
    Anchor {
        frame: 2,
        elevation: -11.106712852371931,
        azimuth: 90.18429228027907,
    },
    Anchor {
        frame: 3,
        elevation: -5.355608729615683,
        azimuth: 90.17504041417948,
    },
    Anchor {
        frame: 4,
        elevation: 0.18772787277664793,
        azimuth: 90.16773780886592,
    },
    Anchor {
        frame: 5,
        elevation: 6.042681701408481,
        azimuth: 90.16172842462277,
    },
    Anchor {
        frame: 6,
        elevation: 11.978559286514264,
        azimuth: 90.15767253075893,
    },
    Anchor {
        frame: 7,
        elevation: 18.453044152377785,
        azimuth: 90.1550761361018,
    },
    Anchor {
        frame: 8,
        elevation: 88.41858716616146,
        azimuth: 92.54659158870732,
    },
    Anchor {
        frame: 9,
        elevation: 18.363179677279813,
        azimuth: 270.0106978090315,
    },
    Anchor {
        frame: 10,
        elevation: 12.137504395377398,
        azimuth: 270.0174023923966,
    },
    Anchor {
        frame: 11,
        elevation: 6.445125948652895,
        azimuth: 270.0234735347242,
    },
    Anchor {
        frame: 12,
        elevation: 0.7510129535215688,
        azimuth: 270.0299192069967,
    },
    Anchor {
        frame: 13,
        elevation: -5.445832546887168,
        azimuth: 270.0372725068212,
    },
    Anchor {
        frame: 14,
        elevation: -11.196970640174076,
        azimuth: 270.0443273499976,
    },
    Anchor {
        frame: 15,
        elevation: -17.698226731979346,
        azimuth: 270.0532386567825,
    },
];

const MADRID: Place = Place {
    latitude: 40.4,
    longitude: -3.683_333,
};

/// Unix seconds of a UTC date and time.
fn utc(year: i64, month: i64, day: i64, hour: i64, minute: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + hour * 3600 + minute * 60
}

#[test]
fn days_are_counted_from_the_years_first() {
    assert_eq!(year_and_day(0), (1970, 1));
    assert_eq!(year_and_day(utc(2024, 12, 31, 0, 0) / 86_400), (2024, 366));
    assert_eq!(year_and_day(utc(2026, 10, 30, 0, 0) / 86_400), (2026, 303));
    assert_eq!(
        year_and_day(utc(1969, 12, 31, 0, 0).div_euclid(86_400)),
        (1969, 365)
    );
}

#[test]
fn the_sun_at_solar_noon_is_as_high_as_latitude_and_season_put_it() {
    // At the June solstice the noon sun stands 90 - 40.4 + 23.44 = 73.04° over Madrid, due
    // south; Madrid's solar noon is about 12:14 UTC then (longitude and equation of time).
    let noon = position(utc(2026, 6, 21, 12, 15), MADRID);
    assert!((noon.elevation - 73.0).abs() < 0.5, "{noon:?}");
    assert!((noon.azimuth - 180.0).abs() < 3.0, "{noon:?}");
    // At the December one, 90 - 40.4 - 23.44 = 26.16°.
    let noon = position(utc(2026, 12, 21, 12, 2), MADRID);
    assert!((noon.elevation - 26.2).abs() < 0.5, "{noon:?}");
    // At midnight it is as low: -(90 - 40.4 - 23.44) at the June solstice.
    let midnight = position(utc(2026, 6, 21, 0, 15), MADRID);
    assert!((midnight.elevation + 26.2).abs() < 0.5, "{midnight:?}");
    // On the equator at an equinox, overhead.
    let equator = position(
        utc(2026, 3, 20, 12, 7),
        Place {
            latitude: 0.0,
            longitude: 0.0,
        },
    );
    assert!(equator.elevation > 88.0, "{equator:?}");
}

#[test]
fn the_sun_rises_in_the_east_and_sets_in_the_west() {
    // Madrid's sunrise at the June solstice is about 04:45 UTC, its sunset about 19:48.
    let rising = position(utc(2026, 6, 21, 4, 45), MADRID);
    assert!(
        rising.elevation.abs() < 1.5 && rising.rising(),
        "{rising:?}"
    );
    assert!((rising.azimuth - 58.0).abs() < 3.0, "{rising:?}");
    let setting = position(utc(2026, 6, 21, 19, 48), MADRID);
    assert!(
        setting.elevation.abs() < 1.5 && !setting.rising(),
        "{setting:?}"
    );
    assert!((setting.azimuth - 302.0).abs() < 3.0, "{setting:?}");
}

#[test]
fn a_zones_place_comes_from_the_time_zone_database() {
    let table = "# comment\nES\t+4024-00341\tEurope/Madrid\tSpain (mainland)\nFI,AX\t+6010+02458\tEurope/Helsinki\nUS\t+404251-0740023\tAmerica/New_York\tEastern (most areas)\n";
    let madrid = zone_place("Europe/Madrid", table).unwrap();
    assert!((madrid.latitude - 40.4).abs() < 1e-9 && (madrid.longitude + 3.683_333).abs() < 1e-5);
    let helsinki = zone_place("Europe/Helsinki", table).unwrap();
    assert!(
        (helsinki.latitude - 60.166_667).abs() < 1e-5
            && (helsinki.longitude - 24.966_667).abs() < 1e-5
    );
    let new_york = zone_place("America/New_York", table).unwrap();
    assert!(
        (new_york.latitude - 40.714_167).abs() < 1e-5
            && (new_york.longitude + 74.006_389).abs() < 1e-5
    );
    assert_eq!(zone_place("UTC", table), None);
}

#[test]
fn every_anchor_resolves_to_its_frame_on_its_phase() {
    for anchor in ANCHORS {
        assert_eq!(
            select_frame(ANCHORS, anchor.elevation, anchor.azimuth < 180.0),
            Some(anchor.frame)
        );
    }
}

#[test]
fn morning_and_evening_twilight_use_different_frames_and_no_position_chooses_none() {
    assert_eq!(select_frame(ANCHORS, -5.4, true), Some(3));
    assert_eq!(select_frame(ANCHORS, -5.4, false), Some(13));
    assert_eq!(select_frame(ANCHORS, -90.0, true), Some(0));
    assert_eq!(select_frame(ANCHORS, 89.0, false), Some(8));
    assert_eq!(select_frame(ANCHORS, f64::NAN, true), None);
    assert_eq!(select_frame(ANCHORS, 91.0, true), None);
    assert_eq!(select_frame(&[], 10.0, true), None);
}

#[test]
fn the_frame_follows_the_day_and_says_when_it_changes() {
    // Madrid, a June day: night at 02:00 UTC, morning frames, the day's highest at noon, evening
    // frames, night again.
    let at = |hour, minute| frame_at(ANCHORS, utc(2026, 6, 21, hour, minute), MADRID).unwrap();
    assert_eq!(at(1, 0).0, 1, "deep night, the morning's darkest twilight");
    assert_eq!(at(4, 45).0, 4, "sunrise");
    assert_eq!(
        at(12, 15).0,
        8,
        "noon: 73°, closest to the day's highest frame (88°)"
    );
    assert_eq!(at(19, 48).0, 12, "sunset");
    assert_eq!(at(21, 30).0, 15, "after dusk");
    // The change it announces is the first minute the frame differs.
    let (frame, next) = at(4, 45);
    assert!(next > utc(2026, 6, 21, 4, 45));
    let sun = position(next, MADRID);
    assert_ne!(
        select_frame(ANCHORS, sun.elevation, sun.rising()),
        Some(frame)
    );
    let before = position(next - 60, MADRID);
    assert_eq!(
        select_frame(ANCHORS, before.elevation, before.rising()),
        Some(frame)
    );
}

#[test]
fn night_is_the_sun_below_the_horizon_and_says_when_it_ends() {
    // Madrid, the June solstice: sunrise about 04:45 UTC, sunset about 19:48.
    let (night, sunrise) = night_at(utc(2026, 6, 21, 1, 0), MADRID);
    assert!(night);
    assert!(
        (sunrise - utc(2026, 6, 21, 4, 45)).abs() <= 15 * 60,
        "{sunrise}"
    );
    // The first minute the sun is up, the one before it still down.
    assert!(position(sunrise, MADRID).elevation >= 0.0);
    assert!(position(sunrise - 60, MADRID).elevation < 0.0);
    let (night, sunset) = night_at(utc(2026, 6, 21, 12, 0), MADRID);
    assert!(!night);
    assert!(
        (sunset - utc(2026, 6, 21, 19, 48)).abs() <= 15 * 60,
        "{sunset}"
    );
    // Where the sun does not set (the pole in June), look again a day on.
    let pole = Place {
        latitude: 89.0,
        longitude: 0.0,
    };
    let now = utc(2026, 6, 21, 0, 0);
    assert_eq!(night_at(now, pole), (false, now + 86_400));
}
