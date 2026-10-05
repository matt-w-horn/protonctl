//! macOS: the login Keychain, `~/Library`, and File Provider placeholders.

use std::fs::Metadata;
use std::os::macos::fs::MetadataExt as _;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use core_foundation::data::CFData;
use security_framework::item::{
    ItemAddOptions, ItemAddValue, ItemClass, ItemSearchOptions, ItemUpdateOptions, ItemUpdateValue,
    Limit, update_item,
};
use security_framework::passwords;

use crate::config::home;

pub const SECRET_STORE: &str = "Keychain";
pub const DOCUMENT_READERS: bool = true;
/// `errSecItemNotFound`.
const NOT_FOUND: i32 = -25300;
/// `st_flags` bit for a cloud-only (dataless) File Provider placeholder.
const SF_DATALESS: u32 = 0x4000_0000;

pub fn secret_get(service: &str, account: &str) -> Result<Option<Vec<u8>>> {
    match passwords::get_generic_password(service, account) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.code() == NOT_FOUND => Ok(None),
        Err(e) => Err(anyhow!("Keychain read of {service}/{account} failed: {e}")),
    }
}

pub fn secret_set(service: &str, account: &str, value: &[u8]) -> Result<()> {
    passwords::set_generic_password(service, account, value)
        .map_err(|e| anyhow!("Keychain write of {service}/{account} failed: {e}"))
}

/// An update when the item exists, so the value and comment change together;
/// otherwise an add.
pub fn secret_set_with_comment(
    service: &str,
    account: &str,
    value: &[u8],
    comment: &str,
) -> Result<()> {
    let mut search = ItemSearchOptions::new();
    search
        .class(ItemClass::generic_password())
        .service(service)
        .account(account);
    let mut update = ItemUpdateOptions::new();
    update
        .set_value(ItemUpdateValue::Data(CFData::from_buffer(value)))
        .set_comment(comment);
    match update_item(&search, &update) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == NOT_FOUND => {
            let mut add = ItemAddOptions::new(ItemAddValue::Data {
                class: ItemClass::generic_password(),
                data: CFData::from_buffer(value),
            });
            add.set_service(service)
                .set_account_name(account)
                .set_comment(comment);
            add.add()
                .map_err(|e| anyhow!("Keychain write of {service}/{account} failed: {e}"))
        }
        Err(e) => Err(anyhow!("Keychain write of {service}/{account} failed: {e}")),
    }
}

/// Reads attributes only, so no secret is loaded and macOS asks for no approval.
pub fn secret_comment(service: &str, account: &str) -> Result<Option<String>> {
    let found = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(service)
        .account(account)
        .load_attributes(true)
        .limit(Limit::Max(1))
        .search();
    match found {
        // "icmt" is kSecAttrComment.
        Ok(items) => Ok(items
            .first()
            .and_then(security_framework::item::SearchResult::simplify_dict)
            .and_then(|mut d| d.remove("icmt"))),
        Err(e) if e.code() == NOT_FOUND => Ok(None),
        Err(e) => Err(anyhow!("Keychain read of {service}/{account} failed: {e}")),
    }
}

pub fn secret_delete(service: &str, account: &str) -> Result<bool> {
    match passwords::delete_generic_password(service, account) {
        Ok(()) => Ok(true),
        Err(e) if e.code() == NOT_FOUND => Ok(false),
        Err(e) => Err(anyhow!(
            "Keychain delete of {service}/{account} failed: {e}"
        )),
    }
}

/// Reads attributes only, so no secret is loaded and macOS asks for no approval.
pub fn secret_accounts(service: &str) -> Result<Vec<String>> {
    let found = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(service)
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
        Err(e) => Err(anyhow!("Keychain search of {service} failed: {e}")),
    }
}

pub fn cache_dir() -> PathBuf {
    home().join("Library/Caches/protonctl")
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "Linux has no such place, and both systems share this signature"
)]
pub fn cloud_storage() -> Option<PathBuf> {
    Some(home().join("Library/CloudStorage"))
}

pub fn cloud_only(meta: &Metadata) -> bool {
    meta.st_flags() & SF_DATALESS != 0
}
