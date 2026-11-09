//! Extra forecast sources for `web_lookup`.
//!
//! [`FmiEcmwfForecaster`] is the global second weather source (decision D18a
//! gap): the ECMWF IFS model, served as a point forecast by the Finnish
//! Meteorological Institute's open data WFS (`opendata.fmi.fi`).
//!
//! - Licence: the FMI open data service is CC BY 4.0
//!   (<https://en.ilmatieteenlaitos.fi/open-data-licence>) and ECMWF's whole
//!   real-time catalogue has been CC BY 4.0 since October 2025
//!   (<https://www.ecmwf.int/en/about/media-centre/news/2025/ecmwf-makes-its-entire-real-time-catalogue-open-all>).
//!   Commercial use is allowed with attribution. No key.
//! - Limits (FMI open data manual): 20 000 download requests a day and 600 per
//!   5 minutes per user. One forecast needs one request.
//! - Shape: one GET returns hourly `Temperature` (degrees C) and
//!   `Precipitation1h` (mm in the hour) for any latitude/longitude on Earth.
//!   Hours the model run does not cover come back as `NaN` and are skipped.
//!
//! All network access goes through the [`Http`] trait, so tests use canned
//! replies and never touch the network.

use crate::web_lookup::{DayForecast, Forecaster, Http, HttpError, Place};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use regex::Regex;
use std::collections::BTreeMap;

/// Minutes between samples asked for. Hourly keeps the rain totals and the
/// daily high and low close to the hourly MET Norway series.
const STEP_MINUTES: i64 = 60;
/// How many days ahead to ask for (enough for "this weekend").
const SPAN_DAYS: i64 = 6;
/// A day with fewer temperature samples than this is a cut-off edge of the
/// window, not a day to report.
const MIN_SAMPLES_PER_DAY: usize = (12 * 60 / STEP_MINUTES) as usize;

/// Credit asked for by CC BY 4.0 (FMI republishes ECMWF data).
pub const FMI_ECMWF_CREDIT: &str =
    "(Data from ECMWF via the Finnish Meteorological Institute, CC BY 4.0)";

/// ECMWF IFS point forecast through the FMI open data WFS.
pub struct FmiEcmwfForecaster;

impl Forecaster for FmiEcmwfForecaster {
    fn id(&self) -> &'static str {
        "fmi_ecmwf"
    }

    fn credit(&self) -> &'static str {
        FMI_ECMWF_CREDIT
    }

    fn days(
        &self,
        http: &dyn Http,
        at: &Place,
        offset_secs: i64,
    ) -> Result<Vec<DayForecast>, HttpError> {
        let url = fmi_url(at, offset_secs, Utc::now());
        daily_from_fmi(&http.get(&url)?, offset_secs).ok_or(HttpError::NotFound)
    }
}

/// The request: from the start of the local day (so "today" has its early
/// hours too) for [`SPAN_DAYS`] days.
fn fmi_url(at: &Place, offset_secs: i64, now: DateTime<Utc>) -> String {
    let local_today = (now + Duration::seconds(offset_secs)).date_naive();
    let start = local_today
        .and_hms_opt(0, 0, 0)
        .map(|t| t.and_utc() - Duration::seconds(offset_secs))
        .unwrap_or(now);
    let end = start + Duration::days(SPAN_DAYS);
    format!(
        "https://opendata.fmi.fi/wfs?service=WFS&version=2.0.0&request=getFeature\
         &storedquery_id=ecmwf::forecast::surface::point::simple\
         &latlon={:.4},{:.4}&timestep={STEP_MINUTES}\
         &parameters=Temperature,Precipitation1h&starttime={}&endtime={}",
        at.lat,
        at.lon,
        start.format("%Y-%m-%dT%H:%M:%SZ"),
        end.format("%Y-%m-%dT%H:%M:%SZ"),
    )
}

/// Turn the FMI simple-feature XML into local days.
///
/// Each `wfs:member` holds one `Time`, `ParameterName`, `ParameterValue`.
/// Temperature is already in degrees C. `Precipitation1h` is the rain (mm) in
/// the hour ending at `Time`, so each sample is counted into the day that
/// hour mostly belongs to (the sample time minus half a step), and samples are
/// scaled by the step length in hours should the step ever not be one hour.
fn daily_from_fmi(body: &str, offset_secs: i64) -> Option<Vec<DayForecast>> {
    let member = Regex::new(
        r"(?s)<(?:\w+:)?Time>\s*([^<\s]+)\s*</(?:\w+:)?Time>\s*<(?:\w+:)?ParameterName>\s*([^<\s]+)\s*</(?:\w+:)?ParameterName>\s*<(?:\w+:)?ParameterValue>\s*([^<\s]+)\s*</(?:\w+:)?ParameterValue>",
    )
    .ok()?;

    let mut temps: BTreeMap<DateTime<Utc>, f64> = BTreeMap::new();
    let mut rains: BTreeMap<DateTime<Utc>, f64> = BTreeMap::new();
    for c in member.captures_iter(body) {
        let Ok(time) = DateTime::parse_from_rfc3339(&c[1]) else {
            continue;
        };
        let time = time.with_timezone(&Utc);
        // "NaN" parses as a number, so the finite check is what skips it.
        let Some(value) = c[3].parse::<f64>().ok().filter(|v| v.is_finite()) else {
            continue;
        };
        match &c[2] {
            "Temperature" => {
                temps.insert(time, value);
            }
            "Precipitation1h" => {
                rains.insert(time, value.max(0.0));
            }
            _ => {}
        }
    }
    if temps.is_empty() {
        return None;
    }

    let local_date = |t: DateTime<Utc>| -> Option<NaiveDate> {
        t.checked_add_signed(Duration::seconds(offset_secs))
            .map(|l| l.date_naive())
    };
    struct Acc {
        high: f64,
        low: f64,
        samples: usize,
        rain_mm: f64,
    }
    let mut days: BTreeMap<NaiveDate, Acc> = BTreeMap::new();
    for (&t, &temp) in &temps {
        let Some(date) = local_date(t) else { continue };
        let d = days.entry(date).or_insert(Acc {
            high: f64::NEG_INFINITY,
            low: f64::INFINITY,
            samples: 0,
            rain_mm: 0.0,
        });
        d.high = d.high.max(temp);
        d.low = d.low.min(temp);
        d.samples += 1;
    }
    let step_hours = STEP_MINUTES as f64 / 60.0;
    for (&t, &mm) in &rains {
        let mid = t - Duration::minutes(STEP_MINUTES / 2);
        if let Some(d) = local_date(mid).and_then(|date| days.get_mut(&date)) {
            d.rain_mm += mm * step_hours;
        }
    }

    let out: Vec<DayForecast> = days
        .into_iter()
        .filter(|(_, d)| d.samples >= MIN_SAMPLES_PER_DAY)
        .map(|(date, d)| DayForecast {
            date,
            high: d.high,
            low: d.low,
            rain_mm: d.rain_mm,
            rain_chance: None,
        })
        .collect();
    (!out.is_empty()).then_some(out)
}

// ---------------------------------------------------------------------------
// Tests (none touch the network except the ignored live test)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web_lookup::{MetNoForecaster, WebLookup};
    use crate::{Args, Connector};
    use std::sync::{Arc, Mutex};

    struct FakeHttp {
        routes: Vec<(&'static str, Result<String, HttpError>)>,
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl Http for FakeHttp {
        fn get(&self, url: &str) -> Result<String, HttpError> {
            self.seen.lock().unwrap().push(url.to_string());
            for (needle, reply) in &self.routes {
                if url.contains(needle) {
                    return reply.clone();
                }
            }
            Err(HttpError::Other("no route".into()))
        }
    }

    /// One `wfs:member` laid out the way the real service prints it.
    fn member(time: &str, name: &str, value: &str) -> String {
        format!(
            "\t<wfs:member>\n            <BsWfs:BsWfsElement gml:id=\"BsWfsElement.1.1.1\">\n\
             \x20               <BsWfs:Location>\n                    <gml:Point gml:id=\"P.1\" srsDimension=\"2\">\n\
             \x20                       <gml:pos>18.52000 73.86000 </gml:pos>\n                    </gml:Point>\n\
             \x20               </BsWfs:Location>\n                <BsWfs:Time>{time}</BsWfs:Time>\n\
             \x20               <BsWfs:ParameterName>{name}</BsWfs:ParameterName>\n\
             \x20               <BsWfs:ParameterValue>{value}</BsWfs:ParameterValue>\n\
             \x20           </BsWfs:BsWfsElement>\n\t</wfs:member>\n"
        )
    }

    /// Hourly series for UTC days: (temp, rain) per hour from `f(hour_index)`.
    fn feed(first: &str, hours: usize, f: impl Fn(usize) -> (String, String)) -> String {
        let start: DateTime<Utc> = first.parse().unwrap();
        let mut s =
            String::from("<?xml version=\"1.0\"?>\n<wfs:FeatureCollection numberReturned=\"1\">\n");
        for h in 0..hours {
            let t = (start + Duration::hours(h as i64))
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string();
            let (temp, rain) = f(h);
            s += &member(&t, "Temperature", &temp);
            s += &member(&t, "Precipitation1h", &rain);
        }
        s + "</wfs:FeatureCollection>\n"
    }

    fn place() -> Place {
        Place {
            name: "Pune".into(),
            lat: 18.5196,
            lon: 73.8553,
        }
    }

    #[test]
    fn groups_hours_into_local_days_with_the_offset() {
        // Pune solar offset is +5 h, so local midnight is 19:00 UTC. The feed
        // runs from local 2026-11-05 00:00 for 48 h; temperature is the local
        // hour of day (0..23) so the high is 23 and the low 0 on both days.
        let body = feed("2026-11-04T19:00:00Z", 49, |h| {
            (format!("{}", (h % 24) as f64), "0.0".into())
        });
        let days = daily_from_fmi(&body, 5 * 3600).unwrap();
        // 48 h make two full days; the 49th sample (next local 00:00) is a lone
        // edge sample and is dropped.
        assert_eq!(days.len(), 2, "{days:?}");
        assert_eq!(days[0].date, NaiveDate::from_ymd_opt(2026, 11, 5).unwrap());
        assert_eq!(days[1].date, NaiveDate::from_ymd_opt(2026, 11, 6).unwrap());
        for d in &days {
            assert_eq!((d.high, d.low), (23.0, 0.0));
            assert_eq!(d.rain_chance, None);
        }
    }

    #[test]
    fn rain_is_summed_per_local_day_and_the_hour_before_midnight_counts_to_that_day() {
        // 1 mm in every hour on UTC day 1 only (offset 0). The sample stamped
        // 00:00 on day 2 is the hour 23:00-24:00 of day 1 and counts there.
        let body = feed("2026-11-05T00:00:00Z", 49, |h| {
            (
                "10.0".into(),
                if (1..=24).contains(&h) { "1.0" } else { "0.0" }.into(),
            )
        });
        let days = daily_from_fmi(&body, 0).unwrap();
        assert_eq!(days.len(), 2);
        assert!((days[0].rain_mm - 24.0).abs() < 1e-9, "{}", days[0].rain_mm);
        assert!(days[1].rain_mm.abs() < 1e-9);
    }

    #[test]
    fn nan_and_missing_hours_are_skipped_and_edges_are_dropped() {
        // First 3 hours NaN (model run does not cover them), then real data.
        let body = feed("2026-11-05T00:00:00Z", 30, |h| {
            if h < 3 {
                ("NaN".into(), "NaN".into())
            } else {
                ("20.5".into(), "0.5".into())
            }
        });
        let days = daily_from_fmi(&body, 0).unwrap();
        // Day 1 has 21 temperature samples (kept); day 2 only 6 (dropped). 22 rain hours count to day 1: the sample stamped 00:00 is the hour before it.
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].high, 20.5);
        assert!((days[0].rain_mm - 0.5 * 22.0).abs() < 1e-9);
        // Negative or garbage rain never subtracts.
        let body = feed("2026-11-05T00:00:00Z", 24, |_| ("5".into(), "-3".into()));
        assert_eq!(daily_from_fmi(&body, 0).unwrap()[0].rain_mm, 0.0);
    }

    #[test]
    fn bad_replies_are_not_found_and_never_panic() {
        for body in [
            "",
            "not xml",
            "{}",
            "<wfs:FeatureCollection></wfs:FeatureCollection>",
            // The service's error report for a point off the map.
            "<ExceptionReport><Exception exceptionCode=\"OperationParsingFailed\"><ExceptionText>out of bounds</ExceptionText></Exception></ExceptionReport>",
            // All NaN.
            &feed("2026-11-05T00:00:00Z", 24, |_| ("NaN".into(), "NaN".into())),
            // Bad time.
            &member("yesterday", "Temperature", "5.0"),
        ] {
            assert_eq!(daily_from_fmi(body, 0), None, "{body}");
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let http = FakeHttp {
            routes: vec![("", Ok("garbage".into()))],
            seen,
        };
        assert_eq!(
            FmiEcmwfForecaster.days(&http, &place(), 0),
            Err(HttpError::NotFound)
        );
        let http = FakeHttp {
            routes: vec![("", Err(HttpError::Offline))],
            seen: Default::default(),
        };
        assert_eq!(
            FmiEcmwfForecaster.days(&http, &place(), 0),
            Err(HttpError::Offline)
        );
    }

    #[test]
    fn the_request_names_only_coordinates_and_a_window_from_local_midnight() {
        let now: DateTime<Utc> = "2026-11-09T04:30:00Z".parse().unwrap();
        // Sydney, +10 h: local date is 9 Nov 14:30, so local midnight is 8 Nov 14:00 UTC.
        let sydney = Place {
            name: "Sydney".into(),
            lat: -33.86883,
            lon: 151.20929,
        };
        let url = fmi_url(&sydney, 10 * 3600, now);
        assert!(url.starts_with("https://opendata.fmi.fi/wfs?"), "{url}");
        assert!(url.contains("latlon=-33.8688,151.2093"), "{url}");
        assert!(url.contains("starttime=2026-11-08T14:00:00Z"), "{url}");
        assert!(url.contains("endtime=2026-11-14T14:00:00Z"), "{url}");
        assert!(url.contains("storedquery_id=ecmwf::forecast::surface::point::simple"));
        assert!(url.contains("timestep=60"));
        // Nothing but numbers in the position: no place name leaves the device.
        assert!(!url.contains("Sydney"));

        let http = FakeHttp {
            routes: vec![(
                "opendata.fmi.fi",
                Ok(feed("2026-11-05T00:00:00Z", 24, |_| {
                    ("9".into(), "0".into())
                })),
            )],
            seen: Default::default(),
        };
        let seen = http.seen.clone();
        FmiEcmwfForecaster.days(&http, &place(), 0).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn credit_and_id() {
        assert_eq!(FmiEcmwfForecaster.id(), "fmi_ecmwf");
        assert!(FmiEcmwfForecaster.credit().contains("ECMWF"));
        assert!(FmiEcmwfForecaster.credit().contains("CC BY 4.0"));
    }

    const GEO: &str = r#"{"query":{"pages":[
        {"pageid":1,"title":"Pune","index":1,"coordinates":[{"lat":18.5196,"lon":73.8553,"primary":true}]}]}}"#;

    fn args(text: &str) -> Args {
        [("text".to_string(), text.to_string())].into()
    }

    #[test]
    fn answers_when_met_norway_is_down() {
        // Local day 2026-11-05 in Pune (+5 h): 4 mm of rain, 21..29 degrees.
        let body = feed("2026-11-04T19:00:00Z", 72, |h| {
            let local_day = h / 24;
            let hour = (h % 24) as f64;
            let temp = 21.0 + (hour / 23.0) * 8.0;
            let rain = if local_day == 1 && (1.0..=4.0).contains(&hour) {
                1.0
            } else {
                0.0
            };
            (format!("{temp:.1}"), format!("{rain}"))
        });
        let http = FakeHttp {
            routes: vec![
                ("prop=coordinates", Ok(GEO.into())),
                ("api.met.no", Err(HttpError::Other("503".into()))),
                ("opendata.fmi.fi", Ok(body)),
            ],
            seen: Default::default(),
        };
        let seen = http.seen.clone();
        let c = WebLookup::with_http(Box::new(http), Some("Pune".into()))
            .with_clock(|| "2026-11-04T10:00:00Z".parse().unwrap())
            .with_forecaster(Box::new(FmiEcmwfForecaster));
        let out = c
            .execute("weather", &args("will it rain in Pune tomorrow"))
            .unwrap();
        assert_eq!(
            out.0["message"],
            "Rain is likely in Pune tomorrow: about 4.0 mm of rain, high 29\u{b0}C, low 21\u{b0}C. (Data from ECMWF via the Finnish Meteorological Institute, CC BY 4.0)"
        );
        let seen = seen.lock().unwrap();
        assert!(seen.iter().any(|u| u.contains("api.met.no")));
        assert!(seen.iter().all(|u| !u.to_lowercase().contains("will")));

        // The `lookup_providers` setting can put it first, by its id.
        let http = FakeHttp {
            routes: vec![
                ("prop=coordinates", Ok(GEO.into())),
                (
                    "opendata.fmi.fi",
                    Ok(feed("2026-11-04T19:00:00Z", 72, |_| {
                        ("20".into(), "0".into())
                    })),
                ),
            ],
            seen: Default::default(),
        };
        let seen = http.seen.clone();
        let c = WebLookup::with_http(Box::new(http), Some("Pune".into()))
            .with_clock(|| "2026-11-04T10:00:00Z".parse().unwrap())
            .with_forecaster(Box::new(FmiEcmwfForecaster))
            .with_config_source(|| Some("weather=fmi_ecmwf".into()));
        let out = c.execute("weather", &args("weather tomorrow")).unwrap();
        assert!(out.0["message"]
            .ends_with("(Data from ECMWF via the Finnish Meteorological Institute, CC BY 4.0)"));
        assert!(!seen
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.contains("api.met.no")));
    }

    /// Prints the FMI/ECMWF forecast next to MET Norway for three cities.
    /// Run: cargo test -p jitmo-connectors live_vs_metno -- --ignored --nocapture
    #[test]
    #[ignore = "uses the network"]
    fn live_vs_metno() {
        let http = crate::web_lookup::UreqHttp::new();
        for (name, lat, lon, offset_h) in [
            ("London", 51.5072, -0.1276, 0),
            ("Pune", 18.5196, 73.8553, 5),
            ("Sydney", -33.8688, 151.2093, 10),
        ] {
            let at = Place {
                name: name.into(),
                lat,
                lon,
            };
            let off = offset_h * 3600;
            let t = std::time::Instant::now();
            let fmi = FmiEcmwfForecaster.days(&http, &at, off);
            let ms = t.elapsed().as_millis();
            let t = std::time::Instant::now();
            let met = MetNoForecaster.days(&http, &at, off);
            let ms_met = t.elapsed().as_millis();
            println!("== {name}: FMI/ECMWF {ms} ms, MET Norway {ms_met} ms");
            let (fmi, met) = (fmi.expect("fmi"), met.expect("met"));
            assert!(!fmi.is_empty());
            for f in fmi.iter().take(6) {
                let m = met.iter().find(|m| m.date == f.date);
                println!(
                    "{} ecmwf high {:5.1} low {:5.1} rain {:5.1} mm | metno {}",
                    f.date,
                    f.high,
                    f.low,
                    f.rain_mm,
                    m.map(|m| format!(
                        "high {:5.1} low {:5.1} rain {:5.1} mm",
                        m.high, m.low, m.rain_mm
                    ))
                    .unwrap_or_else(|| "-".into())
                );
            }
        }
    }
}
