//! A small reader for the iCalendar feed Proton serves from a calendar's share
//! link: line unfolding, properties, VEVENTs, and recurrence expansion. RRULE,
//! RDATE and EXDATE go through the `rrule` crate; RECURRENCE-ID overrides
//! (one moved, edited or cancelled occurrence) are applied here.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};

use crate::content::{Invalid, clean};
use mail_parser::decoders::html::html_to_text;

/// Most occurrences one event can contribute to one query window.
const MAX_OCCURRENCES: u16 = 2000;
/// Longest DURATION accepted: a century. Anything longer is refused, which also
/// keeps occurrence arithmetic far inside chrono's range, where it cannot panic.
const MAX_DURATION_DAYS: i64 = 36_600;
/// How far daylight saving can stretch an all-day occurrence past its nominal length.
const DST_SLACK: Duration = Duration::hours(3);

/// The zone for floating times and for output: IANA, or the Mac's own.
#[derive(Clone, Copy, Debug)]
pub enum Zone {
    Iana(chrono_tz::Tz),
    Local,
}

impl Zone {
    pub fn parse(name: Option<&str>) -> Result<Self> {
        match name {
            None => Ok(Self::Local),
            Some(n) => n.parse().map(Self::Iana).map_err(|e| {
                Invalid::quoting(
                    "The time zone is unknown; use an IANA name like America/Los_Angeles.",
                    format!(
                        "unknown time zone {n:?}; use an IANA name like America/Los_Angeles: {e}"
                    ),
                )
                .into()
            }),
        }
    }

    fn rrule_tz(self) -> rrule::Tz {
        match self {
            Self::Iana(tz) => tz.into(),
            Self::Local => rrule::Tz::LOCAL,
        }
    }

    pub fn to_utc(self, local: NaiveDateTime) -> DateTime<Utc> {
        match self {
            Self::Iana(tz) => resolve(&tz, local),
            Self::Local => resolve(&chrono::Local, local),
        }
    }

    pub fn format(self, t: DateTime<Utc>) -> String {
        match self {
            Self::Iana(tz) => t.with_timezone(&tz).to_rfc3339(),
            Self::Local => t.with_timezone(&chrono::Local).to_rfc3339(),
        }
    }

    /// The wall-clock time `t` shows in this zone.
    pub fn wall(self, t: DateTime<Utc>) -> NaiveDateTime {
        match self {
            Self::Iana(tz) => t.with_timezone(&tz).naive_local(),
            Self::Local => t.with_timezone(&chrono::Local).naive_local(),
        }
    }
}

/// Local time to UTC: the earlier reading of an ambiguous time, and a step
/// forward over a daylight-saving gap.
fn resolve<Z: TimeZone>(tz: &Z, local: NaiveDateTime) -> DateTime<Utc> {
    (0..3)
        .find_map(|h| {
            tz.from_local_datetime(&(local + Duration::hours(h)))
                .earliest()
        })
        .map_or_else(|| Utc.from_utc_datetime(&local), |t| t.with_timezone(&Utc))
}

/// A DTSTART/DTEND/EXDATE/RDATE/RECURRENCE-ID value.
#[derive(Clone, Debug, PartialEq)]
pub enum When {
    Date(NaiveDate),
    Utc(DateTime<Utc>),
    Zoned(NaiveDateTime, chrono_tz::Tz),
    Floating(NaiveDateTime),
}

impl When {
    fn parse(value: &str, params: &[(String, String)]) -> Result<Self> {
        let v = value.trim();
        let date_valued = param(params, "VALUE").is_some_and(|x| x.eq_ignore_ascii_case("DATE"));
        if date_valued || v.len() == 8 {
            return Ok(Self::Date(NaiveDate::parse_from_str(v, "%Y%m%d")?));
        }
        if let Some(utc) = v.strip_suffix('Z') {
            let t = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S")?;
            return Ok(Self::Utc(Utc.from_utc_datetime(&t)));
        }
        let t = NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S")?;
        Ok(match param(params, "TZID").and_then(parse_tzid) {
            Some(tz) => Self::Zoned(t, tz),
            None => Self::Floating(t),
        })
    }

    pub fn utc(&self, zone: Zone) -> DateTime<Utc> {
        match *self {
            Self::Date(d) => zone.to_utc(d.and_time(chrono::NaiveTime::MIN)),
            Self::Utc(t) => t,
            Self::Zoned(t, tz) => resolve(&tz, t),
            Self::Floating(t) => zone.to_utc(t),
        }
    }

    pub fn is_date(&self) -> bool {
        matches!(self, Self::Date(_))
    }

    /// The zone recurrences expand in, so "9:00 weekly" stays 9:00 across DST.
    fn expansion_tz(&self, zone: Zone) -> rrule::Tz {
        match self {
            Self::Utc(_) => rrule::Tz::UTC,
            Self::Zoned(_, tz) => (*tz).into(),
            Self::Date(_) | Self::Floating(_) => zone.rrule_tz(),
        }
    }
}

/// TZIDs are normally IANA names; some writers prefix them with a path such as
/// `/mozilla.org/20050126_1/America/New_York`.
fn parse_tzid(tzid: &str) -> Option<chrono_tz::Tz> {
    let parts: Vec<&str> = tzid
        .trim_matches('"')
        .split('/')
        .filter(|p| !p.is_empty())
        .collect();
    (1..=parts.len().min(3))
        .rev()
        .find_map(|n| parts[parts.len() - n..].join("/").parse().ok())
}

/// An attendee's reply (`PARTSTAT`, RFC 5545 section 3.2.12). A value
/// outside the standard ones, which only an invitation's writer could have
/// chosen, reads as `Other`, so none of its text passes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, strum::EnumString)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case", ascii_case_insensitive)]
pub enum Response {
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
    Delegated,
    #[strum(disabled)]
    Other,
}

/// An event's `STATUS` (RFC 5545 section 3.8.1.11), read the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, strum::EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum EventStatus {
    Tentative,
    Confirmed,
    Cancelled,
    #[strum(disabled)]
    Other,
}

/// One of a property's standard values, read after hidden characters are
/// removed (R6), or `other`.
fn choice<T: std::str::FromStr>(value: &str, other: T) -> T {
    clean(value.trim(), &mut 0).parse().unwrap_or(other)
}

#[derive(Clone, Debug, Default)]
pub struct Person {
    pub name: Option<String>,
    pub email: Option<String>,
    pub response: Option<Response>,
}

#[derive(Clone, Debug)]
pub struct VEvent {
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub start: When,
    pub end: Option<When>,
    pub duration: Option<Duration>,
    pub rrule: Option<String>,
    pub rdates: Vec<When>,
    pub exdates: Vec<When>,
    pub recurrence_id: Option<When>,
    pub status: Option<EventStatus>,
    pub transparent: bool,
    pub organizer: Option<Person>,
    pub attendees: Vec<Person>,
    /// Hidden characters removed from this event's text when it was read (RFC R6).
    pub hidden: usize,
}

impl VEvent {
    /// When an occurrence starting at `start` ends. An all-day occurrence ends
    /// at a local midnight, so a daylight-saving change cannot shorten its last day.
    pub fn end_at(&self, start: DateTime<Utc>, zone: Zone) -> DateTime<Utc> {
        let len = self.length(zone);
        if !self.start.is_date() {
            return start + len;
        }
        let days = (len + Duration::hours(12)).num_days().max(1);
        zone.to_utc((zone.wall(start).date() + Duration::days(days)).and_time(NaiveTime::MIN))
    }

    pub fn length(&self, zone: Zone) -> Duration {
        let start = self.start.utc(zone);
        if let Some(d) = self
            .end
            .as_ref()
            .map(|e| e.utc(zone) - start)
            .filter(|d| *d > Duration::zero())
        {
            return d;
        }
        if let Some(d) = self.duration.filter(|d| *d > Duration::zero()) {
            return d;
        }
        if self.start.is_date() {
            Duration::days(1)
        } else {
            Duration::zero()
        }
    }

    /// Whether the event repeats, by RRULE or RDATE.
    pub fn is_recurring(&self) -> bool {
        self.rrule.is_some() || !self.rdates.is_empty()
    }

    fn cancelled(&self) -> bool {
        self.status == Some(EventStatus::Cancelled)
    }
}

#[derive(Debug, Default)]
pub struct Feed {
    pub name: Option<String>,
    pub events: Vec<VEvent>,
    /// VEVENTs that could not be read (no UID, unparseable DTSTART, ...).
    pub skipped: usize,
}

struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

fn param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// RFC 5545 §3.1: a line starting with a space or tab continues the previous one.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        match (raw.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ if !raw.is_empty() => lines.push(raw.to_string()),
            _ => {}
        }
    }
    lines
}

/// Split on `sep` outside double quotes.
fn split_unquoted(s: &str, sep: char) -> Vec<&str> {
    let (mut out, mut start, mut quoted) = (Vec::new(), 0, false);
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == sep && !quoted {
            out.push(&s[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&s[start..]);
    out
}

fn parse_line(line: &str) -> Option<Prop> {
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(i, c)| {
        if c == '"' {
            quoted = !quoted;
        }
        (c == ':' && !quoted).then_some(i)
    })?;
    let mut head = split_unquoted(&line[..colon], ';').into_iter();
    let name = head.next()?.trim().to_ascii_uppercase();
    let params = head
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| {
            (
                k.trim().to_ascii_uppercase(),
                v.trim_matches('"').to_string(),
            )
        })
        .collect();
    Some(Prop {
        name,
        params,
        value: line[colon + 1..].to_string(),
    })
}

/// TEXT values escape `\n`, `\,`, `\;` and `\\`.
fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// ISO 8601 durations as RFC 5545 uses them: P1W, P1D, PT1H30M, -PT15M.
fn parse_duration(v: &str) -> Option<Duration> {
    let (negative, v) = match v.trim().strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, v.trim().trim_start_matches('+')),
    };
    let (mut total, mut digits, mut time) = (Duration::zero(), String::new(), false);
    for c in v.strip_prefix('P')?.chars() {
        let n = || digits.parse::<i64>().ok();
        let part = match (c, time) {
            ('T', _) => {
                time = true;
                continue;
            }
            ('0'..='9', _) => {
                digits.push(c);
                continue;
            }
            ('W', _) => Duration::try_weeks(n()?)?,
            ('D', _) => Duration::try_days(n()?)?,
            ('H', true) => Duration::try_hours(n()?)?,
            ('M', true) => Duration::try_minutes(n()?)?,
            ('S', true) => Duration::try_seconds(n()?)?,
            _ => return None,
        };
        total = total.checked_add(&part)?;
        digits.clear();
    }
    (total.num_days() <= MAX_DURATION_DAYS).then_some(if negative { -total } else { total })
}

/// A DESCRIPTION as text. Invitations from some calendars carry HTML, whose
/// tag names and attributes would otherwise be returned and matched by
/// `search_events`. Only text with a tag such as `<br` or `</` counts as
/// HTML, so plain text with a lone `<` passes through unchanged.
fn description_text(v: String) -> String {
    let lower = v.to_ascii_lowercase();
    let html = ["<br", "<p>", "<p ", "<div", "<a ", "</"]
        .iter()
        .any(|tag| lower.contains(tag));
    if html { html_to_text(&v) } else { v }
}

fn person(p: &Prop) -> Person {
    let v = p.value.trim();
    let email = v
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("mailto:"))
        .map(|_| v[7..].to_string());
    Person {
        name: param(&p.params, "CN").map(str::to_string),
        email,
        response: param(&p.params, "PARTSTAT").map(|v| choice(v, Response::Other)),
    }
}

fn build_event(props: &[Prop]) -> Result<VEvent> {
    let get = |name: &str| props.iter().find(|p| p.name == name);
    let text = |name: &str| get(name).map(|p| unescape(&p.value)).unwrap_or_default();
    let when = |name: &str| {
        get(name)
            .map(|p| When::parse(&p.value, &p.params))
            .transpose()
    };
    let mut rdates = Vec::new();
    let mut exdates = Vec::new();
    for p in props {
        let list = match p.name.as_str() {
            "RDATE" => &mut rdates,
            "EXDATE" => &mut exdates,
            _ => continue,
        };
        // A value that is not a date or date-time (RDATE;VALUE=PERIOD) is skipped.
        list.extend(
            p.value
                .split(',')
                .filter_map(|v| When::parse(v, &p.params).ok()),
        );
    }
    let mut e = VEvent {
        uid: get("UID")
            .map(|p| p.value.trim().to_string())
            .context("VEVENT without UID")?,
        summary: text("SUMMARY"),
        description: description_text(text("DESCRIPTION")),
        location: text("LOCATION"),
        start: when("DTSTART")?.context("VEVENT without DTSTART")?,
        end: when("DTEND")?,
        duration: get("DURATION").and_then(|p| parse_duration(&p.value)),
        rrule: get("RRULE").map(|p| p.value.trim().to_string()),
        rdates,
        exdates,
        recurrence_id: when("RECURRENCE-ID")?,
        status: get("STATUS").map(|p| choice(&p.value, EventStatus::Other)),
        transparent: get("TRANSP")
            .is_some_and(|p| p.value.trim().eq_ignore_ascii_case("TRANSPARENT")),
        organizer: get("ORGANIZER").map(person),
        attendees: props
            .iter()
            .filter(|p| p.name == "ATTENDEE")
            .map(person)
            .collect(),
        hidden: 0,
    };
    // RFC R6, once for every field an invitation's sender can write, so no
    // output site can forget one. The UID is cleaned too: it becomes the eventId.
    let mut hidden = 0;
    let people = e.organizer.iter_mut().chain(e.attendees.iter_mut());
    let fields = [
        &mut e.uid,
        &mut e.summary,
        &mut e.description,
        &mut e.location,
    ]
    .into_iter()
    .chain(people.flat_map(|p| [&mut p.name, &mut p.email].into_iter().flatten()));
    for s in fields {
        *s = clean(s, &mut hidden);
    }
    e.hidden = hidden;
    Ok(e)
}

pub fn parse(text: &str) -> Result<Feed> {
    let lines = unfold(text);
    if !lines
        .first()
        .is_some_and(|l| l.trim().eq_ignore_ascii_case("BEGIN:VCALENDAR"))
    {
        bail!("not an iCalendar feed (it does not start with BEGIN:VCALENDAR)");
    }
    let mut feed = Feed::default();
    let mut stack: Vec<String> = Vec::new();
    let mut props: Vec<Prop> = Vec::new();
    for prop in lines.iter().filter_map(|l| parse_line(l)) {
        match prop.name.as_str() {
            "BEGIN" => {
                stack.push(prop.value.trim().to_ascii_uppercase());
                if stack == ["VCALENDAR", "VEVENT"] {
                    props.clear();
                }
            }
            "END" => {
                if stack == ["VCALENDAR", "VEVENT"]
                    && prop.value.trim().eq_ignore_ascii_case("VEVENT")
                {
                    match build_event(&props) {
                        Ok(e) => feed.events.push(e),
                        Err(_) => feed.skipped += 1,
                    }
                }
                stack.pop();
            }
            "X-WR-CALNAME" if stack == ["VCALENDAR"] => feed.name = Some(unescape(&prop.value)),
            _ if stack == ["VCALENDAR", "VEVENT"] => props.push(prop),
            _ => {}
        }
    }
    Ok(feed)
}

/// One occurrence of an event inside a query window.
pub struct Occurrence<'a> {
    pub event: &'a VEvent,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// The event UID, or `UID|<original start, UTC>` for one occurrence of a series.
    pub id: String,
    pub recurring: bool,
}

/// An occurrence's eventId: the UID, then '|' and the start that names it. A
/// start fixed to an instant (UTC or a named zone) is written in UTC with
/// 'Z'. An all-day or floating start is written as wall-clock time without
/// 'Z', since its instant depends on the zone a caller asks in and its ID
/// must not.
pub fn occurrence_id(uid: &str, instance: Option<(&When, DateTime<Utc>)>, zone: Zone) -> String {
    match instance {
        Some((When::Date(_) | When::Floating(_), t)) => {
            format!("{uid}|{}", zone.wall(t).format("%Y%m%dT%H%M%S"))
        }
        Some((_, t)) => format!("{uid}|{}", t.format("%Y%m%dT%H%M%SZ")),
        None => uid.to_string(),
    }
}

/// The crate reads an UNTIL without `Z` (a DATE, or a floating time) in the
/// Mac's zone, then refuses it next to a DTSTART in any named zone. RFC 5545
/// means the series' own zone, so restate it in UTC.
fn until_in_utc(
    r: rrule::RRule<rrule::Unvalidated>,
    tz: rrule::Tz,
) -> rrule::RRule<rrule::Unvalidated> {
    match r.get_until().filter(|u| u.timezone().is_local()) {
        Some(u) => {
            let utc = resolve(&tz, u.naive_local()).with_timezone(&rrule::Tz::UTC);
            r.until(utc)
        }
        None => r,
    }
}

/// Start times of a series that fall in [from, to), in UTC. An RRULE the
/// `rrule` crate rejects degrades to the first occurrence alone.
fn series_starts(
    e: &VEvent,
    zone: Zone,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<DateTime<Utc>> {
    let first = e.start.utc(zone);
    let exdates: HashSet<DateTime<Utc>> = e.exdates.iter().map(|x| x.utc(zone)).collect();
    let mut starts: Vec<DateTime<Utc>> = match &e.rrule {
        None => vec![first],
        Some(rule) => {
            let tz = e.start.expansion_tz(zone);
            let dtstart = first.with_timezone(&tz);
            match rule
                .parse::<rrule::RRule<rrule::Unvalidated>>()
                .and_then(|r| until_in_utc(r, tz).validate(dtstart))
            {
                Ok(r) => rrule::RRuleSet::new(dtstart)
                    .rrule(r)
                    .after(from.with_timezone(&tz))
                    .before(to.with_timezone(&tz))
                    .all(MAX_OCCURRENCES)
                    .dates
                    .into_iter()
                    .map(|d| d.with_timezone(&Utc))
                    .collect(),
                Err(_) => vec![first],
            }
        }
    };
    starts.extend(e.rdates.iter().map(|r| r.utc(zone)));
    starts.retain(|s| !exdates.contains(s) && *s < to);
    starts.sort();
    starts.dedup();
    starts
}

/// Every occurrence overlapping [from, to), sorted by start. Cancelled events
/// and cancelled single occurrences are left out.
pub fn expand(
    feed: &Feed,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    zone: Zone,
) -> Vec<Occurrence<'_>> {
    let mut overridden: HashMap<&str, HashSet<DateTime<Utc>>> = HashMap::new();
    for e in &feed.events {
        if let Some(rid) = &e.recurrence_id {
            overridden
                .entry(e.uid.as_str())
                .or_default()
                .insert(rid.utc(zone));
        }
    }
    let overlaps =
        |s: DateTime<Utc>, end: DateTime<Utc>| s < to && (end > from || (end == s && s >= from));
    let mut out = Vec::new();
    for e in feed.events.iter().filter(|e| !e.cancelled()) {
        if let Some(rid) = &e.recurrence_id {
            let s = e.start.utc(zone);
            let end = e.end_at(s, zone);
            if overlaps(s, end) {
                let id = occurrence_id(&e.uid, Some((rid, rid.utc(zone))), zone);
                out.push(Occurrence {
                    event: e,
                    start: s,
                    end,
                    id,
                    recurring: true,
                });
            }
            continue;
        }
        let recurring = e.is_recurring();
        let skip = overridden.get(e.uid.as_str());
        let lookback = from - e.length(zone) - DST_SLACK;
        for s in series_starts(e, zone, lookback, to) {
            let end = e.end_at(s, zone);
            if skip.is_some_and(|set| set.contains(&s)) || !overlaps(s, end) {
                continue;
            }
            let id = occurrence_id(&e.uid, recurring.then_some((&e.start, s)), zone);
            out.push(Occurrence {
                event: e,
                start: s,
                end,
                id,
                recurring,
            });
        }
    }
    out.sort_by_key(|o| o.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Feed-like text: real property lines with odd values, folded lines and noise.
    fn feed_text() -> impl Strategy<Value = String> {
        let line = prop_oneof![
            Just("BEGIN:VEVENT".to_string()),
            Just("END:VEVENT".to_string()),
            "(DTSTART|DTEND|RECURRENCE-ID|EXDATE|RDATE)(;TZID=[A-Za-z/_+-]{0,16}|;VALUE=DATE)?:[0-9TZ+:,-]{0,18}",
            "RRULE:(FREQ=(SECONDLY|MINUTELY|HOURLY|DAILY|WEEKLY|MONTHLY|YEARLY);?)?(INTERVAL=[0-9]{1,6};?)?(COUNT=[0-9]{1,7};?)?(UNTIL=[0-9TZ]{0,16};?)?(BYDAY=[A-Z0-9,+-]{0,12};?)?(BYMONTHDAY=[0-9,-]{0,8})?",
            "DURATION:[-+]?P([0-9]{0,14}W)?([0-9]{0,14}D)?(T([0-9]{0,14}H)?([0-9]{0,14}M)?([0-9]{0,14}S)?)?",
            "(UID|SUMMARY|DESCRIPTION|LOCATION|STATUS|X-WR-CALNAME):\\PC{0,24}",
            " \\PC{0,10}",
            any::<String>(),
        ];
        prop::collection::vec(line, 0..20).prop_map(|lines| {
            format!(
                "BEGIN:VCALENDAR\r\n{}\r\nEND:VCALENDAR\r\n",
                lines.join("\r\n")
            )
        })
    }

    proptest! {
        #[test]
        fn parsing_and_expanding_any_feed_never_panics(text in feed_text()) {
            if let Ok(feed) = parse(&text) {
                let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
                let zone = Zone::parse(Some("America/Los_Angeles")).unwrap();
                let _occurrences = expand(&feed, from, from + Duration::days(400), zone);
            }
        }
    }

    const FEED: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
X-WR-CALNAME:Personal\r\n\
BEGIN:VEVENT\r\n\
UID:weekly@example.test\r\n\
SUMMARY:Standup\\, team\r\n\
DTSTART;TZID=America/Los_Angeles:20261026T090000\r\n\
DTEND;TZID=America/Los_Angeles:20261026T093000\r\n\
RRULE:FREQ=WEEKLY;COUNT=4\r\n\
EXDATE;TZID=America/Los_Angeles:20261102T090000\r\n\
BEGIN:VALARM\r\n\
ACTION:DISPLAY\r\n\
DESCRIPTION:not part of the event\r\n\
END:VALARM\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
UID:weekly@example.test\r\n\
RECURRENCE-ID;TZID=America/Los_Angeles:20261109T090000\r\n\
SUMMARY:Standup (moved)\r\n\
DTSTART;TZID=America/Los_Angeles:20261109T110000\r\n\
DTEND;TZID=America/Los_Angeles:20261109T113000\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
UID:allday@example.test\r\n\
SUMMARY:Holiday\r\n\
DESCRIPTION:Line one\\nline two that is folded\r\n  onto a second line\r\n\
DTSTART;VALUE=DATE:20261111\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
UID:gone@example.test\r\n\
SUMMARY:Cancelled thing\r\n\
STATUS:CANCELLED\r\n\
DTSTART:20261028T170000Z\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
SUMMARY:No UID, skipped\r\n\
DTSTART:20261028T170000Z\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

    fn la() -> Zone {
        Zone::Iana(chrono_tz::America::Los_Angeles)
    }

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn reads_events_and_skips_bad_ones() {
        let feed = parse(FEED).unwrap();
        assert_eq!(feed.name.as_deref(), Some("Personal"));
        assert_eq!(feed.events.len(), 4);
        assert_eq!(feed.skipped, 1);
        assert_eq!(feed.events[0].summary, "Standup, team");
        assert_eq!(
            feed.events[2].description,
            "Line one\nline two that is folded onto a second line"
        );
    }

    /// A status or reply outside the standard values is `Other`, so no text
    /// an invitation's writer chose passes on; a hidden character inside a
    /// standard value does not hide it.
    #[test]
    fn statuses_and_replies_are_standard_values_or_other() {
        let feed = parse(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nDTSTART:20261005T170000Z\r\n\
             STATUS:Call Dana Ruiz\r\n\
             ATTENDEE;CN=Ann;PARTSTAT=ACCEPTED:mailto:ann@x.test\r\n\
             ATTENDEE;CN=Bo;PARTSTAT=X-DANA:mailto:bo@x.test\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:b\r\nDTSTART:20261005T170000Z\r\n\
             STATUS:CANCEL\u{200B}LED\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let (a, b) = (&feed.events[0], &feed.events[1]);
        assert_eq!(a.status, Some(EventStatus::Other));
        let replies: Vec<_> = a.attendees.iter().map(|p| p.response).collect();
        assert_eq!(replies, [Some(Response::Accepted), Some(Response::Other)]);
        assert!(b.cancelled());
        assert_eq!(
            serde_json::to_value(Response::NeedsAction).unwrap(),
            "needs-action"
        );
    }

    #[test]
    fn expands_series_across_dst_with_exdate_and_override() {
        let feed = parse(FEED).unwrap();
        let got = expand(
            &feed,
            utc("2026-10-25T00:00:00Z"),
            utc("2026-11-30T00:00:00Z"),
            la(),
        );
        let standups: Vec<String> = got
            .iter()
            .filter(|o| o.event.uid == "weekly@example.test")
            .map(|o| la().format(o.start))
            .collect();
        // Oct 26 (PDT), Nov 2 excluded, Nov 9 moved to 11:00, Nov 16 (PST).
        assert_eq!(
            standups,
            [
                "2026-10-26T09:00:00-07:00",
                "2026-11-09T11:00:00-08:00",
                "2026-11-16T09:00:00-08:00"
            ]
        );
        let moved = got
            .iter()
            .find(|o| o.event.summary == "Standup (moved)")
            .unwrap();
        assert_eq!(moved.id, "weekly@example.test|20261109T170000Z");
    }

    #[test]
    fn all_day_events_cover_the_day_and_cancelled_events_are_hidden() {
        let feed = parse(FEED).unwrap();
        let got = expand(
            &feed,
            utc("2026-11-11T20:00:00Z"),
            utc("2026-11-11T21:00:00Z"),
            la(),
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].event.summary, "Holiday");
        assert_eq!(got[0].end - got[0].start, Duration::days(1));
        let oct28 = expand(
            &feed,
            utc("2026-10-28T00:00:00Z"),
            utc("2026-10-29T00:00:00Z"),
            la(),
        );
        assert!(oct28.iter().all(|o| o.event.uid != "gone@example.test"));
    }

    #[test]
    fn durations_parse() {
        assert_eq!(parse_duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(parse_duration("P1W"), Some(Duration::weeks(1)));
        assert_eq!(parse_duration("-PT15M"), Some(Duration::minutes(-15)));
        assert_eq!(parse_duration("P1H"), None);
    }

    #[test]
    fn rejects_non_calendars() {
        assert!(parse("<html>").is_err());
    }

    fn one_event(lines: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n{lines}END:VEVENT\r\nEND:VCALENDAR\r\n")
    }

    #[test]
    fn huge_durations_cannot_panic() {
        for d in ["P999999999999D", "P100000000D", "P99999999999W"] {
            let feed = parse(&one_event(&format!(
                "UID:x\r\nDTSTART:20261028T170000Z\r\nDURATION:{d}\r\n"
            )))
            .unwrap();
            expand(
                &feed,
                utc("2026-10-01T00:00:00Z"),
                utc("2026-11-01T00:00:00Z"),
                la(),
            );
        }
    }

    #[test]
    fn all_day_and_floating_occurrences_keep_their_ids_in_any_zone() {
        let feed = parse(
            "BEGIN:VCALENDAR\r\n\
BEGIN:VEVENT\r\nUID:a\r\nDTSTART;VALUE=DATE:20261005\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:f\r\nDTSTART:20261005T090000\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n",
        )
        .unwrap();
        let (from, to) = (utc("2026-10-01T00:00:00Z"), utc("2026-10-31T00:00:00Z"));
        let ids = |zone: &str| {
            let mut ids: Vec<String> = expand(&feed, from, to, Zone::parse(Some(zone)).unwrap())
                .iter()
                .map(|o| o.id.clone())
                .collect();
            ids.sort();
            ids
        };
        let paris = ids("Europe/Paris");
        assert_eq!(paris.len(), 6);
        assert_eq!(paris, ids("America/Los_Angeles"));
        assert!(
            paris.contains(&"a|20261012T000000".to_string()),
            "{paris:?}"
        );
        assert!(
            paris.contains(&"f|20261012T090000".to_string()),
            "{paris:?}"
        );
    }

    #[test]
    fn all_day_series_with_a_date_until_expands_in_a_named_zone() {
        // RFC 5545 writes UNTIL as a DATE when DTSTART is one.
        let feed = parse(&one_event(
            "UID:w\r\nDTSTART;VALUE=DATE:20261025\r\nDTEND;VALUE=DATE:20261026\r\n\
             RRULE:FREQ=WEEKLY;UNTIL=20261115\r\n",
        ))
        .unwrap();
        let got = expand(
            &feed,
            utc("2026-10-20T00:00:00Z"),
            utc("2026-12-31T00:00:00Z"),
            la(),
        );
        let days: Vec<String> = got
            .iter()
            .map(|o| la().wall(o.start).date().to_string())
            .collect();
        assert_eq!(
            days,
            ["2026-10-25", "2026-11-01", "2026-11-08", "2026-11-15"]
        );
        // 1 November 2026 is the 25-hour day when Los Angeles leaves daylight
        // time; the 24-hour series must still end that occurrence at midnight.
        assert_eq!(la().wall(got[1].end).date().to_string(), "2026-11-02");
        assert_eq!(got[2].end - got[2].start, Duration::days(1));
    }

    #[test]
    fn html_descriptions_become_text_and_plain_text_is_kept() {
        let html = parse(&one_event(
            "UID:h\r\nDTSTART:20261028T170000Z\r\n\
             DESCRIPTION:<div>Agenda<br>Room <b>4</b></div><a href=\"https://x.test\">Notes</a>\r\n",
        ))
        .unwrap();
        let text = &html.events[0].description;
        assert!(
            text.contains("Agenda") && text.contains("Notes"),
            "{text:?}"
        );
        assert!(!text.contains('<') && !text.contains("href"), "{text:?}");
        // A lone '<' in plain text is not HTML.
        let plain = "Budget < 5k\\, and 3 > 2; bring a <pen>";
        let kept = parse(&one_event(&format!(
            "UID:p\r\nDTSTART:20261028T170000Z\r\nDESCRIPTION:{plain}\r\n"
        )))
        .unwrap();
        assert_eq!(
            kept.events[0].description,
            "Budget < 5k, and 3 > 2; bring a <pen>"
        );
    }

    #[test]
    fn hidden_characters_are_removed_from_every_field() {
        let feed = parse(&one_event(
            "UID:id\u{E0041}\r\nDTSTART:20261028T170000Z\r\nSUMMARY:a\u{202E}b\r\n\
             ATTENDEE;CN=n\u{2066};PARTSTAT=ACCEPTED:mailto:x\u{202E}@example.test\r\n",
        ))
        .unwrap();
        let e = &feed.events[0];
        assert_eq!(e.uid, "id");
        assert_eq!(e.summary, "ab");
        assert_eq!(e.attendees[0].name.as_deref(), Some("n"));
        assert_eq!(e.attendees[0].email.as_deref(), Some("x@example.test"));
        assert_eq!(e.hidden, 4);
    }
}
