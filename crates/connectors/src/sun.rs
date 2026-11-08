//! Sunrise and sunset from latitude and longitude, with the NOAA solar
//! calculator formulas (Meeus, "Astronomical Algorithms", chapters 7, 22, 25
//! and 28). Pure arithmetic: no network, nothing stored.
//!
//! Accuracy is about a minute at ordinary latitudes (the sun's edge touching the
//! horizon, 0.833 degrees of refraction and radius), worse near the poles.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;

/// A place the almanac knows: name, latitude, longitude (degrees, north and
/// east positive) and IANA time zone. Also used for "time in <city>".
pub const PLACES: &[(&str, f64, f64, &str)] = &[
    ("new york", 40.7128, -74.0060, "America/New_York"),
    ("nyc", 40.7128, -74.0060, "America/New_York"),
    ("boston", 42.3601, -71.0589, "America/New_York"),
    ("washington", 38.9072, -77.0369, "America/New_York"),
    ("chicago", 41.8781, -87.6298, "America/Chicago"),
    ("dallas", 32.7767, -96.7970, "America/Chicago"),
    ("houston", 29.7604, -95.3698, "America/Chicago"),
    ("denver", 39.7392, -104.9903, "America/Denver"),
    ("phoenix", 33.4484, -112.0740, "America/Phoenix"),
    ("los angeles", 34.0522, -118.2437, "America/Los_Angeles"),
    ("san francisco", 37.7749, -122.4194, "America/Los_Angeles"),
    ("seattle", 47.6062, -122.3321, "America/Los_Angeles"),
    ("vancouver", 49.2827, -123.1207, "America/Vancouver"),
    ("toronto", 43.6532, -79.3832, "America/Toronto"),
    ("mexico city", 19.4326, -99.1332, "America/Mexico_City"),
    ("sao paulo", -23.5505, -46.6333, "America/Sao_Paulo"),
    (
        "buenos aires",
        -34.6037,
        -58.3816,
        "America/Argentina/Buenos_Aires",
    ),
    ("london", 51.5074, -0.1278, "Europe/London"),
    ("dublin", 53.3498, -6.2603, "Europe/Dublin"),
    ("paris", 48.8566, 2.3522, "Europe/Paris"),
    ("berlin", 52.5200, 13.4050, "Europe/Berlin"),
    ("madrid", 40.4168, -3.7038, "Europe/Madrid"),
    ("rome", 41.9028, 12.4964, "Europe/Rome"),
    ("amsterdam", 52.3676, 4.9041, "Europe/Amsterdam"),
    ("zurich", 47.3769, 8.5417, "Europe/Zurich"),
    ("moscow", 55.7558, 37.6173, "Europe/Moscow"),
    ("istanbul", 41.0082, 28.9784, "Europe/Istanbul"),
    ("reykjavik", 64.1466, -21.9426, "Atlantic/Reykjavik"),
    ("tromso", 69.6492, 18.9553, "Europe/Oslo"),
    ("longyearbyen", 78.2232, 15.6267, "Arctic/Longyearbyen"),
    ("cairo", 30.0444, 31.2357, "Africa/Cairo"),
    ("lagos", 6.5244, 3.3792, "Africa/Lagos"),
    ("nairobi", -1.2921, 36.8219, "Africa/Nairobi"),
    ("johannesburg", -26.2041, 28.0473, "Africa/Johannesburg"),
    ("dubai", 25.2048, 55.2708, "Asia/Dubai"),
    ("abu dhabi", 24.4539, 54.3773, "Asia/Dubai"),
    ("riyadh", 24.7136, 46.6753, "Asia/Riyadh"),
    ("tehran", 35.6892, 51.3890, "Asia/Tehran"),
    ("karachi", 24.8607, 67.0011, "Asia/Karachi"),
    ("delhi", 28.6139, 77.2090, "Asia/Kolkata"),
    ("mumbai", 19.0760, 72.8777, "Asia/Kolkata"),
    ("pune", 18.5204, 73.8567, "Asia/Kolkata"),
    ("bangalore", 12.9716, 77.5946, "Asia/Kolkata"),
    ("bengaluru", 12.9716, 77.5946, "Asia/Kolkata"),
    ("hyderabad", 17.3850, 78.4867, "Asia/Kolkata"),
    ("chennai", 13.0827, 80.2707, "Asia/Kolkata"),
    ("kolkata", 22.5726, 88.3639, "Asia/Kolkata"),
    ("ahmedabad", 23.0225, 72.5714, "Asia/Kolkata"),
    ("jaipur", 26.9124, 75.7873, "Asia/Kolkata"),
    ("dhaka", 23.8103, 90.4125, "Asia/Dhaka"),
    ("bangkok", 13.7563, 100.5018, "Asia/Bangkok"),
    ("jakarta", -6.2088, 106.8456, "Asia/Jakarta"),
    ("singapore", 1.3521, 103.8198, "Asia/Singapore"),
    ("kuala lumpur", 3.1390, 101.6869, "Asia/Kuala_Lumpur"),
    ("hong kong", 22.3193, 114.1694, "Asia/Hong_Kong"),
    ("shanghai", 31.2304, 121.4737, "Asia/Shanghai"),
    ("beijing", 39.9042, 116.4074, "Asia/Shanghai"),
    ("taipei", 25.0330, 121.5654, "Asia/Taipei"),
    ("manila", 14.5995, 120.9842, "Asia/Manila"),
    ("seoul", 37.5665, 126.9780, "Asia/Seoul"),
    ("tokyo", 35.6762, 139.6503, "Asia/Tokyo"),
    ("sydney", -33.8688, 151.2093, "Australia/Sydney"),
    ("melbourne", -37.8136, 144.9631, "Australia/Melbourne"),
    ("perth", -31.9505, 115.8605, "Australia/Perth"),
    ("auckland", -36.8485, 174.7633, "Pacific/Auckland"),
    ("honolulu", 21.3099, -157.8581, "Pacific/Honolulu"),
];

/// Look a place up by (lower-case) name.
pub fn place(name: &str) -> Option<&'static (&'static str, f64, f64, &'static str)> {
    PLACES.iter().find(|p| p.0 == name)
}

/// What the sun does at a place on one local day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SunEvent {
    At(DateTime<Utc>),
    /// The sun stays below the horizon all day (polar night): no sunrise or sunset.
    PolarNight,
    /// The sun stays up all day (midnight sun): no sunrise or sunset.
    PolarDay,
    /// Rare: the event does not fall on this local date.
    NoneToday,
}

/// Julian day number at 00:00 UTC of `date`.
fn julian_day(date: NaiveDate) -> f64 {
    // 1970-01-01 is JD 2440587.5 at 00:00 UTC.
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch");
    2_440_587.5 + (date - epoch).num_days() as f64
}

struct SolarPosition {
    declination: f64, // radians
    eq_time: f64,     // minutes
}

fn solar_position(jd: f64) -> SolarPosition {
    let t = (jd - 2_451_545.0) / 36_525.0;
    let l0 = (280.46646 + t * (36_000.769_83 + 0.0003032 * t)).rem_euclid(360.0);
    let m = 357.52911 + t * (35_999.050_29 - 0.0001537 * t);
    let e = 0.016_708_634 - t * (0.000_042_037 + 0.000_000_126_7 * t);
    let mr = m.to_radians();
    let c = mr.sin() * (1.914602 - t * (0.004817 + 0.000014 * t))
        + (2.0 * mr).sin() * (0.019993 - 0.000101 * t)
        + (3.0 * mr).sin() * 0.000289;
    let true_long = l0 + c;
    let omega = (125.04 - 1934.136 * t).to_radians();
    let lambda = (true_long - 0.00569 - 0.00478 * omega.sin()).to_radians();
    let seconds = 21.448 - t * (46.8150 + t * (0.00059 - t * 0.001813));
    let e0 = 23.0 + (26.0 + seconds / 60.0) / 60.0;
    let eps = (e0 + 0.00256 * omega.cos()).to_radians();
    let declination = (eps.sin() * lambda.sin()).asin();
    let y = (eps / 2.0).tan().powi(2);
    let l0r = l0.to_radians();
    let eq_time = 4.0
        * (y * (2.0 * l0r).sin() - 2.0 * e * mr.sin() + 4.0 * e * y * mr.sin() * (2.0 * l0r).cos()
            - 0.5 * y * y * (4.0 * l0r).sin()
            - 1.25 * e * e * (2.0 * mr).sin())
        .to_degrees();
    SolarPosition {
        declination,
        eq_time,
    }
}

/// Minutes after 00:00 UTC of the day starting at `jd0` when the sun's upper
/// edge crosses the horizon. `Err(true)` is polar day, `Err(false)` polar night.
fn event_minutes(jd0: f64, lat: f64, lon: f64, rise: bool) -> Result<f64, bool> {
    let mut minutes = 720.0 - 4.0 * lon;
    for _ in 0..3 {
        let pos = solar_position(jd0 + minutes / 1440.0);
        let latr = lat.to_radians();
        let arg = 90.833_f64.to_radians().cos() / (latr.cos() * pos.declination.cos())
            - latr.tan() * pos.declination.tan();
        if arg > 1.0 {
            return Err(false);
        }
        if arg < -1.0 {
            return Err(true);
        }
        let ha = arg.acos().to_degrees();
        let delta = lon + if rise { ha } else { -ha };
        minutes = 720.0 - 4.0 * delta - pos.eq_time;
    }
    Ok(minutes)
}

/// Sunrise (`rise`) or sunset on the local calendar day `date` in `tz`.
pub fn sun_event(date: NaiveDate, lat: f64, lon: f64, tz: Tz, rise: bool) -> SunEvent {
    let mut polar = None;
    let mut found = None;
    for offset in [-1i64, 0, 1] {
        let Some(d) = date.checked_add_signed(Duration::days(offset)) else {
            continue;
        };
        let jd0 = julian_day(d);
        match event_minutes(jd0, lat, lon, rise) {
            Ok(min) => {
                let midnight = Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).expect("midnight"));
                let t = midnight + Duration::milliseconds((min * 60_000.0).round() as i64);
                if found.is_none() && t.with_timezone(&tz).date_naive() == date {
                    found = Some(t);
                }
            }
            Err(day) if offset == 0 => polar = Some(day),
            Err(_) => {}
        }
    }
    match (found, polar) {
        (Some(t), _) => SunEvent::At(t),
        (None, Some(true)) => SunEvent::PolarDay,
        (None, Some(false)) => SunEvent::PolarNight,
        (None, None) => SunEvent::NoneToday,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_minutes(e: SunEvent, tz: Tz) -> (u32, u32) {
        use chrono::Timelike;
        let SunEvent::At(t) = e else {
            panic!("expected a time, got {e:?}")
        };
        let l = t.with_timezone(&tz);
        (l.hour(), l.minute())
    }

    /// Compare to a published local time (h, m) within 2 minutes.
    fn near(got: (u32, u32), want: (u32, u32)) {
        let g = i64::from(got.0 * 60 + got.1);
        let w = i64::from(want.0 * 60 + want.1);
        assert!((g - w).abs() <= 2, "got {got:?}, published {want:?}");
    }

    fn check(name: &str, y: i32, mo: u32, d: u32, rise: (u32, u32), set: (u32, u32)) {
        let (_, lat, lon, zone) = *place(name).unwrap();
        let tz: Tz = zone.parse().unwrap();
        let date = NaiveDate::from_ymd_opt(y, mo, d).unwrap();
        near(local_minutes(sun_event(date, lat, lon, tz, true), tz), rise);
        near(local_minutes(sun_event(date, lat, lon, tz, false), tz), set);
    }

    // The published values below were read from api.sunrise-sunset.org (which
    // returns times to the second or minute) on 9 Nov 2026. They are one
    // independent implementation, not an official almanac such as the US
    // Naval Observatory tables; I could not open those.

    #[test]
    fn mumbai_9_nov_2026() {
        // Published: sunrise 06:42:35, sunset 18:01:59 (IST).
        check("mumbai", 2026, 11, 9, (6, 43), (18, 2));
    }

    #[test]
    fn london_midsummer_2026() {
        // Published: sunrise 04:43, sunset 21:21 (BST, 21 June).
        check("london", 2026, 6, 21, (4, 43), (21, 21));
    }

    #[test]
    fn new_york_december_solstice_2026() {
        // Published: sunrise 07:16, sunset 16:31 (EST, 21 December).
        check("new york", 2026, 12, 21, (7, 16), (16, 31));
    }

    #[test]
    fn sydney_march_equinox_2026_is_the_day_before_in_utc() {
        // Published: sunrise 06:58, sunset 19:07 (AEDT, 20 March). Sunrise falls on
        // the previous UTC day, which the local-date search must handle.
        check("sydney", 2026, 3, 20, (6, 58), (19, 7));
    }

    #[test]
    fn tokyo_september_equinox_2026() {
        // Published: sunrise 05:29, sunset 17:37 (JST, 23 September).
        check("tokyo", 2026, 9, 23, (5, 29), (17, 37));
    }

    #[test]
    fn polar_night_and_midnight_sun() {
        let tz: Tz = "Europe/Oslo".parse().unwrap();
        let (_, lat, lon, _) = *place("tromso").unwrap();
        let dec = NaiveDate::from_ymd_opt(2026, 12, 21).unwrap();
        assert_eq!(sun_event(dec, lat, lon, tz, true), SunEvent::PolarNight);
        assert_eq!(sun_event(dec, lat, lon, tz, false), SunEvent::PolarNight);
        let jun = NaiveDate::from_ymd_opt(2026, 6, 21).unwrap();
        assert_eq!(sun_event(jun, lat, lon, tz, true), SunEvent::PolarDay);
    }

    #[test]
    fn the_table_is_sound() {
        assert!(PLACES.len() >= 60);
        for (name, lat, lon, zone) in PLACES {
            assert!((-90.0..=90.0).contains(lat), "{name}");
            assert!((-180.0..=180.0).contains(lon), "{name}");
            assert!(zone.parse::<Tz>().is_ok(), "{name}: {zone}");
        }
    }
}
