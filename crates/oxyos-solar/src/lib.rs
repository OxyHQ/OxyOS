//! Where the sun is, and which frame of a solar wallpaper shows it (docs/adr/0012-solar-wallpaper.mdx):
//! NOAA's general solar position from the clock and a place, the place being the system time
//! zone's (its principal city's, from the time zone database), never a location service.

use std::f64::consts::PI;

/// The sun seen from a place: degrees above the horizon (negative below it), and degrees
/// clockwise from north.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub elevation: f64,
    pub azimuth: f64,
}

impl Position {
    /// Morning: the sun in the eastern half of the sky.
    pub fn rising(&self) -> bool {
        self.azimuth < 180.0
    }
}

/// A place: degrees north and east.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub latitude: f64,
    pub longitude: f64,
}

/// Where the sun is at `unix_seconds` (UTC) from `place`, by NOAA's general solar position
/// calculation (fractional year, equation of time, declination, hour angle), with no
/// refraction: within a few tenths of a degree.
pub fn position(unix_seconds: i64, place: Place) -> Position {
    let days = unix_seconds.div_euclid(86_400);
    let seconds = unix_seconds.rem_euclid(86_400) as f64;
    let (year, ordinal) = year_and_day(days);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let length = if leap { 366.0 } else { 365.0 };
    let hours = seconds / 3600.0;
    // The fractional year, in radians.
    let g = 2.0 * PI / length * (ordinal as f64 - 1.0 + (hours - 12.0) / 24.0);
    // The equation of time (minutes) and the declination (radians).
    let equation = 229.18
        * (0.000075 + 0.001868 * g.cos()
            - 0.032077 * g.sin()
            - 0.014615 * (2.0 * g).cos()
            - 0.040849 * (2.0 * g).sin());
    let declination = 0.006918 - 0.399912 * g.cos() + 0.070257 * g.sin()
        - 0.006758 * (2.0 * g).cos()
        + 0.000907 * (2.0 * g).sin()
        - 0.002697 * (3.0 * g).cos()
        + 0.00148 * (3.0 * g).sin();
    // True solar time (minutes) and the hour angle (radians).
    let solar = seconds / 60.0 + equation + 4.0 * place.longitude;
    let hour_angle = (solar / 4.0 - 180.0).to_radians();
    let latitude = place.latitude.to_radians();
    let cos_zenith = (latitude.sin() * declination.sin()
        + latitude.cos() * declination.cos() * hour_angle.cos())
    .clamp(-1.0, 1.0);
    let zenith = cos_zenith.acos();
    // Azimuth clockwise from north: from the zenith's sine and the hour angle's side.
    let cos_azimuth = ((latitude.sin() * cos_zenith - declination.sin())
        / (latitude.cos() * zenith.sin()).max(1e-12))
    .clamp(-1.0, 1.0);
    let from_south = cos_azimuth.acos().to_degrees();
    let azimuth = if hour_angle.sin() > 0.0 {
        (180.0 + from_south) % 360.0
    } else {
        (540.0 - from_south) % 360.0
    };
    Position {
        elevation: 90.0 - zenith.to_degrees(),
        azimuth,
    }
}

/// The civil year and the day in it (1 for January 1st) of `days` since 1970-01-01.
fn year_and_day(days: i64) -> (i64, i64) {
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    // Days before the year's January 1st.
    let jan1 = days_from_civil(year);
    (year, days - jan1 + 1)
}

fn days_from_civil(year: i64) -> i64 {
    let y = year - 1;
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = 306;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The place of a time zone (`Europe/Madrid`), from the time zone database's `zone1970.tab`:
/// its principal city's coordinates, `±DDMM±DDDMM` or `±DDMMSS±DDDMMSS`.
pub fn zone_place(zone: &str, table: &str) -> Option<Place> {
    let line = table
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find(|line| line.split('\t').nth(2) == Some(zone))?;
    let coordinates = line.split('\t').nth(1)?;
    let split = coordinates[1..].find(['+', '-'])? + 1;
    Some(Place {
        latitude: angle(&coordinates[..split], 2)?,
        longitude: angle(&coordinates[split..], 3)?,
    })
}

/// `±DD[D]MM[SS]` in degrees.
fn angle(text: &str, degree_digits: usize) -> Option<f64> {
    let sign = match text.as_bytes().first()? {
        b'+' => 1.0,
        b'-' => -1.0,
        _ => return None,
    };
    let digits = &text[1..];
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let part = |range: std::ops::Range<usize>| digits.get(range)?.parse::<f64>().ok();
    let degrees = part(0..degree_digits)?;
    let minutes = part(degree_digits..degree_digits + 2)?;
    let seconds = match digits.len() - degree_digits {
        2 => 0.0,
        4 => part(degree_digits + 2..degree_digits + 4)?,
        _ => return None,
    };
    Some(sign * (degrees + minutes / 60.0 + seconds / 3600.0))
}

/// The place of the time zone the system's clock is set to, if the time zone database gives
/// it one (`UTC` has none).
pub fn system_place() -> Option<Place> {
    let zone = jiff::tz::TimeZone::system();
    let name = zone.iana_name()?;
    let directory = std::env::var_os("TZDIR").unwrap_or_else(|| "/usr/share/zoneinfo".into());
    let table =
        std::fs::read_to_string(std::path::Path::new(&directory).join("zone1970.tab")).ok()?;
    zone_place(name, &table)
}

/// A frame of a solar wallpaper and the sun it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub frame: u32,
    pub elevation: f64,
    pub azimuth: f64,
}

/// The frame of `anchors` that shows the sun at `elevation`, `rising` or setting: among the
/// anchors of that phase (the eastern half of the sky rising, the western setting; the lowest
/// and the highest belong to both), the closest in elevation. None for an elevation that is
/// none, or no anchors.
pub fn select_frame(anchors: &[Anchor], elevation: f64, rising: bool) -> Option<u32> {
    if !elevation.is_finite() || !(-90.0..=90.0).contains(&elevation) {
        return None;
    }
    let lowest = anchors
        .iter()
        .map(|a| a.elevation)
        .fold(f64::INFINITY, f64::min);
    let highest = anchors
        .iter()
        .map(|a| a.elevation)
        .fold(f64::NEG_INFINITY, f64::max);
    anchors
        .iter()
        .filter(|a| {
            a.elevation == lowest || a.elevation == highest || (a.azimuth < 180.0) == rising
        })
        .fold(None::<&Anchor>, |closest, anchor| match closest {
            Some(closest)
                if (closest.elevation - elevation).abs()
                    <= (anchor.elevation - elevation).abs() =>
            {
                Some(closest)
            }
            _ => Some(anchor),
        })
        .map(|anchor| anchor.frame)
}

/// The frame showing the sun at `unix_seconds` from `place`, and when (unix seconds) it next
/// changes, looked for minute by minute over the next day.
pub fn frame_at(anchors: &[Anchor], unix_seconds: i64, place: Place) -> Option<(u32, i64)> {
    let frame_then = |t: i64| {
        let sun = position(t, place);
        select_frame(anchors, sun.elevation, sun.rising())
    };
    let now = frame_then(unix_seconds)?;
    let next = (1..=24 * 60)
        .map(|minute| unix_seconds - unix_seconds.rem_euclid(60) + minute * 60)
        .find(|&t| frame_then(t) != Some(now))
        .unwrap_or(unix_seconds + 86_400);
    Some((now, next))
}

#[cfg(test)]
mod tests;
