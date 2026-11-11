//! Time-zone resolution for date-time answers without an offset
//! (`fsv.forms.tz`).
//!
//! Order: explicit `tz` > `FSV_TZ` > the Freshservice user's `time_zone`
//! (from the business_rules response `user` object) > the system time zone.
//! Freshservice stores Rails/ActiveSupport zone names ("Bangkok", "Eastern
//! Time (US & Canada)"); they are mapped to IANA names here.

use jiff::civil::DateTime;
use jiff::tz::{AmbiguousOffset, Offset, TimeZone};
use serde_json::{Map, Value as Json};

use crate::errors::{FormError, Result};
use crate::pydt::{DateTimeValue, offset_text};
use crate::pytext::{jstr, strip};

/// `ActiveSupport::TimeZone::MAPPING` (Rails zone name -> IANA).
pub const RAILS_ZONES: [(&str, &str); 152] = [
    ("International Date Line West", "Etc/GMT+12"),
    ("Midway Island", "Pacific/Midway"),
    ("American Samoa", "Pacific/Pago_Pago"),
    ("Hawaii", "Pacific/Honolulu"),
    ("Alaska", "America/Juneau"),
    ("Pacific Time (US & Canada)", "America/Los_Angeles"),
    ("Tijuana", "America/Tijuana"),
    ("Mountain Time (US & Canada)", "America/Denver"),
    ("Arizona", "America/Phoenix"),
    ("Chihuahua", "America/Chihuahua"),
    ("Mazatlan", "America/Mazatlan"),
    ("Central Time (US & Canada)", "America/Chicago"),
    ("Saskatchewan", "America/Regina"),
    ("Guadalajara", "America/Mexico_City"),
    ("Mexico City", "America/Mexico_City"),
    ("Monterrey", "America/Monterrey"),
    ("Central America", "America/Guatemala"),
    ("Eastern Time (US & Canada)", "America/New_York"),
    ("Indiana (East)", "America/Indiana/Indianapolis"),
    ("Bogota", "America/Bogota"),
    ("Lima", "America/Lima"),
    ("Quito", "America/Lima"),
    ("Atlantic Time (Canada)", "America/Halifax"),
    ("Caracas", "America/Caracas"),
    ("La Paz", "America/La_Paz"),
    ("Santiago", "America/Santiago"),
    ("Newfoundland", "America/St_Johns"),
    ("Brasilia", "America/Sao_Paulo"),
    ("Buenos Aires", "America/Argentina/Buenos_Aires"),
    ("Montevideo", "America/Montevideo"),
    ("Georgetown", "America/Guyana"),
    ("Puerto Rico", "America/Puerto_Rico"),
    ("Greenland", "America/Godthab"),
    ("Mid-Atlantic", "Atlantic/South_Georgia"),
    ("Azores", "Atlantic/Azores"),
    ("Cape Verde Is.", "Atlantic/Cape_Verde"),
    ("Dublin", "Europe/Dublin"),
    ("Edinburgh", "Europe/London"),
    ("Lisbon", "Europe/Lisbon"),
    ("London", "Europe/London"),
    ("Casablanca", "Africa/Casablanca"),
    ("Monrovia", "Africa/Monrovia"),
    ("UTC", "Etc/UTC"),
    ("Belgrade", "Europe/Belgrade"),
    ("Bratislava", "Europe/Bratislava"),
    ("Budapest", "Europe/Budapest"),
    ("Ljubljana", "Europe/Ljubljana"),
    ("Prague", "Europe/Prague"),
    ("Sarajevo", "Europe/Sarajevo"),
    ("Skopje", "Europe/Skopje"),
    ("Warsaw", "Europe/Warsaw"),
    ("Zagreb", "Europe/Zagreb"),
    ("Brussels", "Europe/Brussels"),
    ("Copenhagen", "Europe/Copenhagen"),
    ("Madrid", "Europe/Madrid"),
    ("Paris", "Europe/Paris"),
    ("Amsterdam", "Europe/Amsterdam"),
    ("Berlin", "Europe/Berlin"),
    ("Bern", "Europe/Zurich"),
    ("Zurich", "Europe/Zurich"),
    ("Rome", "Europe/Rome"),
    ("Stockholm", "Europe/Stockholm"),
    ("Vienna", "Europe/Vienna"),
    ("West Central Africa", "Africa/Algiers"),
    ("Bucharest", "Europe/Bucharest"),
    ("Cairo", "Africa/Cairo"),
    ("Helsinki", "Europe/Helsinki"),
    ("Kyiv", "Europe/Kiev"),
    ("Kyev", "Europe/Kiev"),
    ("Riga", "Europe/Riga"),
    ("Sofia", "Europe/Sofia"),
    ("Tallinn", "Europe/Tallinn"),
    ("Vilnius", "Europe/Vilnius"),
    ("Athens", "Europe/Athens"),
    ("Istanbul", "Europe/Istanbul"),
    ("Minsk", "Europe/Minsk"),
    ("Jerusalem", "Asia/Jerusalem"),
    ("Harare", "Africa/Harare"),
    ("Pretoria", "Africa/Johannesburg"),
    ("Kaliningrad", "Europe/Kaliningrad"),
    ("Moscow", "Europe/Moscow"),
    ("St. Petersburg", "Europe/Moscow"),
    ("Volgograd", "Europe/Volgograd"),
    ("Samara", "Europe/Samara"),
    ("Kuwait", "Asia/Kuwait"),
    ("Riyadh", "Asia/Riyadh"),
    ("Nairobi", "Africa/Nairobi"),
    ("Baghdad", "Asia/Baghdad"),
    ("Tehran", "Asia/Tehran"),
    ("Abu Dhabi", "Asia/Muscat"),
    ("Muscat", "Asia/Muscat"),
    ("Baku", "Asia/Baku"),
    ("Tbilisi", "Asia/Tbilisi"),
    ("Yerevan", "Asia/Yerevan"),
    ("Kabul", "Asia/Kabul"),
    ("Ekaterinburg", "Asia/Yekaterinburg"),
    ("Islamabad", "Asia/Karachi"),
    ("Karachi", "Asia/Karachi"),
    ("Tashkent", "Asia/Tashkent"),
    ("Chennai", "Asia/Kolkata"),
    ("Kolkata", "Asia/Kolkata"),
    ("Mumbai", "Asia/Kolkata"),
    ("New Delhi", "Asia/Kolkata"),
    ("Kathmandu", "Asia/Kathmandu"),
    ("Astana", "Asia/Dhaka"),
    ("Dhaka", "Asia/Dhaka"),
    ("Sri Jayawardenepura", "Asia/Colombo"),
    ("Almaty", "Asia/Almaty"),
    ("Novosibirsk", "Asia/Novosibirsk"),
    ("Rangoon", "Asia/Rangoon"),
    ("Bangkok", "Asia/Bangkok"),
    ("Hanoi", "Asia/Bangkok"),
    ("Jakarta", "Asia/Jakarta"),
    ("Krasnoyarsk", "Asia/Krasnoyarsk"),
    ("Beijing", "Asia/Shanghai"),
    ("Chongqing", "Asia/Chongqing"),
    ("Hong Kong", "Asia/Hong_Kong"),
    ("Urumqi", "Asia/Urumqi"),
    ("Kuala Lumpur", "Asia/Kuala_Lumpur"),
    ("Singapore", "Asia/Singapore"),
    ("Taipei", "Asia/Taipei"),
    ("Perth", "Australia/Perth"),
    ("Irkutsk", "Asia/Irkutsk"),
    ("Ulaanbaatar", "Asia/Ulaanbaatar"),
    ("Seoul", "Asia/Seoul"),
    ("Osaka", "Asia/Tokyo"),
    ("Sapporo", "Asia/Tokyo"),
    ("Tokyo", "Asia/Tokyo"),
    ("Yakutsk", "Asia/Yakutsk"),
    ("Darwin", "Australia/Darwin"),
    ("Adelaide", "Australia/Adelaide"),
    ("Canberra", "Australia/Melbourne"),
    ("Melbourne", "Australia/Melbourne"),
    ("Sydney", "Australia/Sydney"),
    ("Brisbane", "Australia/Brisbane"),
    ("Hobart", "Australia/Hobart"),
    ("Vladivostok", "Asia/Vladivostok"),
    ("Guam", "Pacific/Guam"),
    ("Port Moresby", "Pacific/Port_Moresby"),
    ("Magadan", "Asia/Magadan"),
    ("Srednekolymsk", "Asia/Srednekolymsk"),
    ("Solomon Is.", "Pacific/Guadalcanal"),
    ("New Caledonia", "Pacific/Noumea"),
    ("Fiji", "Pacific/Fiji"),
    ("Kamchatka", "Asia/Kamchatka"),
    ("Marshall Is.", "Pacific/Majuro"),
    ("Auckland", "Pacific/Auckland"),
    ("Wellington", "Pacific/Auckland"),
    ("Nuku'alofa", "Pacific/Tongatapu"),
    ("Tokelau Is.", "Pacific/Fakaofo"),
    ("Chatham Is.", "Pacific/Chatham"),
    ("Samoa", "Pacific/Apia"),
];

/// A resolved zone and its Python `str()` (`ZoneInfo.key`, or the fixed
/// offset's name).
#[derive(Clone, Debug)]
pub struct Zone {
    pub tz: TimeZone,
    pub name: String,
}

impl Zone {
    /// A fixed UTC offset (`datetime.timezone(...)`), named like Python.
    pub fn fixed(offset: Offset) -> Zone {
        let name = if offset.seconds() == 0 {
            "UTC".to_string()
        } else {
            format!("UTC{}", offset_text(offset))
        };
        Zone {
            tz: TimeZone::fixed(offset),
            name,
        }
    }

    /// The offset this zone gives an instant.
    pub fn offset_at(&self, ts: jiff::Timestamp) -> Offset {
        self.tz.to_offset(ts)
    }
}

impl PartialEq for Zone {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

/// `tz_name(tz)`.
pub fn tz_name(tz: &Zone) -> &str {
    &tz.name
}

fn rails(name: &str) -> Option<&'static str> {
    RAILS_ZONES
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| *v)
}

fn load(name: &str) -> Option<TimeZone> {
    if name.is_empty()
        || name.starts_with('/')
        || name
            .split('/')
            .any(|p| p == ".." || p == "." || p.is_empty())
    {
        return None;
    }
    TimeZone::get(name).ok()
}

/// IANA or Rails zone name -> zone (`None` if unknown or empty).
pub fn zone(name: &str) -> Option<Zone> {
    let name = strip(name);
    if name.is_empty() {
        return None;
    }
    for cand in [Some(name), rails(name)].into_iter().flatten() {
        if let Some(tz) = load(cand) {
            return Some(Zone {
                tz,
                name: cand.to_string(),
            });
        }
    }
    None
}

/// The process environment `resolve_timezone` reads; injectable so tests
/// do not touch real env vars.
#[derive(Clone, Debug, Default)]
pub struct TzEnv {
    /// `FSV_TZ`.
    pub fsv_tz: Option<String>,
    /// `TZ`.
    pub tz: Option<String>,
    /// `os.path.realpath("/etc/localtime")`.
    pub localtime: Option<String>,
}

impl TzEnv {
    /// Read `FSV_TZ`, `TZ` and resolve `/etc/localtime`.
    pub fn from_process() -> TzEnv {
        TzEnv {
            fsv_tz: std::env::var("FSV_TZ").ok(),
            tz: std::env::var("TZ").ok(),
            localtime: std::fs::canonicalize("/etc/localtime")
                .ok()
                .map(|p| p.to_string_lossy().into_owned()),
        }
    }

    /// No variables set and no `/etc/localtime` (falls back to jiff's system zone).
    pub fn empty() -> TzEnv {
        TzEnv::default()
    }

    /// Only `TZ` set (hermetic tests: `TzEnv::with_tz("UTC")`).
    pub fn with_tz(tz: &str) -> TzEnv {
        TzEnv {
            tz: Some(tz.to_string()),
            ..TzEnv::default()
        }
    }
}

/// The machine's zone: `TZ`, then `/etc/localtime`, then the current offset.
pub fn system_timezone(env: &TzEnv) -> Zone {
    if let Some(z) = zone(env.tz.as_deref().unwrap_or("").trim_start_matches(':')) {
        return z;
    }
    if let Some(target) = &env.localtime
        && let Some((_, rest)) = target.split_once("zoneinfo/")
        && let Some(z) = zone(rest)
    {
        return z;
    }
    let sys = TimeZone::system();
    match sys.iana_name() {
        Some(name) => Zone {
            name: name.to_string(),
            tz: sys,
        },
        None => Zone::fixed(sys.to_offset(jiff::Timestamp::now())),
    }
}

/// (zone, where it came from) for date-times entered without an offset,
/// reading the real process environment.
pub fn resolve_timezone(
    explicit: Option<&str>,
    user: Option<&Map<String, Json>>,
) -> Result<(Zone, String)> {
    resolve_timezone_in(explicit, user, &TzEnv::from_process())
}

/// [`resolve_timezone`] with an explicit environment.
pub fn resolve_timezone_in(
    explicit: Option<&str>,
    user: Option<&Map<String, Json>>,
    env: &TzEnv,
) -> Result<(Zone, String)> {
    for (value, source) in [(explicit, "tz argument"), (env.fsv_tz.as_deref(), "FSV_TZ")] {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            return match zone(v) {
                Some(z) => Ok((z, source.to_string())),
                None => Err(FormError::Value(format!(
                    "unknown time zone {} ({source})",
                    fsv_py::py_repr_str(v)
                ))),
            };
        }
    }
    if let Some(user) = user.filter(|u| !u.is_empty()) {
        let raw = user.get("time_zone").unwrap_or(&Json::Null);
        if !raw.is_null()
            && let Some(z) = zone(&jstr(raw))
        {
            return Ok((z, format!("Freshservice user time zone ({})", jstr(raw))));
        }
    }
    Ok((system_timezone(env), "system time zone".to_string()))
}

/// Attach `tz` to a wall-clock time, refusing times a daylight-saving
/// change makes ambiguous: a time that does not exist (the clocks skip it)
/// or that happens twice (the clocks go back) is an error naming the
/// offsets to choose from. Fixed-offset zones never fail.
pub fn localize(naive: DateTime, tz: &Zone) -> std::result::Result<DateTimeValue, String> {
    let when = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        naive.year(),
        naive.month(),
        naive.day(),
        naive.hour(),
        naive.minute()
    );
    match tz.tz.to_ambiguous_timestamp(naive).offset() {
        AmbiguousOffset::Unambiguous { offset } => Ok(DateTimeValue::new(naive, offset)),
        AmbiguousOffset::Gap { before, after } => Err(format!(
            "{when} does not exist in {} (the clocks skip ahead then); type another time, or add an offset ({} or {})",
            tz.name,
            offset_text(before),
            offset_text(after)
        )),
        AmbiguousOffset::Fold { before, after } => Err(format!(
            "{when} happens twice in {} (the clocks go back then); add the offset: {} for the first, {} for the second",
            tz.name,
            offset_text(before),
            offset_text(after)
        )),
    }
}

/// The offset `tz` gives wall time `local` with `fold=0` (the earlier
/// offset for gaps and overlaps), as `local.replace(tzinfo=tz).utcoffset()`.
pub fn fold0_offset(local: DateTime, tz: &Zone) -> Offset {
    match tz.tz.to_ambiguous_timestamp(local).offset() {
        AmbiguousOffset::Unambiguous { offset } => offset,
        AmbiguousOffset::Gap { before, .. } | AmbiguousOffset::Fold { before, .. } => before,
    }
}

#[cfg(test)]
mod py_tests;
