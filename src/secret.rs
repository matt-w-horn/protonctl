//! The macOS Keychain is the only place protonctl keeps secrets (RFC R2).
//! Items live under service `protonctl`; accounts are `calendar/<id>` and,
//! after Phase 0, `bridge/<address>`.

use anyhow::{Result, anyhow};
use secrecy::{ExposeSecret as _, SecretString};
use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
use security_framework::passwords;

/// Every secret protonctl holds in memory: `Debug` shows it redacted, and
/// `expose_secret()` marks each place that reads it.
pub type Secret = SecretString;

const SERVICE: &str = "protonctl";
/// `errSecItemNotFound`.
const NOT_FOUND: i32 = -25300;

pub fn get(account: &str) -> Result<Option<Secret>> {
    match passwords::get_generic_password(SERVICE, account) {
        #[expect(
            clippy::map_err_ignore,
            reason = "the UTF-8 error owns the secret's bytes, and {:#?} would print them"
        )]
        Ok(bytes) => String::from_utf8(bytes)
            .map(|s| Some(Secret::from(s)))
            .map_err(|_| anyhow!("Keychain item {SERVICE}/{account} is not UTF-8")),
        Err(e) if e.code() == NOT_FOUND => Ok(None),
        Err(e) => Err(anyhow!("Keychain read of {SERVICE}/{account} failed: {e}")),
    }
}

pub fn set(account: &str, value: &Secret) -> Result<()> {
    passwords::set_generic_password(SERVICE, account, value.expose_secret().as_bytes())
        .map_err(|e| anyhow!("Keychain write of {SERVICE}/{account} failed: {e}"))
}

/// Returns false when there was nothing to delete.
pub fn delete(account: &str) -> Result<bool> {
    match passwords::delete_generic_password(SERVICE, account) {
        Ok(()) => Ok(true),
        Err(e) if e.code() == NOT_FOUND => Ok(false),
        Err(e) => Err(anyhow!(
            "Keychain delete of {SERVICE}/{account} failed: {e}"
        )),
    }
}

/// Every account protonctl holds under its service, including any whose
/// calendar has left the config. Reads attributes only, so no secret is loaded
/// and macOS asks for no approval.
pub fn accounts() -> Result<Vec<String>> {
    let found = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(SERVICE)
        .load_attributes(true)
        .limit(Limit::All)
        .search();
    match found {
        Ok(items) => {
            // "acct" is kSecAttrAccount.
            let mut accounts: Vec<String> = items
                .iter()
                .filter_map(|i| i.simplify_dict()?.remove("acct"))
                .collect();
            accounts.sort();
            Ok(accounts)
        }
        Err(e) if e.code() == NOT_FOUND => Ok(Vec::new()),
        Err(e) => Err(anyhow!("Keychain search of {SERVICE} failed: {e}")),
    }
}

pub fn calendar_account(id: &str) -> String {
    format!("calendar/{id}")
}

pub fn bridge_account(address: &str) -> String {
    format!("bridge/{address}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_redacted_in_debug_output() {
        let secret = Secret::from("hunter2-not-a-real-secret");
        let shown = format!("{secret:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
    }
}
