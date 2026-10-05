//! protonctl keeps secrets only in the system's secret store (RFC R2): the
//! login Keychain on macOS, and the Secret Service on Linux (RFC section 11,
//! Q31).
//! Items live under service `protonctl`; `Account` names them.

use anyhow::{Result, anyhow};
use secrecy::{ExposeSecret as _, SecretString};

use crate::platform;

/// Every secret protonctl holds in memory: `Debug` shows it redacted, and
/// `expose_secret()` marks each place that reads it.
pub type Secret = SecretString;

const SERVICE: &str = "protonctl";

/// An item under service `protonctl`. Its account name in the store is the
/// only place these are text: an item is named from this type when written,
/// and parsed back into it when listed.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Account {
    /// `calendar/<id>`: a calendar's share link.
    Calendar(String),
    /// `bridge/<address>`: the Bridge password for an address.
    Bridge(String),
    /// `privacy-mode`: the mode, `off` or `aliases`, in its comment (RFC Q28).
    PrivacyMode,
    /// `privacy-key`: the privacy key, with its key ID in the comment (R20).
    PrivacyKey,
    /// Any other name under the service, from an older version: listed and
    /// deleted by `logout`, never read.
    Other(String),
}

impl Account {
    fn parse(name: &str) -> Self {
        match name.split_once('/') {
            Some(("calendar", id)) => Self::Calendar(id.to_string()),
            Some(("bridge", address)) => Self::Bridge(address.to_string()),
            _ => match name {
                "privacy-mode" => Self::PrivacyMode,
                "privacy-key" => Self::PrivacyKey,
                other => Self::Other(other.to_string()),
            },
        }
    }
}

impl std::fmt::Display for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Calendar(id) => write!(f, "calendar/{id}"),
            Self::Bridge(address) => write!(f, "bridge/{address}"),
            Self::PrivacyMode => f.write_str("privacy-mode"),
            Self::PrivacyKey => f.write_str("privacy-key"),
            Self::Other(name) => f.write_str(name),
        }
    }
}

pub fn get(account: &Account) -> Result<Option<Secret>> {
    let Some(bytes) = platform::secret_get(SERVICE, &account.to_string())? else {
        return Ok(None);
    };
    #[expect(
        clippy::map_err_ignore,
        reason = "the UTF-8 error owns the secret's bytes, and {:#?} would print them"
    )]
    String::from_utf8(bytes)
        .map(|s| Some(Secret::from(s)))
        .map_err(|_| {
            let store = platform::SECRET_STORE;
            anyhow!("{store} item {SERVICE}/{account} is not UTF-8")
        })
}

pub fn set(account: &Account, value: &Secret) -> Result<()> {
    platform::secret_set(
        SERVICE,
        &account.to_string(),
        value.expose_secret().as_bytes(),
    )
}

/// Returns false when there was nothing to delete.
pub fn delete(account: &Account) -> Result<bool> {
    platform::secret_delete(SERVICE, &account.to_string())
}

/// Every account protonctl holds under its service, including any whose
/// calendar has left the config. Reads attributes only, so no secret is loaded
/// and macOS asks for no approval.
pub fn accounts() -> Result<Vec<Account>> {
    Ok(platform::secret_accounts(SERVICE)?
        .iter()
        .map(|name| Account::parse(name))
        .collect())
}

/// Store `value` with a non-secret `comment` beside it, in one update (RFC
/// Q28, R20: the privacy mode and the privacy key's ID).
pub fn set_with_comment(account: &Account, value: &Secret, comment: &str) -> Result<()> {
    let name = account.to_string();
    platform::secret_set_with_comment(SERVICE, &name, value.expose_secret().as_bytes(), comment)
}

/// An item's comment, read without loading its secret.
pub fn comment(account: &Account) -> Result<Option<String>> {
    platform::secret_comment(SERVICE, &account.to_string())
}

/// How status and logout name an item: the store, then service/account.
pub fn shown(account: &Account) -> String {
    format!("{} {SERVICE}/{account}", platform::SECRET_STORE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_names_round_trip() {
        for account in [
            Account::Calendar("personal".into()),
            Account::Bridge("you@proton.me".into()),
            Account::PrivacyMode,
            Account::PrivacyKey,
            Account::Other("legacy".into()),
        ] {
            assert_eq!(Account::parse(&account.to_string()), account);
        }
        // An ID may hold a slash; only the first one splits.
        assert_eq!(
            Account::parse("calendar/a/b"),
            Account::Calendar("a/b".into())
        );
    }

    #[test]
    fn secrets_are_redacted_in_debug_output() {
        let secret = Secret::from("hunter2-not-a-real-secret");
        let shown = format!("{secret:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
    }
}
