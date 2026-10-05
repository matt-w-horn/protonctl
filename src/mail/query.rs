//! The Gmail-style query language of `search_threads` and `count_messages`,
//! compiled into the mailbox to search, IMAP SEARCH criteria, and an
//! attachment test that runs on each fetched summary.

use anyhow::{Result, bail};
use async_imap::types::NameAttribute;
use chrono::{Duration, NaiveDate};

use crate::content::{Invalid, tokens, unquote};

const OPERATORS: [&str; 13] = [
    "from",
    "to",
    "cc",
    "bcc",
    "subject",
    "label",
    "in",
    "is",
    "has",
    "after",
    "before",
    "newer_than",
    "older_than",
];

/// Where a query searches.
#[derive(Clone, Debug, PartialEq)]
pub enum Mailbox {
    /// All Mail, minus Trash and Spam unless includeTrash.
    All,
    /// INBOX, which has no special-use attribute.
    Inbox,
    /// The mailbox with this special-use attribute.
    Role(NameAttribute<'static>),
    Label(String),
}

/// A query as one mailbox, IMAP SEARCH criteria and an attachment test.
#[derive(Debug, PartialEq)]
pub struct Query {
    pub mailbox: Mailbox,
    pub criteria: String,
    /// has:attachment (true) or -has:attachment (false), tested on each fetched
    /// summary: Bridge matches every message for `HEADER X-Attached ""`.
    pub attachment: Option<bool>,
}

/// An IMAP quoted string. The value is sent as UTF-8; CR, LF and NUL are refused.
pub(super) fn quoted(s: &str) -> Result<String> {
    if s.contains(['\r', '\n', '\0']) {
        bail!(Invalid::rule("search terms cannot contain line breaks"));
    }
    Ok(format!(
        "\"{}\"",
        s.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn date(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(&value.replace('/', "-"), "%Y-%m-%d").map_err(|e| {
        Invalid::quoting(
            "A date in the query cannot be read; use YYYY-MM-DD.",
            format!("cannot read date {value:?}; use YYYY-MM-DD: {e}"),
        )
        .into()
    })
}

/// IMAP dates look like 2-Oct-2026.
fn imap_date(d: NaiveDate) -> String {
    d.format("%-d-%b-%Y").to_string()
}

/// A span such as 7d, 3m or 1y (a month is 30 days, a year 365). Every step
/// is checked: the value comes from the query, and chrono panics on overflow.
fn span(value: &str) -> Result<Duration> {
    let usage = |cause: String| {
        Invalid::quoting(
            "A span in newer_than: or older_than: cannot be read; use 7d, 3m or 1y.",
            format!("cannot read {value:?}; use 7d, 3m or 1y{cause}"),
        )
    };
    let (cut, unit) = value
        .char_indices()
        .last()
        .ok_or_else(|| usage(String::new()))?;
    let n: i64 = value[..cut].parse().map_err(|e| usage(format!(": {e}")))?;
    let days = match unit {
        'd' => Some(n),
        'm' => n.checked_mul(30),
        'y' => n.checked_mul(365),
        _ => bail!(usage(String::new())),
    };
    days.and_then(Duration::try_days).ok_or_else(|| {
        Invalid::quoting(
            "A span in newer_than: or older_than: is too long.",
            format!("{value:?} is too long"),
        )
        .into()
    })
}

/// The day that the span `value` reaches back to from `today`.
fn day_before(today: NaiveDate, value: &str) -> Result<NaiveDate> {
    today.checked_sub_signed(span(value)?).ok_or_else(|| {
        Invalid::quoting(
            "A span in newer_than: or older_than: reaches outside the calendar.",
            format!("{value:?} reaches outside the calendar"),
        )
        .into()
    })
}

/// The mailbox an `in:` value names: a special-use role, INBOX, or All Mail.
fn in_mailbox(value: &str) -> Result<Mailbox> {
    Ok(match value.to_ascii_lowercase().as_str() {
        "inbox" => Mailbox::Inbox,
        "sent" => Mailbox::Role(NameAttribute::Sent),
        "drafts" => Mailbox::Role(NameAttribute::Drafts),
        "archive" => Mailbox::Role(NameAttribute::Archive),
        "starred" => Mailbox::Role(NameAttribute::Flagged),
        "spam" => Mailbox::Role(NameAttribute::Junk),
        "trash" => Mailbox::Role(NameAttribute::Trash),
        "anywhere" | "all" => Mailbox::All,
        _ => bail!(Invalid::quoting(
            "This in: value is not supported; use inbox, sent, drafts, archive, starred, spam or trash.",
            format!(
                "in:{value} is not supported; use inbox, sent, drafts, archive, starred, spam or trash"
            )
        )),
    })
}

/// Translate the Gmail-style subset into a [`Query`].
#[expect(
    clippy::too_many_lines,
    reason = "one pass over the tokens, each operator's rule beside its error"
)]
pub fn compile(query: &str, today: NaiveDate) -> Result<Query> {
    let mut mailbox: Option<Mailbox> = None;
    let mut attachment: Option<bool> = None;
    let mut terms: Vec<String> = Vec::new();
    // OR joins the term just added to the next one; in:, label: and has: add none.
    let mut after_term = false;
    let mut or_pending = false;
    for token in tokens(query)? {
        if token == "OR" {
            if !after_term || or_pending {
                bail!(Invalid::rule(
                    "OR needs a search term on each side; in:, label: and has: are not search terms"
                ));
            }
            or_pending = true;
            continue;
        }
        let (negated, token) = match token.strip_prefix('-') {
            Some(rest) if !rest.is_empty() => (true, rest),
            _ => (false, token.as_str()),
        };
        let operator = token
            .split_once(':')
            .filter(|(op, _)| OPERATORS.contains(&op.to_ascii_lowercase().as_str()));
        let criterion = match operator {
            None => format!("TEXT {}", quoted(unquote(token))?),
            Some((op, value)) => {
                let value = unquote(value);
                match op.to_ascii_lowercase().as_str() {
                    "in" | "label" => {
                        if negated || or_pending || mailbox.is_some() {
                            bail!(Invalid::rule(
                                "use at most one in: or label:, without - or OR"
                            ));
                        }
                        mailbox = Some(if op.eq_ignore_ascii_case("label") {
                            Mailbox::Label(value.to_string())
                        } else {
                            in_mailbox(value)?
                        });
                        after_term = false;
                        continue;
                    }
                    "is" => match value.to_ascii_lowercase().as_str() {
                        "unread" => "UNSEEN".to_string(),
                        "read" => "SEEN".to_string(),
                        "starred" => "FLAGGED".to_string(),
                        _ => bail!(Invalid::quoting(
                            "This is: value is not supported; use unread, read or starred.",
                            format!("is:{value} is not supported; use unread, read or starred")
                        )),
                    },
                    "has" if value.eq_ignore_ascii_case("attachment") => {
                        if or_pending || attachment.is_some() {
                            bail!(Invalid::rule("use has:attachment at most once, without OR"));
                        }
                        attachment = Some(!negated);
                        after_term = false;
                        continue;
                    }
                    "has" => bail!(Invalid::quoting(
                        "This has: value is not supported; use has:attachment.",
                        format!("has:{value} is not supported; use has:attachment")
                    )),
                    "after" => format!("SINCE {}", imap_date(date(value)?)),
                    "before" => format!("BEFORE {}", imap_date(date(value)?)),
                    "newer_than" => format!("SINCE {}", imap_date(day_before(today, value)?)),
                    "older_than" => format!("BEFORE {}", imap_date(day_before(today, value)?)),
                    field => format!("{} {}", field.to_ascii_uppercase(), quoted(value)?),
                }
            }
        };
        let criterion = if negated {
            format!("NOT {criterion}")
        } else {
            criterion
        };
        match (or_pending, terms.pop()) {
            (true, Some(left)) => terms.push(format!("OR {left} {criterion}")),
            (_, left) => {
                terms.extend(left);
                terms.push(criterion);
            }
        }
        or_pending = false;
        after_term = true;
    }
    if or_pending {
        bail!(Invalid::rule(
            "OR needs a search term on each side; in:, label: and has: are not search terms"
        ));
    }
    if terms.is_empty() {
        terms.push("ALL".into());
    }
    let criteria = terms.join(" ");
    let criteria = if criteria.is_ascii() {
        criteria
    } else {
        format!("CHARSET UTF-8 {criteria}")
    };
    Ok(Query {
        mailbox: mailbox.unwrap_or(Mailbox::All),
        criteria,
        attachment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Query-like text: operators with odd values, quotes, negation, OR and
    /// anything else.
    fn query_text() -> impl Strategy<Value = String> {
        let token = prop_oneof![
            any::<String>(),
            "(from|to|subject|label|in|is|has):[a-z0-9/\"\\\\-]{0,10}",
            "(newer_than|older_than):-?[0-9]{1,20}[dmyé]?",
            "(newer_than|older_than|after|before):\\PC{0,6}",
            "(after|before):[0-9]{1,6}[-/][0-9]{1,3}[-/][0-9]{1,3}",
            Just("OR".to_string()),
            Just("\"".to_string()),
            Just("-".to_string()),
        ];
        prop::collection::vec(token, 0..8).prop_map(|t| t.join(" "))
    }

    proptest! {
        #[test]
        fn compile_never_panics_and_keeps_line_breaks_out(q in query_text()) {
            if let Ok(query) = compile(&q, day()) {
                prop_assert!(!query.criteria.contains(['\r', '\n', '\0']), "{:?}", query.criteria);
            }
        }

        #[test]
        fn quoted_strings_read_back(s in "\\PC*") {
            let q = quoted(&s).unwrap();
            let inner = q.strip_prefix('"').and_then(|q| q.strip_suffix('"')).unwrap();
            let (mut back, mut chars) = (String::new(), inner.chars());
            while let Some(c) = chars.next() {
                if c == '\\' {
                    let next = chars.next().unwrap();
                    prop_assert!(next == '\\' || next == '"', "{:?}", q);
                    back.push(next);
                } else {
                    prop_assert!(c != '"', "an unescaped quote in {:?}", q);
                    back.push(c);
                }
            }
            prop_assert_eq!(back, s);
        }
    }

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 3).unwrap()
    }

    fn criteria(q: &str) -> String {
        compile(q, day()).unwrap().criteria
    }

    #[test]
    fn plain_words_and_phrases_search_text() {
        assert_eq!(criteria("lease renewal"), "TEXT \"lease\" TEXT \"renewal\"");
        assert_eq!(criteria("\"lease renewal\""), "TEXT \"lease renewal\"");
        assert_eq!(criteria(""), "ALL");
    }

    #[test]
    fn operators_translate_to_imap() {
        assert_eq!(
            criteria("from:\"Ann Lee\" subject:invoice is:unread"),
            "FROM \"Ann Lee\" SUBJECT \"invoice\" UNSEEN"
        );
        assert_eq!(
            criteria("after:2026/09/01 before:2026-10-01"),
            "SINCE 1-Sep-2026 BEFORE 1-Oct-2026"
        );
        assert_eq!(
            criteria("newer_than:7d older_than:1y"),
            "SINCE 26-Sep-2026 BEFORE 3-Oct-2025"
        );
    }

    #[test]
    fn negation_and_or_nest_like_gmail() {
        assert_eq!(criteria("-is:read"), "NOT SEEN");
        assert_eq!(
            criteria("from:a OR from:b invoice"),
            "OR FROM \"a\" FROM \"b\" TEXT \"invoice\""
        );
        // The documented idiom for any recipient chains.
        assert_eq!(
            criteria("to:x OR cc:x OR bcc:x"),
            "OR OR TO \"x\" CC \"x\" BCC \"x\""
        );
        assert!(compile("OR from:a", day()).is_err());
        assert!(compile("from:a OR", day()).is_err());
        // An OR beside in: or label: must not reach past it to an earlier term.
        assert!(compile("from:b in:sent OR from:a", day()).is_err());
    }

    #[test]
    fn has_attachment_filters_summaries_instead_of_searching() {
        let q = compile("has:attachment from:a", day()).unwrap();
        assert_eq!(
            (q.criteria.as_str(), q.attachment),
            ("FROM \"a\"", Some(true))
        );
        let q = compile("-has:attachment", day()).unwrap();
        assert_eq!((q.criteria.as_str(), q.attachment), ("ALL", Some(false)));
        assert!(compile("has:attachment OR from:a", day()).is_err());
        assert!(compile("from:b has:attachment OR from:a", day()).is_err());
        assert!(compile("from:a OR has:attachment", day()).is_err());
    }

    #[test]
    fn mailboxes_come_from_in_and_label() {
        assert_eq!(
            compile("in:sent x", day()).unwrap().mailbox,
            Mailbox::Role(NameAttribute::Sent)
        );
        assert_eq!(
            compile("label:\"Project Notes\"", day()).unwrap().mailbox,
            Mailbox::Label("Project Notes".into())
        );
        assert_eq!(compile("x", day()).unwrap().mailbox, Mailbox::All);
        assert!(compile("in:sent in:trash", day()).is_err());
        assert!(compile("-label:x", day()).is_err());
    }

    #[test]
    fn unsupported_and_unsafe_input_is_refused() {
        assert!(compile("is:important", day()).is_err());
        assert!(compile("from:\"unbalanced", day()).is_err());
        // A backslash in a value, and a quote reaching the IMAP quoting, are both escaped.
        assert_eq!(criteria("from:a\\b"), "FROM \"a\\\\b\"");
        assert_eq!(quoted("a\"b").unwrap(), "\"a\\\"b\"");
        assert!(criteria("café").starts_with("CHARSET UTF-8 "));
    }
}
