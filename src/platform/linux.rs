//! Linux: the Secret Service over D-Bus (RFC Q31), XDG folders, and no
//! Drive app folder. Items carry the attributes `service` and `account`
//! (RFC lld-api, Keychain items), and `comment` where macOS has its item
//! comment. With no Secret Service running, every secret call fails, and
//! nothing is ever written to a file instead.

use std::collections::HashMap;
use std::fs::Metadata;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use secret_service::EncryptionType;
use secret_service::blocking::{Item, SecretService};

use crate::config::home;

pub const SECRET_STORE: &str = "Secret Service";
/// Linux readers wait for Phase P4 (RFC Q34).
pub const DOCUMENT_READERS: bool = false;

/// Every protonctl value is text.
const CONTENT_TYPE: &str = "text/plain";
/// The only service unit tests may reach, in the private D-Bus session that
/// `scripts/check.sh` starts; other tests never touch the user's keyring.
const TEST_SERVICE: &str = "protonctl-test";

/// What crosses the D-Bus session in a call.
#[derive(Clone, Copy, Debug)]
enum Carries {
    /// A secret: the session is encrypted, as libsecret's are by default.
    Secret,
    /// Attributes only. The per-call reads of the privacy setting are these,
    /// and an encrypted session's key exchange would cost each call more
    /// than the call itself.
    Attributes,
}

/// Run `f` against the Secret Service on a thread of its own: zbus's
/// blocking calls start their own runtime, which tokio forbids on a thread
/// that is running async code, and some callers are.
fn with_store<T: Send>(
    service: &str,
    what: &str,
    carries: Carries,
    f: impl FnOnce(&SecretService<'_>) -> Result<T> + Send,
) -> Result<T> {
    if cfg!(test) && service != TEST_SERVICE {
        bail!("unit tests never reach the user's secret store");
    }
    std::thread::scope(|s| {
        s.spawn(|| {
            let encryption = match carries {
                Carries::Secret => EncryptionType::Dh,
                Carries::Attributes => EncryptionType::Plain,
            };
            let store = SecretService::connect(encryption).map_err(|e| {
                anyhow!(
                    "cannot reach the Secret Service ({e}); protonctl keeps secrets only there, \
                     so start GNOME Keyring or KWallet in this session (RFC-0001 Q31)"
                )
            })?;
            f(&store)
        })
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
    .map_err(|e| e.context(format!("Secret Service {what} failed")))
}

fn attributes<'a>(service: &'a str, account: &'a str) -> HashMap<&'a str, &'a str> {
    HashMap::from([("service", service), ("account", account)])
}

/// The items whose attributes match, unlocked: unlocking a collection can
/// show the desktop's prompt for the keyring password.
fn search<'a>(store: &'a SecretService<'a>, attrs: HashMap<&str, &str>) -> Result<Vec<Item<'a>>> {
    let found = store.search_items(attrs)?;
    let mut items = found.unlocked;
    if !found.locked.is_empty() {
        store.unlock_all(&found.locked.iter().collect::<Vec<_>>())?;
        items.extend(found.locked);
    }
    Ok(items)
}

/// The one item for `account`, or `None`. Two would leave a reader to pick
/// one, so that is an error.
fn one<'a>(store: &'a SecretService<'a>, service: &str, account: &str) -> Result<Option<Item<'a>>> {
    let mut items = search(store, attributes(service, account))?;
    if items.len() > 1 {
        bail!(
            "{} items are stored for {service}/{account}; delete them with `protonctl logout` and set it up again",
            items.len()
        );
    }
    Ok(items.pop())
}

pub fn secret_get(service: &str, account: &str) -> Result<Option<Vec<u8>>> {
    with_store(service, "read", Carries::Secret, |store| {
        one(store, service, account)?
            .map(|item| item.get_secret().map_err(anyhow::Error::from))
            .transpose()
    })
}

pub fn secret_set(service: &str, account: &str, value: &[u8]) -> Result<()> {
    with_store(service, "write", Carries::Secret, |store| {
        match one(store, service, account)? {
            Some(item) => item.set_secret(value, CONTENT_TYPE)?,
            None => {
                store.get_default_collection()?.create_item(
                    &format!("{service}/{account}"),
                    attributes(service, account),
                    value,
                    false,
                    CONTENT_TYPE,
                )?;
            }
        }
        Ok(())
    })
}

/// A new item is made with its comment in one call. The Secret Service has
/// no single update of a secret and its attributes, so an existing item
/// takes the value first and the comment second: a reader between the two
/// sees the new value under the old comment, never the reverse, and
/// `KeyWatch` reads the comment again before calling that a mismatch.
pub fn secret_set_with_comment(
    service: &str,
    account: &str,
    value: &[u8],
    comment: &str,
) -> Result<()> {
    with_store(service, "write", Carries::Secret, |store| {
        let mut attrs = attributes(service, account);
        attrs.insert("comment", comment);
        match one(store, service, account)? {
            Some(item) => {
                item.set_secret(value, CONTENT_TYPE)?;
                item.set_attributes(attrs)?;
            }
            None => {
                store.get_default_collection()?.create_item(
                    &format!("{service}/{account}"),
                    attrs,
                    value,
                    false,
                    CONTENT_TYPE,
                )?;
            }
        }
        Ok(())
    })
}

/// Reads attributes only, never the secret.
pub fn secret_comment(service: &str, account: &str) -> Result<Option<String>> {
    with_store(service, "read", Carries::Attributes, |store| {
        Ok(match one(store, service, account)? {
            Some(item) => item.get_attributes()?.remove("comment"),
            None => None,
        })
    })
}

pub fn secret_delete(service: &str, account: &str) -> Result<bool> {
    with_store(service, "delete", Carries::Attributes, |store| {
        let items = search(store, attributes(service, account))?;
        for item in &items {
            item.delete()?;
        }
        Ok(!items.is_empty())
    })
}

/// Reads attributes only, never a secret.
pub fn secret_accounts(service: &str) -> Result<Vec<String>> {
    with_store(service, "search", Carries::Attributes, |store| {
        let mut accounts = Vec::new();
        for item in search(store, HashMap::from([("service", service)]))? {
            accounts.extend(item.get_attributes()?.remove("account"));
        }
        accounts.sort();
        accounts.dedup();
        Ok(accounts)
    })
}

/// `$XDG_CACHE_HOME/protonctl`, or `~/.cache/protonctl`. The XDG Base
/// Directory specification says to ignore a relative value.
pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".cache"))
        .join("protonctl")
}

/// No Proton Drive app is known for Linux, so Drive lists through the CLI.
pub fn cloud_storage() -> Option<PathBuf> {
    None
}

/// Without the app there are no File Provider placeholders.
pub fn cloud_only(_meta: &Metadata) -> bool {
    false
}

/// Run by `scripts/check.sh` in a private D-Bus session with a throwaway
/// GNOME Keyring, under their own service name.
#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = TEST_SERVICE;

    #[test]
    #[ignore = "needs a Secret Service; scripts/check.sh runs it in a private D-Bus session"]
    fn the_secret_service_keeps_items_values_and_comments() {
        for account in secret_accounts(S).unwrap() {
            secret_delete(S, &account).unwrap();
        }
        assert_eq!(secret_get(S, "a").unwrap(), None);
        assert_eq!(secret_comment(S, "a").unwrap(), None);
        assert!(!secret_delete(S, "a").unwrap());

        secret_set(S, "a", b"one").unwrap();
        secret_set(S, "a", b"two").unwrap();
        assert_eq!(secret_get(S, "a").unwrap().as_deref(), Some(&b"two"[..]));

        secret_set_with_comment(S, "k", b"key 1", "id 1").unwrap();
        assert_eq!(secret_comment(S, "k").unwrap().as_deref(), Some("id 1"));
        secret_set_with_comment(S, "k", b"key 2", "id 2").unwrap();
        // One item, updated in place: a second would make these refuse.
        assert_eq!(secret_comment(S, "k").unwrap().as_deref(), Some("id 2"));
        assert_eq!(secret_get(S, "k").unwrap().as_deref(), Some(&b"key 2"[..]));
        assert_eq!(secret_accounts(S).unwrap(), ["a", "k"]);

        assert!(secret_delete(S, "a").unwrap());
        assert_eq!(secret_get(S, "a").unwrap(), None);
        assert_eq!(secret_accounts(S).unwrap(), ["k"]);
        secret_delete(S, "k").unwrap();
    }

    /// Some callers read secrets on tokio's async threads, where zbus's own
    /// runtime would panic without a thread of its own.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs a Secret Service; scripts/check.sh runs it in a private D-Bus session"]
    async fn the_secret_service_answers_on_an_async_thread() {
        secret_set(S, "async", b"x").unwrap();
        assert_eq!(secret_get(S, "async").unwrap().as_deref(), Some(&b"x"[..]));
        assert!(secret_delete(S, "async").unwrap());
    }

    #[test]
    fn unit_tests_never_reach_the_user_s_items() {
        let err = secret_get("protonctl", "privacy-mode").unwrap_err();
        assert!(err.to_string().contains("never reach"), "{err}");
    }
}
