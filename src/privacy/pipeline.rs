//! The privacy pipeline (RFC section 6; low-level design "The pipeline"):
//! every aliases-mode result passes through `run` before it leaves. It
//! collects the names the result's own headers hold, finds every mention,
//! gives each entity one alias, rewrites each field by its policy, and adds
//! `entities`, `detectors`, `queryEntities`, `guidance` and `dropped`.
//! Anything it cannot do fails the call (R13); nothing falls back.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};

use serde::Serialize;

use super::canon;
use super::detect::dict::{Dictionary, Names};
use super::detect::{self, Detector, Kind, Mention, pattern};
use super::fields::{Policy, policy};
use super::ident::{AliasClass, EntityType, ItemKind};
use super::key::Keys;
use crate::tool::Tool;

/// The largest result, in characters of JSON: under Claude Code's limit of
/// 25,000 tokens with room to spare (RFC section 6, Size).
pub const CAP: usize = 90_000;

/// What the pipeline reports to `detectors`.
pub const DETECTORS: [Detector; 2] = [Detector::Regex, Detector::Dictionary];

/// In tests only, text that makes the rewrite stage panic, for the
/// fail-closed test (RFC section 7; low-level design, "Testing hooks").
#[cfg(test)]
pub const PLANTED_PANIC: &str = "protonctl-planted-panic";

/// MIME types kept as they are; a sender can write any other, a name
/// included, into an attachment's type.
const KNOWN_MIME: &[&str] = &[
    "application/pdf",
    "application/json",
    "application/zip",
    "application/rtf",
    "application/msword",
    "application/vnd.ms-excel",
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/vnd.oasis.opendocument.text",
    "application/vnd.oasis.opendocument.spreadsheet",
    "application/vnd.oasis.opendocument.presentation",
    "application/ics",
    "application/pgp-keys",
    "application/pgp-signature",
    "application/pkcs7-signature",
    "application/octet-stream",
    "text/plain",
    "text/html",
    "text/markdown",
    "text/csv",
    "text/calendar",
    "text/rtf",
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/heic",
    "image/tiff",
    "image/svg+xml",
    "audio/mpeg",
    "video/mp4",
    "message/rfc822",
];

/// The IANA top-level types, kept when the subtype is not known.
const MIME_TOP: &[&str] = &[
    "application",
    "audio",
    "font",
    "image",
    "message",
    "model",
    "multipart",
    "text",
    "video",
];

/// `s` if it is a known MIME type; else its top-level type and "other".
fn known_mime(s: &str) -> String {
    let lower = s.trim().to_ascii_lowercase();
    if KNOWN_MIME.contains(&lower.as_str()) {
        return lower;
    }
    match lower.split_once('/') {
        Some((top, _)) if MIME_TOP.contains(&top) => format!("{top}/other"),
        _ => "other".into(),
    }
}

/// Replaces a path on this machine (R16, R17).
const LOCAL_PATH: &str = "<local path>";

/// Providers whose domain says nothing about an organization: a hint of
/// "your organization" there would mark every user of the provider.
const WEBMAIL: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "msn.com",
    "yahoo.com",
    "ymail.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "aol.com",
    "proton.me",
    "protonmail.com",
    "protonmail.ch",
    "pm.me",
    "gmx.com",
    "gmx.de",
    "gmx.net",
    "web.de",
    "mail.com",
    "yandex.com",
    "yandex.ru",
    "zoho.com",
    "fastmail.com",
    "tutanota.com",
    "tuta.io",
    "hey.com",
];

/// What the call was, for hints and `queryEntities`.
pub struct Context<'a> {
    pub tool: Tool,
    /// The caller's query, when the tool takes one.
    pub query: Option<&'a str>,
    /// The user's own address, for the `you` and `your-organization` hints.
    pub you: Option<&'a str>,
    /// Names known to the whole process (RFC Q22).
    pub names: &'a Names,
    /// The sources of `names` that could not be read whole.
    pub incomplete: &'a [NameSource],
}

/// A source of the process dictionary (RFC Q22).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NameSource {
    Mail,
    Calendar,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PipelineError {
    TooLarge,
    Failed,
}

/// An entity's role in this result; the first is the one shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    Sender,
    Recipient,
    Cc,
    Organizer,
    Attendee,
    Mentioned,
}

impl Role {
    /// The role a result member's name gives the people it holds.
    fn of(key: &str) -> Self {
        match key {
            "from" => Self::Sender,
            "to" => Self::Recipient,
            "cc" => Self::Cc,
            "organizer" => Self::Organizer,
            "attendees" => Self::Attendee,
            _ => Self::Mentioned,
        }
    }
}

/// How the user relates to an address (RFC section 6, Hints).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Relation {
    You,
    YourOrganization,
    External,
}

/// What a domain says about who runs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum DomainType {
    Webmail,
    Government,
    Education,
    Organization,
}

/// One hint in an `entities` row: a role, then a relation, then a domain
/// type, each shown as its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum Hint {
    Role(Role),
    Relation(Relation),
    Domain(DomainType),
}

/// An entity's identity: its alias class and canonical value.
type EntityKey = (AliasClass, String);

struct Entity {
    t: EntityType,
    roles: BTreeSet<Role>,
    /// For a person, the canonical addresses seen with them.
    emails: BTreeSet<String>,
}

#[derive(Default)]
struct Registry {
    entities: BTreeMap<EntityKey, Entity>,
    /// Links by canonical URL, in order of first appearance.
    links: Vec<String>,
    aliases: HashMap<EntityKey, String>,
}

impl Registry {
    fn key(t: EntityType, raw: &str) -> Option<EntityKey> {
        let canonical = match t {
            EntityType::Person | EntityType::Organization | EntityType::Location => {
                canon::name(raw)
            }
            EntityType::Email => canon::email(raw),
            EntityType::Phone => raw.to_string(),
            EntityType::Card | EntityType::Iban => canon::compact(raw),
            EntityType::Domain => canon::domain(raw),
            EntityType::Ip => canon::ip(raw),
            EntityType::Address
            | EntityType::Secret
            | EntityType::NationalId
            | EntityType::Account => canon::plain(raw),
        };
        (!canonical.is_empty()).then(|| (t.class(), canonical))
    }

    fn add(&mut self, t: EntityType, raw: &str, role: Role) -> Option<EntityKey> {
        let key = Self::key(t, raw)?;
        self.entities
            .entry(key.clone())
            .or_insert_with(|| Entity {
                t,
                roles: BTreeSet::new(),
                emails: BTreeSet::new(),
            })
            .roles
            .insert(role);
        Some(key)
    }

    fn link(&mut self, url: &str) {
        let canonical = canon::url(url);
        if !self.links.contains(&canonical) {
            self.links.push(canonical);
        }
        if let Some(host) = host(url) {
            let t = if host.parse::<std::net::IpAddr>().is_ok() {
                EntityType::Ip
            } else {
                EntityType::Domain
            };
            self.add(t, &host, Role::Mentioned);
        }
    }

    /// Three words each; within one result, entities that share three words
    /// are ordered by their refs, and each later one takes one more word.
    fn settle_aliases(&mut self, keys: &Keys) {
        let mut by_alias: BTreeMap<String, Vec<(String, EntityKey)>> = BTreeMap::new();
        for (key, e) in &self.entities {
            let alias = keys.alias(e.t, &key.1, 0);
            by_alias
                .entry(alias)
                .or_default()
                .push((keys.reference(e.t, &key.1), key.clone()));
        }
        for group in by_alias.into_values() {
            let mut group = group;
            group.sort();
            for (extra, (_, key)) in group.into_iter().enumerate() {
                let t = self.entities[&key].t;
                self.aliases
                    .insert(key.clone(), keys.alias(t, &key.1, extra));
            }
        }
    }

    fn alias(&self, t: EntityType, raw: &str) -> Option<&str> {
        self.aliases.get(&Self::key(t, raw)?).map(String::as_str)
    }

    fn link_text(&self, url: &str) -> String {
        let canonical = canon::url(url);
        let n = self
            .links
            .iter()
            .position(|l| *l == canonical)
            .map_or(0, |i| i + 1);
        let host = host(url).and_then(|h| {
            let t = if h.parse::<std::net::IpAddr>().is_ok() {
                EntityType::Ip
            } else {
                EntityType::Domain
            };
            self.alias(t, &h).map(str::to_string)
        });
        match host {
            Some(h) => format!("link {n} ({h})"),
            None => format!("link {n}"),
        }
    }
}

fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..end].rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_lowercase())
}

fn domain_of(email: &str) -> Option<&str> {
    email.rsplit_once('@').map(|(_, d)| d)
}

fn domain_type(d: &str) -> DomainType {
    let ends = |s: &str| d == s || d.ends_with(&format!(".{s}"));
    if WEBMAIL.contains(&d) {
        DomainType::Webmail
    } else if ends("gov")
        || ends("mil")
        || d.contains(".gov.")
        || ends("gouv.fr")
        || ends("gc.ca")
        || ends("europa.eu")
    {
        DomainType::Government
    } else if ends("edu") || d.contains(".edu.") || d.contains(".ac.") {
        DomainType::Education
    } else {
        DomainType::Organization
    }
}

/// Which stage a walk is in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Names,
    Register,
    Rewrite,
}

struct State<'a> {
    stage: Stage,
    tool: Tool,
    keys: &'a Keys,
    names: Names,
    dict: Option<Dictionary>,
    reg: Registry,
    dropped: Vec<String>,
}

/// What to do with a value after a walk.
enum After {
    Keep,
    Remove,
}

/// Hidden characters and their `\u{...}` escapes (R6) removed, with a map
/// from each byte of the copy back to the original, so a zero-width
/// character cannot split a name the detectors would otherwise find.
fn without_hidden(text: &str) -> (String, Vec<usize>) {
    static ESCAPE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\\u\{[0-9A-Fa-f]{1,6}\}").expect("a fixed pattern"));
    let mut copy = String::with_capacity(text.len());
    let mut map = Vec::with_capacity(text.len() + 1);
    let mut skip_until = 0;
    for (i, c) in text.char_indices() {
        if i < skip_until {
            continue;
        }
        if c == '\\'
            && let Some(m) = ESCAPE.find_at(text, i).filter(|m| m.start() == i)
            && m.as_str() != "\\u{5C}"
        {
            skip_until = m.end();
            continue;
        }
        if crate::content::hidden(c) {
            continue;
        }
        // Every byte of the copy's character maps to where the character
        // starts in `text`, so a span's end can be found from its last byte.
        copy.push(c);
        map.extend(std::iter::repeat_n(i, c.len_utf8()));
    }
    map.push(text.len());
    (copy, map)
}

impl State<'_> {
    fn mentions(&self, text: &str) -> Vec<Mention> {
        let (copy, map) = without_hidden(text);
        let mut found = Vec::new();
        pattern::find(&copy, &mut found);
        if let Some(d) = &self.dict {
            d.find(&copy, &mut found);
        }
        detect::settle(found)
            .into_iter()
            .map(|mut m| {
                m.start = map[m.start];
                // The end maps through the last byte of the span.
                m.end = map[m.end - 1]
                    + text[map[m.end - 1]..]
                        .chars()
                        .next()
                        .map_or(1, char::len_utf8);
                m
            })
            .collect()
    }

    /// Free text: register in the Register stage; rewritten in Rewrite.
    fn text(&mut self, text: &str, role: Role) -> Option<String> {
        let mentions = self.mentions(text);
        match self.stage {
            Stage::Names => None,
            Stage::Register => {
                for m in &mentions {
                    match &m.kind {
                        Kind::Entity(t) => {
                            self.reg.add(*t, &m.value, role);
                        }
                        Kind::Link => self.reg.link(&m.value),
                    }
                }
                None
            }
            Stage::Rewrite => {
                #[cfg(test)]
                assert!(!text.contains(PLANTED_PANIC), "a planted panic");
                let mut out = String::with_capacity(text.len());
                let mut at = 0;
                for m in mentions {
                    out.push_str(&text[at..m.start]);
                    let replaced = match &m.kind {
                        Kind::Entity(t) => self.reg.alias(*t, &m.value).map(str::to_string),
                        Kind::Link => Some(self.reg.link_text(&m.value)),
                    };
                    // A mention with no alias would leak; fail the call (R13).
                    out.push_str(&replaced?);
                    at = m.end;
                }
                out.push_str(&text[at..]);
                Some(out)
            }
        }
    }

    /// "Name <email>" or "email".
    /// A domain, the whole value.
    fn domain(&mut self, s: &str) -> Option<String> {
        match self.stage {
            Stage::Names => {
                self.names.add(s, EntityType::Domain);
                None
            }
            Stage::Register => {
                self.reg.add(EntityType::Domain, s, Role::Mentioned);
                None
            }
            Stage::Rewrite => self.reg.alias(EntityType::Domain, s).map(str::to_string),
        }
    }

    /// "Name <email>", "email", or a name alone: mail shows a From with no
    /// address as "Name <>".
    fn address(&mut self, s: &str, role: Role) -> Option<String> {
        let (name, email) = match s.rsplit_once('<') {
            Some((n, rest)) if rest.ends_with('>') => (
                n.trim().trim_matches('"').trim(),
                rest[..rest.len() - 1].trim(),
            ),
            _ if s.contains('@') => ("", s.trim()),
            _ => (s.trim(), ""),
        };
        let has_email = !email.is_empty();
        let has_name = !name.is_empty() && canon::email(name) != canon::email(email);
        match self.stage {
            Stage::Names => {
                if has_name {
                    self.names.add(name, EntityType::Person);
                }
                if has_email {
                    self.names.add_address(email);
                }
                None
            }
            Stage::Register => {
                let e = has_email
                    .then(|| self.reg.add(EntityType::Email, email, role))
                    .flatten();
                if has_name
                    && let Some(p) = self.reg.add(EntityType::Person, name, role)
                    && let Some(e) = e
                {
                    self.reg
                        .entities
                        .get_mut(&p)
                        .expect("just added")
                        .emails
                        .insert(e.1);
                }
                None
            }
            Stage::Rewrite => {
                let alias = |t, v| self.reg.alias(t, v).map(str::to_string);
                let e = if has_email {
                    Some(alias(EntityType::Email, email)?)
                } else {
                    None
                };
                let p = if has_name {
                    Some(alias(EntityType::Person, name)?)
                } else {
                    None
                };
                Some(match (p, e) {
                    (Some(p), Some(e)) => format!("{p} <{e}>"),
                    (Some(one), None) | (None, Some(one)) => one,
                    (None, None) => String::new(),
                })
            }
        }
    }

    fn drive_path(&mut self, path: &str) -> Option<String> {
        let mut parts = Vec::new();
        for part in path.split('/') {
            let rewritten = self.text(part, Role::Mentioned);
            parts.push(rewritten.unwrap_or_default());
        }
        (self.stage == Stage::Rewrite).then(|| parts.join("/"))
    }

    /// A string's new value in the Rewrite stage; `None` keeps it. Text that
    /// cannot be rewritten fails the call rather than pass (R13).
    fn string(&mut self, s: &str, p: Policy, key: &str) -> Result<Option<Value>, PipelineError> {
        let rewrite = self.stage == Stage::Rewrite;
        let keys = self.keys;
        let rewritten = match p {
            Policy::Text => self.text(s, Role::Mentioned),
            Policy::Address => self.address(s, Role::of(key)),
            Policy::AddressOrDomain if s.contains('@') => self.address(s, Role::Mentioned),
            Policy::AddressOrDomain => self.domain(s),
            Policy::DrivePath => self.drive_path(s),
            _ if !rewrite => return Ok(None),
            Policy::LocalPath => return Ok(Some(json!(LOCAL_PATH))),
            Policy::MimeType => return Ok(Some(json!(known_mime(s)))),
            Policy::Id(kind) => return Ok(Some(json!(keys.handle(kind, s)))),
            Policy::Digest(alg) => {
                let raw = hex::decode(s).unwrap_or_else(|_| s.as_bytes().to_vec());
                return Ok(Some(json!(keys.keyed_digest(alg, &raw))));
            }
            Policy::Token(kind) => return Ok(Some(json!(keys.seal_token(kind, s)))),
            Policy::Error => {
                let kept = if s == crate::DRIVE_NOT_SET_UP {
                    s
                } else {
                    "unavailable"
                };
                return Ok(Some(json!(kept)));
            }
            _ => return Ok(None),
        };
        match (rewrite, rewritten) {
            (true, Some(text)) => Ok(Some(Value::String(text))),
            (true, None) => Err(PipelineError::Failed),
            (false, _) => Ok(None),
        }
    }
}

/// Walk `v`, under the member `key` with policy `p`.
fn walk(
    v: &mut Value,
    p: Policy,
    key: &str,
    path: &str,
    st: &mut State,
) -> Result<After, PipelineError> {
    if p == Policy::Drop {
        return Ok(After::Remove);
    }
    match v {
        Value::String(s) => {
            if p == Policy::Unlisted || p == Policy::Image || p == Policy::Person {
                // Removed, and named in `dropped`, in the last stage only.
                if st.stage != Stage::Rewrite {
                    return Ok(After::Keep);
                }
                st.dropped.push(path.to_string());
                return Ok(After::Remove);
            }
            if let Some(new) = st.string(s, p, key)? {
                *v = new;
            }
            Ok(After::Keep)
        }
        Value::Array(items) => {
            let mut keep = Vec::with_capacity(items.len());
            for (i, mut item) in std::mem::take(items).into_iter().enumerate() {
                if let After::Keep = walk(&mut item, p, key, &format!("{path}/{i}"), st)? {
                    keep.push(item);
                }
            }
            *items = keep;
            Ok(After::Keep)
        }
        Value::Object(map) => {
            if p == Policy::Image {
                if st.stage == Stage::Rewrite {
                    let mime = map.get("mimeType").and_then(Value::as_str).map(known_mime);
                    *map = json!({ "mimeType": mime, "text": null,
                        "reason": "aliases mode returns no images until Phase 4 reads their text" })
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                }
                return Ok(After::Keep);
            }
            let person = p == Policy::Person;
            if person {
                person_object(map, key, st)?;
            }
            let names: Vec<String> = map.keys().cloned().collect();
            let mut sibling = Vec::new();
            for k in names {
                if person && matches!(k.as_str(), "name" | "email") {
                    continue;
                }
                let child = if person && k == "response" {
                    Policy::Plain
                } else {
                    policy(st.tool, &k)
                };
                let at = format!("{path}/{k}");
                let before = map.get(&k).and_then(Value::as_str).map(str::to_string);
                let entry = map.get_mut(&k).expect("a listed key");
                if let After::Remove = walk(entry, child, &k, &at, st)? {
                    map.remove(&k);
                } else if child == Policy::DrivePath
                    && st.stage == Stage::Rewrite
                    && let Some(original) = before
                {
                    sibling.push((k, original));
                }
            }
            // A listing's own `path`, at the top, is the folder it lists.
            let listing =
                path.is_empty() && matches!(st.tool, Tool::ListFolder | Tool::ListDriveTree);
            for (k, original) in sibling {
                let id = match (k.as_str(), map.get("kind").and_then(Value::as_str)) {
                    ("parent", _) => "parentId",
                    (_, Some("folder")) => "folderId",
                    _ if listing => "folderId",
                    _ => "fileId",
                };
                map.insert(
                    id.into(),
                    json!(st.keys.handle(ItemKind::DrivePath, &original)),
                );
            }
            Ok(After::Keep)
        }
        _ => Ok(After::Keep),
    }
}

fn person_object(
    map: &mut Map<String, Value>,
    key: &str,
    st: &mut State,
) -> Result<(), PipelineError> {
    let role = Role::of(key);
    let name = map.get("name").and_then(Value::as_str).map(str::to_string);
    let email = map.get("email").and_then(Value::as_str).map(str::to_string);
    match st.stage {
        Stage::Names => {
            if let Some(n) = &name {
                st.names.add(n, EntityType::Person);
            }
            if let Some(e) = &email {
                st.names.add_address(e);
            }
        }
        Stage::Register => {
            let e = email
                .as_deref()
                .and_then(|e| st.reg.add(EntityType::Email, e, role));
            if let Some(p) = name
                .as_deref()
                .and_then(|n| st.reg.add(EntityType::Person, n, role))
                && let Some(e) = e
            {
                st.reg
                    .entities
                    .get_mut(&p)
                    .expect("just added")
                    .emails
                    .insert(e.1);
            }
        }
        Stage::Rewrite => {
            for (field, t) in [("name", EntityType::Person), ("email", EntityType::Email)] {
                if let Some(s) = map.get(field).and_then(Value::as_str).map(str::to_string) {
                    let alias = st
                        .reg
                        .alias(t, &s)
                        .ok_or(PipelineError::Failed)?
                        .to_string();
                    map.insert(field.into(), json!(alias));
                }
            }
        }
    }
    Ok(())
}

/// What a name typed in a query is: an address when it has an `@`.
fn typed_type(name: &str) -> EntityType {
    if name.contains('@') {
        EntityType::Email
    } else {
        EntityType::Person
    }
}

/// Names the query holds in `from:`, `to:`, `cc:` and `bcc:`, as typed.
fn query_names(query: &str) -> Vec<String> {
    static OPERATOR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)\b(?:from|to|cc|bcc):(?:"([^"]+)"|(\S+))"#).expect("a fixed pattern")
    });
    OPERATOR
        .captures_iter(query)
        .filter_map(|c| {
            c.get(1)
                .or_else(|| c.get(2))
                .map(|m| m.as_str().to_string())
        })
        .filter(|v| !v.starts_with("ref:"))
        .collect()
}

fn hints(e: &Entity, key: &EntityKey, you: Option<&str>) -> Vec<Hint> {
    let mut h: Vec<Hint> = e
        .roles
        .iter()
        .next()
        .map(|r| Hint::Role(*r))
        .into_iter()
        .collect();
    let email = match e.t {
        EntityType::Email => Some(key.1.clone()),
        EntityType::Person => e.emails.iter().next().cloned(),
        _ => None,
    };
    let domain = match e.t {
        EntityType::Domain => Some(key.1.clone()),
        _ => email.as_deref().and_then(domain_of).map(str::to_string),
    };
    if let Some(email) = &email {
        let you = you.map(canon::email);
        let mine = you
            .as_deref()
            .and_then(domain_of)
            .filter(|d| !WEBMAIL.contains(d));
        h.push(Hint::Relation(
            if you.as_deref() == Some(email.as_str())
                || e.emails.iter().any(|m| Some(m.as_str()) == you.as_deref())
            {
                Relation::You
            } else if mine.is_some() && mine == domain_of(email) {
                Relation::YourOrganization
            } else {
                Relation::External
            },
        ));
    }
    if let Some(d) = &domain {
        h.push(Hint::Domain(domain_type(d)));
    }
    h.truncate(3);
    h
}

/// Rewrite one result. `keys` are the call's (R20).
pub fn run(mut value: Value, keys: &Keys, ctx: &Context) -> Result<Value, PipelineError> {
    let mut st = State {
        stage: Stage::Names,
        tool: ctx.tool,
        keys,
        names: ctx.names.clone(),
        dict: None,
        reg: Registry::default(),
        dropped: Vec::new(),
    };
    let typed = ctx.query.map(query_names).unwrap_or_default();
    for name in &typed {
        st.names.add(name, typed_type(name));
    }
    walk(&mut value, Policy::Within, "", "", &mut st)?;
    #[expect(
        clippy::map_err_ignore,
        reason = "R13: the error can quote a name; the call fails with a fixed fault"
    )]
    let dict = Dictionary::new(&st.names).map_err(|_| PipelineError::Failed)?;
    st.dict = Some(dict);
    st.stage = Stage::Register;
    walk(&mut value, Policy::Within, "", "", &mut st)?;
    st.reg.settle_aliases(keys);
    st.stage = Stage::Rewrite;
    walk(&mut value, Policy::Within, "", "", &mut st)?;

    let mut entities = Map::new();
    for (key, e) in &st.reg.entities {
        let alias = st.reg.aliases[key].clone();
        let mut row = json!({
            "type": e.t,
            "hints": hints(e, key, ctx.you),
            "ref": keys.reference(e.t, &key.1),
        });
        if !e.emails.is_empty() {
            let addresses: Vec<&str> = e
                .emails
                .iter()
                .filter_map(|m| {
                    st.reg
                        .aliases
                        .get(&(EntityType::Email.class(), m.clone()))
                        .map(String::as_str)
                })
                .collect();
            row["addresses"] = json!(addresses);
        }
        entities.insert(alias, row);
    }
    let Value::Object(out) = &mut value else {
        return Err(PipelineError::Failed);
    };
    if !entities.is_empty() {
        out.insert("entities".into(), Value::Object(entities));
    }
    out.insert("detectors".into(), json!(DETECTORS));
    // R24: the dictionary ran, but names from these sources can be missed.
    if !ctx.incomplete.is_empty() {
        out.insert("dictionaryIncomplete".into(), json!(ctx.incomplete));
    }
    let mut query_entities = Map::new();
    for name in &typed {
        if let Some(alias) = st.reg.alias(typed_type(name), name) {
            query_entities.insert(name.clone(), json!(alias));
        }
    }
    if !query_entities.is_empty() {
        out.insert("queryEntities".into(), Value::Object(query_entities));
    }
    if !typed.is_empty() {
        out.insert(
            "guidance".into(),
            json!("A name typed in a query stays in the transcript; to search for someone without naming them, use ref: with a ref from an entities table."),
        );
    }
    if !st.dropped.is_empty() {
        out.insert("dropped".into(), json!(st.dropped));
    }
    if value.to_string().chars().count() > CAP {
        return Err(PipelineError::TooLarge);
    }
    Ok(value)
}

/// String fields `tool`'s result holds that no policy names, for the
/// coverage tests: each must get a rule when it is added.
#[cfg(test)]
pub fn unlisted(tool: Tool, v: &Value) -> Vec<String> {
    fn go(tool: Tool, v: &Value, p: Policy, path: &str, out: &mut Vec<String>) {
        match v {
            Value::String(_) if p == Policy::Unlisted => out.push(path.to_string()),
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    go(tool, item, p, &format!("{path}/{i}"), out);
                }
            }
            Value::Object(map) if !matches!(p, Policy::Drop | Policy::Image) => {
                for (k, child) in map {
                    let cp = if p == Policy::Person
                        && matches!(k.as_str(), "name" | "email" | "response")
                    {
                        Policy::Plain
                    } else {
                        policy(tool, k)
                    };
                    go(tool, child, cp, &format!("{path}/{k}"), out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    go(tool, v, Policy::Within, "", &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::ident::TokenKind;
    use crate::privacy::words;

    fn keys() -> Keys {
        Keys::derive(&[5; 32])
    }

    fn run_as(tool: Tool, query: Option<&str>, names: &Names, v: Value) -> Value {
        let ctx = Context {
            tool,
            query,
            you: Some("sam@okafor.example"),
            names,
            incomplete: &[],
        };
        run(v, &keys(), &ctx).unwrap()
    }

    fn search() -> Value {
        json!({
            "messages": [{
                "messageId": "Qm3vT8pLx2NaK7cD",
                "threadId": "8f2c.v3@ruiz-events.example",
                "date": "2026-09-29T16:42:00+00:00",
                "from": "Dana Ruiz <dana@ruiz-events.example>",
                "to": ["Sam Okafor <sam@okafor.example>", "ops@okafor.example"],
                "subject": "Venue contract v3",
                "snippet": "v3 attached; deposit due Oct 10. DANA RUIZ, Events Manager. See https://ruiz-events.example/c?id=7 or call +1 415 555 0132.",
            }],
            "estimatedTotal": 3,
            "nextPageToken": "20",
            "provenance": "subject, names and addresses can be written by the sender",
            "aNewIdField": "vol~node-123",
        })
    }

    #[test]
    fn a_search_result_keeps_no_name_id_or_number() {
        let out = run_as(Tool::SearchThreads, None, &Names::default(), search());
        let text = out.to_string();
        for leak in [
            "Dana",
            "DANA",
            "Ruiz",
            "Okafor",
            "dana@",
            "ruiz-events",
            "Qm3vT8pLx2NaK7cD",
            "8f2c",
            "415",
            "vol~node",
        ] {
            assert!(!text.contains(leak), "{leak} leaked: {text}");
        }
        let row = &out["messages"][0];
        assert_eq!(row["subject"], "Venue contract v3");
        assert_eq!(row["date"], "2026-09-29T16:42:00+00:00");
        assert_eq!(out["estimatedTotal"], 3);
        assert_eq!(out["dropped"], json!(["/aNewIdField"]));
        assert_eq!(out["detectors"], json!(DETECTORS));
        let k = keys();
        assert_eq!(
            k.open_handle(ItemKind::Message, row["messageId"].as_str().unwrap())
                .unwrap(),
            "Qm3vT8pLx2NaK7cD"
        );
        assert_eq!(
            k.open_token(
                TokenKind::MailCursor,
                out["nextPageToken"].as_str().unwrap()
            )
            .unwrap(),
            "20"
        );
        // "Name <email>": the name and the address each become an alias, linked.
        let from = row["from"].as_str().unwrap();
        let (person, email) = from.split_once(" <").unwrap();
        let email = email.trim_end_matches('>');
        let dana = &out["entities"][person];
        assert_eq!(dana["type"], "person");
        assert_eq!(dana["hints"], json!(["sender", "external", "organization"]));
        assert_eq!(dana["addresses"], json!([email]));
        assert_eq!(
            k.open_ref(dana["ref"].as_str().unwrap()).unwrap().1,
            "dana ruiz"
        );
        // The name in the snippet, in capitals, has the same alias.
        assert!(row["snippet"].as_str().unwrap().contains(person), "{row}");
        assert!(
            row["snippet"].as_str().unwrap().contains("link 1 ("),
            "{row}"
        );
        // The user's own address is marked as theirs.
        let sam = row["to"][0].as_str().unwrap().split_once(" <").unwrap().0;
        assert_eq!(
            out["entities"][sam]["hints"],
            json!(["recipient", "you", "organization"])
        );
        let ops = row["to"][1].as_str().unwrap();
        assert_eq!(
            out["entities"][ops]["hints"],
            json!(["recipient", "your-organization", "organization"])
        );
    }

    #[test]
    fn the_same_value_has_the_same_alias_in_every_result() {
        let a = run_as(Tool::SearchThreads, None, &Names::default(), search());
        let b = run_as(
            Tool::GetMessage,
            None,
            &Names::default(),
            json!({ "message": { "from": "Dana Ruiz <dana@ruiz-events.example>" } }),
        );
        assert_eq!(a["messages"][0]["from"], b["message"]["from"]);
    }

    /// One result for each kind of identifier the pipeline makes: aliases
    /// of every type the detectors find, refs, a handle of each kind, page
    /// tokens and keyed digests.
    fn corpus() -> Vec<(Tool, Value)> {
        vec![
            (Tool::SearchThreads, search()),
            (
                Tool::SearchThreads,
                json!({ "messages": [{
                    "from": "Dana Ruiz <dana@ruiz-events.example>",
                    "subject": "Card 4111 1111 1111 1111, IBAN GB82 WEST 1234 5698 7654 32",
                    "snippet": "From 192.0.2.17; SSN 123-45-6789; code: 4821; see /home/dana/notes",
                }] }),
            ),
            (
                Tool::GetEvent,
                json!({ "event": { "eventId": "uid-77@ruiz-events.example|20261001T090000Z",
                    "summary": "Site visit with Dana Ruiz",
                    "organizer": { "name": "Dana Ruiz", "email": "dana@ruiz-events.example" } } }),
            ),
            (
                Tool::ListFolder,
                json!({ "path": "/Clients", "items": [{
                    "path": "/Clients/dana@ruiz-events.example notes.txt", "kind": "file",
                    "claimedSha1": "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d" }] }),
            ),
            (
                Tool::GetFileMetadata,
                json!({ "file": { "path": "/Clients/a.txt",
                    "sha256": "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae" } }),
            ),
        ]
    }

    /// The corpus's results under keys derived from `key`.
    fn corpus_under(key: &[u8; 32]) -> Vec<Value> {
        let keys = Keys::derive(key);
        corpus()
            .into_iter()
            .map(|(tool, v)| {
                let ctx = Context {
                    tool,
                    query: None,
                    you: Some("sam@okafor.example"),
                    names: &Names::default(),
                    incomplete: &[],
                };
                run(v, &keys, &ctx).unwrap()
            })
            .collect()
    }

    /// Each identifier in `results`, with its kind: an alias by its type, a
    /// ref, or the member that holds a handle, page token or keyed digest.
    fn identifiers(results: &[Value]) -> Vec<(String, String)> {
        fn walk(v: &Value, out: &mut Vec<(String, String)>) {
            match v {
                Value::Object(map) => {
                    for (k, child) in map {
                        match (k.as_str(), child) {
                            ("entities", Value::Object(rows)) => {
                                for (alias, row) in rows {
                                    let t = row["type"].as_str().unwrap();
                                    out.push((format!("{t} alias"), alias.clone()));
                                    out.push(("ref".into(), row["ref"].as_str().unwrap().into()));
                                }
                            }
                            (
                                "messageId" | "threadId" | "eventId" | "fileId" | "folderId"
                                | "nextPageToken" | "sha256" | "claimedSha1",
                                Value::String(s),
                            ) => out.push((k.clone(), s.clone())),
                            _ => walk(child, out),
                        }
                    }
                }
                Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for r in results {
            walk(r, &mut out);
        }
        out
    }

    /// RFC section 7, Stability: the corpus under one key gives the same
    /// aliases, refs, handles, page tokens and keyed digests from keys
    /// derived again, and in a second process; under another key, every one
    /// of them differs.
    #[test]
    fn identifiers_are_stable_under_one_key_and_all_differ_under_another() {
        const OUT: &str = "PROTONCTL_TEST_STABILITY_OUT";
        let ours = corpus_under(&[5; 32]);
        if let Some(path) = std::env::var_os(OUT) {
            // The second process: hand the results to the first.
            std::fs::write(path, serde_json::to_vec(&ours).unwrap()).unwrap();
            return;
        }
        let ids = identifiers(&ours);
        let kinds: BTreeSet<&str> = ids.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            kinds,
            BTreeSet::from([
                "account alias",
                "card alias",
                "claimedSha1",
                "domain alias",
                "email alias",
                "eventId",
                "fileId",
                "folderId",
                "iban alias",
                "ip alias",
                "messageId",
                "national_id alias",
                "nextPageToken",
                "person alias",
                "phone alias",
                "ref",
                "secret alias",
                "sha256",
                "threadId",
            ]),
            "the corpus holds every kind"
        );
        assert_eq!(corpus_under(&[5; 32]), ours, "keys derived again");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("results.json");
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "privacy::pipeline::tests::identifiers_are_stable_under_one_key_and_all_differ_under_another",
                "--test-threads=1",
            ])
            .env(OUT, &file)
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{}",
            String::from_utf8_lossy(&child.stderr)
        );
        let theirs: Vec<Value> = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert_eq!(theirs, ours, "a second process");
        let other = Value::from(corpus_under(&[6; 32])).to_string();
        for (kind, id) in &ids {
            assert!(
                !other.contains(id.as_str()),
                "{kind} {id} under another key"
            );
        }
    }

    /// RFC section 7, Collisions: with a word list of 4 words, two and three
    /// entities that share three words in one result each get their own
    /// alias, more words in the order of their refs, whatever the order of
    /// the input.
    #[test]
    fn entities_sharing_three_words_get_distinct_aliases_in_any_order() {
        const SHORT: &[&str] = &["amber", "falcon", "river", "stone"];
        let k = keys();
        words::with_list(SHORT, || {
            let mut by_words: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for i in 0..200 {
                let email = format!("p{i}@corp.example");
                let three = k.alias(EntityType::Email, &email, 0);
                by_words.entry(three).or_default().push(email);
            }
            let triple = by_words.values().find(|g| g.len() >= 3).unwrap()[..3].to_vec();
            let pair = by_words
                .values()
                .find(|g| g.len() >= 2 && g[0] != triple[0])
                .unwrap()[..2]
                .to_vec();
            // The rule: three words for the first by ref, then one more each.
            let mut expected = BTreeMap::new();
            for group in [&pair, &triple] {
                let mut by_ref = group.clone();
                by_ref.sort_by_key(|e| k.reference(EntityType::Email, e));
                for (extra, e) in by_ref.into_iter().enumerate() {
                    let alias = k.alias(EntityType::Email, &e, extra);
                    assert_eq!(alias.split('-').count(), 3 + extra);
                    assert!(alias.split('-').all(|w| SHORT.contains(&w)), "{alias}");
                    expected.insert(e, alias);
                }
            }
            let addresses: Vec<String> = pair.iter().chain(&triple).cloned().collect();
            for turn in 0..2 * addresses.len() {
                // Each address first in turn, forwards and backwards.
                let mut order = addresses.clone();
                order.rotate_left(turn % addresses.len());
                if turn >= addresses.len() {
                    order.reverse();
                }
                let v = json!({ "messages": [{ "to": order, "subject": "x" }] });
                let out = run_as(Tool::SearchThreads, None, &Names::default(), v);
                let shown: Vec<&str> = out["messages"][0]["to"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap())
                    .collect();
                let distinct: BTreeSet<&str> = shown.iter().copied().collect();
                assert_eq!(distinct.len(), 5, "{shown:?}");
                assert_eq!(out["entities"].as_object().unwrap().len(), 5, "{out}");
                let got: BTreeMap<String, String> = order
                    .iter()
                    .cloned()
                    .zip(shown.iter().map(ToString::to_string))
                    .collect();
                assert_eq!(got, expected, "{order:?}");
            }
        });
    }

    #[test]
    fn a_name_typed_in_the_query_is_paired_with_its_alias() {
        let out = run_as(
            Tool::SearchThreads,
            Some("from:\"Dana Ruiz\" contract"),
            &Names::default(),
            search(),
        );
        let alias = out["queryEntities"]["Dana Ruiz"].as_str().unwrap();
        assert!(
            out["messages"][0]["from"]
                .as_str()
                .unwrap()
                .starts_with(alias)
        );
        assert!(out["guidance"].as_str().unwrap().len() < 200);
        let plain = run_as(
            Tool::SearchThreads,
            Some("contract"),
            &Names::default(),
            search(),
        );
        assert!(plain.get("queryEntities").is_none() && plain.get("guidance").is_none());
    }

    #[test]
    fn drive_paths_keep_their_shape_and_carry_a_handle() {
        let mut names = Names::default();
        names.add("Dana Ruiz", EntityType::Person);
        let path = "/Projects/Interview – Da\\u{200B}na Ruiz.docx";
        let out = run_as(
            Tool::ListFolder,
            None,
            &names,
            json!({
                "path": "/Projects",
                "items": [{ "path": path, "name": "Interview – Dana Ruiz.docx", "kind": "file",
                    "size": 10, "nodeId": "vol~x", "claimedSha1": "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d",
                    "localModified": "2026-09-30T10:00:00Z" }],
            }),
        );
        let item = &out["items"][0];
        let shown = item["path"].as_str().unwrap();
        assert!(
            shown.starts_with("/Projects/Interview – ")
                && std::path::Path::new(shown)
                    .extension()
                    .is_some_and(|x| x == "docx"),
            "{shown}"
        );
        assert!(!shown.contains("Ruiz"), "{shown}");
        assert_eq!(
            keys()
                .open_handle(ItemKind::DrivePath, item["fileId"].as_str().unwrap())
                .unwrap(),
            path
        );
        assert!(item.get("nodeId").is_none());
        assert_eq!(item["claimedSha1"].as_str().unwrap().split('-').count(), 5);
        // A listing's own path is the folder it lists.
        assert!(
            out["folderId"].is_string() && out.get("fileId").is_none(),
            "{out}"
        );
    }

    #[test]
    fn local_paths_and_errors_become_fixed_text() {
        let out = run_as(
            Tool::GetStatus,
            None,
            &Names::default(),
            json!({ "config": "/Users/sam/.config/protonctl/config.toml",
                    "drive": { "error": "cannot open /Users/sam/Library/CloudStorage/ProtonDrive-sam@okafor.example-folder" },
                    "calendars": [{ "calendarId": "personal", "error": "calendar fetch failed: Dana's server" }] }),
        );
        assert_eq!(out["config"], LOCAL_PATH);
        assert_eq!(out["drive"]["error"], "unavailable");
        assert_eq!(out["calendars"][0]["error"], "unavailable");
        assert!(!out.to_string().contains("sam"), "{out}");
    }

    #[test]
    fn people_in_events_become_aliases() {
        let out = run_as(
            Tool::GetEvent,
            None,
            &Names::default(),
            json!({ "summary": "Site visit with Dana Ruiz", "organizer": { "name": "Dana Ruiz", "email": "dana@ruiz-events.example" },
                    "attendees": [{ "name": "Sam Okafor", "email": "sam@okafor.example", "response": "accepted" }] }),
        );
        let text = out.to_string();
        assert!(!text.contains("Dana") && !text.contains("Okafor"), "{text}");
        let organizer = out["organizer"]["name"].as_str().unwrap();
        assert!(out["summary"].as_str().unwrap().ends_with(organizer));
        assert_eq!(out["attendees"][0]["response"], "accepted");
        assert_eq!(out["entities"][organizer]["hints"][0], "organizer");
    }

    #[test]
    fn the_coverage_check_names_a_field_without_a_policy() {
        let v = json!({ "messages": [{ "subject": "x", "mystery": "y", "unread": true }] });
        assert_eq!(unlisted(Tool::SearchThreads, &v), ["/messages/0/mystery"]);
    }

    /// RFC section 6: every string field in the mail results the mail tests
    /// snapshot has an aliases-mode policy, so none is dropped unread.
    #[test]
    fn every_mail_snapshot_field_has_a_policy() {
        macro_rules! snap {
            ($name:literal) => {
                include_str!(concat!(
                    "../mail/snapshots/protonctl__mail__read__tests__",
                    $name,
                    ".snap"
                ))
            };
        }
        for (tool, snap) in [
            (Tool::CountMessages, snap!("count_by_from")),
            (Tool::GetMessage, snap!("get_message")),
            (Tool::GetThread, snap!("get_thread")),
            (Tool::ListLabels, snap!("list_labels")),
            (Tool::SearchThreads, snap!("search_default")),
            (Tool::SearchThreads, snap!("search_has_attachment")),
            (Tool::SearchThreads, snap!("search_include_trash")),
        ] {
            let body = snap.splitn(3, "---\n").nth(2).expect("an insta header");
            let v: Value = serde_json::from_str(body).unwrap();
            assert_eq!(unlisted(tool, &v), Vec::<String>::new(), "{tool:?}");
        }
    }

    /// A mention that ends in a character of more than one byte, a name or
    /// a link, is rewritten whole; a hidden character inside it is no gap.
    #[test]
    fn mentions_ending_in_a_multi_byte_character_are_rewritten() {
        let v = json!({
            "messages": [{
                "from": "Ana José <ana@ruiz-events.example>",
                "subject": "Lunch with Ana José, see https://ruiz-events.example/café",
                "snippet": "Thanks, Ana Jos\u{200B}é",
            }],
        });
        let out = run_as(Tool::SearchThreads, None, &Names::default(), v).to_string();
        for leak in ["Ana", "café"] {
            assert!(!out.contains(leak), "{leak} in {out}");
        }
        assert!(out.contains("link 1"), "{out}");
    }

    /// A domain outside the short list of top-level domains is found when
    /// an address in the result has it, and a group key is aliased whole.
    #[test]
    fn domains_of_addresses_and_group_keys_are_aliased() {
        let by_domain =
            json!({ "by": "fromDomain", "groups": [{ "key": "ruiz-law.pl", "count": 2 }] });
        let by_sender =
            json!({ "by": "from", "groups": [{ "key": "o'brien@firm.law", "count": 1 }] });
        let message = json!({ "message": {
            "from": "Dana Ruiz <dana@ruiz-law.pl>",
            "authentication": "dkim=pass (header.d=ruiz-law.pl); spf=pass",
            "body": "Write to o'brien@firm.law.",
        } });
        for (tool, v) in [
            (Tool::CountMessages, by_domain),
            (Tool::CountMessages, by_sender),
            (Tool::GetMessage, message),
        ] {
            let out = run_as(tool, None, &Names::default(), v).to_string();
            for leak in ["ruiz-law", "brien", "firm.law", "o'"] {
                assert!(!out.contains(leak), "{leak} in {out}");
            }
        }
    }

    /// A sender writes an attachment's MIME type: a known type stays, any
    /// other becomes its top-level type and "other".
    #[test]
    fn unknown_mime_types_are_not_passed_on() {
        let v = json!({ "message": { "attachments": [
            { "index": 0, "mimeType": "application/pdf" },
            { "index": 1, "mimeType": "application/dana.ruiz_dana@ruiz-events.example" },
            { "index": 2, "mimeType": "x-dana/ruiz" },
        ] } });
        let out = run_as(Tool::GetMessage, None, &Names::default(), v);
        let types: Vec<&str> = out["message"]["attachments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["mimeType"].as_str().unwrap())
            .collect();
        assert_eq!(types, ["application/pdf", "application/other", "other"]);
    }

    /// A From with a name and no address, which mail shows as "Name <>",
    /// or a bare name, is a person: the page is rewritten, not failed.
    #[test]
    fn an_address_without_an_email_is_a_person() {
        let v = json!({ "messages": [{
            "from": "Dana Ruiz <>",
            "to": ["Sam Okafor", "ops@okafor.example"],
            "subject": "Dana Ruiz and Sam Okafor",
        }] });
        let out = run_as(Tool::SearchThreads, None, &Names::default(), v);
        let text = out.to_string();
        for leak in ["Dana", "Sam", "ops@", "<>"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
        let types: Vec<&str> = out["entities"]
            .as_object()
            .unwrap()
            .values()
            .map(|e| e["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            types.iter().filter(|t| **t == "person").count(),
            2,
            "{text}"
        );
    }

    /// The pipeline's time on the largest page a tool returns: 40,000
    /// characters (`maxChars`' limit) dense with names, addresses and phone
    /// numbers, against a dictionary of 5,000 correspondents. A measurement,
    /// not a gate:
    /// `cargo test --release time_on_a_full_page -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement; run it with --release"]
    fn time_on_a_full_page() {
        use std::fmt::Write as _;
        const FIRST: [&str; 50] = [
            "Dana", "Sam", "Ana", "Jon", "Mia", "Leo", "Ivy", "Max", "Zoe", "Eli", "Ada", "Kai",
            "Noa", "Ren", "Uma", "Ola", "Tess", "Hugo", "Lena", "Omar", "Ines", "Yuki", "Ravi",
            "Sofia", "Pablo", "Grace", "Felix", "Clara", "Mateo", "Hana", "Luca", "Nora", "Aria",
            "Emil", "Iris", "Theo", "Vera", "Wade", "Xena", "Yara", "Zane", "Bea", "Cyrus",
            "Dora", "Ezra", "Fay", "Gus", "Hope", "Ian", "Jade",
        ];
        const LAST: [&str; 100] = [
            "Ruiz", "Okafor", "Chen", "Patel", "Novak", "Silva", "Kim", "Haddad", "Larsen",
            "Moreau", "Rossi", "Tanaka", "Weber", "Costa", "Ivanova", "Nguyen", "Kowalski",
            "Dubois", "Schmidt", "Fischer", "Romero", "Bauer", "Sato", "Ito", "Ahmed", "Ali",
            "Khan", "Singh", "Mehta", "Rao", "Bose", "Das", "Gupta", "Iyer", "Joshi", "Kapoor",
            "Lal", "Malik", "Nair", "Pillai", "Reddy", "Sharma", "Verma", "Yadav", "Abbott",
            "Baker", "Carter", "Dawson", "Ellis", "Foster", "Gibson", "Hayes", "Irwin",
            "Jensen", "Keller", "Lowe", "Mason", "Nolan", "Owens", "Price", "Quinn", "Reyes",
            "Shaw", "Tate", "Underwood", "Vance", "Walsh", "Young", "Zimmer", "Adler", "Brandt",
            "Cohen", "Dahl", "Eder", "Frey", "Graf", "Hahn", "Imhof", "Jung", "Kraus", "Lang",
            "Mayer", "Nagel", "Otto", "Pohl", "Roth", "Stein", "Thiel", "Ulrich", "Vogel",
            "Wolf", "Ziegler", "Arnaud", "Blanc", "Caron", "Dumas", "Fabre", "Girard", "Henry",
            "Leroy",
        ];
        let people: Vec<String> = FIRST
            .iter()
            .flat_map(|f| LAST.iter().map(move |l| format!("{f} {l}")))
            .collect();
        assert_eq!(people.len(), 5_000);
        let names = Names::people(people.clone());
        let ctx = Context {
            tool: Tool::ReadFileContent,
            query: None,
            you: None,
            names: &names,
            incomplete: &[],
        };
        let keys = keys();
        let mut said = String::new();
        // 50 or 200 people, each with an address and a number, on pages of
        // `maxChars`' default and its limit.
        for (distinct, size) in [(50, 20_000), (50, 40_000), (200, 20_000), (200, 40_000)] {
            let mut text = String::new();
            let step = people.len() / distinct;
            for (i, name) in people.iter().step_by(step).enumerate().cycle() {
                let mail = name.to_lowercase().replace(' ', ".");
                let line = format!(
                    "On Monday {name} wrote to {mail}@example.com about the lease; call +1 415 555 {:04}.\n",
                    100 + i
                );
                if text.len() + line.len() > size {
                    break;
                }
                text.push_str(&line);
            }
            let page = json!({ "content": text, "offset": 0, "nextOffset": size });
            let mut took = Vec::new();
            let mut out = Err(PipelineError::Failed);
            for _ in 0..5 {
                let started = std::time::Instant::now();
                out = run(page.clone(), &keys, &ctx);
                took.push(started.elapsed());
            }
            took.sort();
            let outcome = match &out {
                Ok(v) => format!(
                    "{} characters out, {} entities",
                    v.to_string().chars().count(),
                    v["entities"].as_object().unwrap().len()
                ),
                Err(e) => format!("{e:?}"),
            };
            writeln!(
                said,
                "{distinct} people, {size} characters in: {outcome}; 5 runs, median {} ms, slowest {} ms",
                took[2].as_millis(),
                took[4].as_millis()
            )
            .unwrap();
            // The work measured is the whole pipeline's: with 50 people the
            // page passes, with every name, address and number found.
            if distinct == 50 {
                let found = out.as_ref().map(|v| v["entities"].as_object().unwrap().len());
                assert!(found.is_ok_and(|n| n == 3 * 50), "{said}");
            }
        }
        eprint!("{said}");
    }

    #[test]
    fn a_result_over_the_cap_is_refused() {
        let big = json!({ "content": "word ".repeat(CAP / 4) });
        let ctx = Context {
            tool: Tool::ReadFileContent,
            query: None,
            you: None,
            names: &Names::default(),
            incomplete: &[],
        };
        assert_eq!(
            run(big, &keys(), &ctx).unwrap_err(),
            PipelineError::TooLarge
        );
    }
}
