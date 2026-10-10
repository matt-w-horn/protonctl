//! The privacy key (RFC R20): 32 random bytes in the secret store, as
//! base64url text, with its key ID in the item's comment. HKDF-SHA-256 with
//! no salt gives one subkey per use, each under its own label.

use std::sync::{Arc, RwLock};

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use ring::hkdf;
use zeroize::Zeroizing;

use crate::secret::{self, Account, Secret};

const ALIAS: &[u8] = b"protonctl v1 alias";
const REFERENCE: &[u8] = b"protonctl v1 ref";
const HANDLE: &[u8] = b"protonctl v1 handle";
const DIGEST: &[u8] = b"protonctl v1 digest";
const KEY_ID: &[u8] = b"protonctl v1 key id";

/// Every subkey, zeroized on drop.
pub struct Keys {
    pub alias: Zeroizing<[u8; 32]>,
    pub reference: Zeroizing<[u8; 64]>,
    pub handle: Zeroizing<[u8; 64]>,
    pub digest: Zeroizing<[u8; 32]>,
    /// The key ID, in hex.
    pub id: String,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keys")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// An output length for ring's HKDF.
struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn expand<const N: usize>(prk: &hkdf::Prk, label: &[u8]) -> Zeroizing<[u8; N]> {
    let mut out = Zeroizing::new([0u8; N]);
    prk.expand(&[label], Len(N))
        .and_then(|okm| okm.fill(out.as_mut()))
        .expect("HKDF-SHA-256 gives up to 8,160 bytes, and these are at most 64");
    out
}

impl Keys {
    pub fn derive(key: &[u8; 32]) -> Self {
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &[]).extract(key);
        let id: Zeroizing<[u8; 16]> = expand(&prk, KEY_ID);
        Self {
            alias: expand(&prk, ALIAS),
            reference: expand(&prk, REFERENCE),
            handle: expand(&prk, HANDLE),
            digest: expand(&prk, DIGEST),
            id: hex::encode(id.as_ref()),
        }
    }
}

/// Where the key lives: the secret store, or a fake in tests.
pub trait KeySource: Send + Sync {
    /// The stored key's ID, read without loading the key.
    fn key_id(&self) -> Result<Option<String>>;
    fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>>;
}

/// The key in the secret store, as `setup privacy` and `rotate-key` write it.
pub struct StoredKey;

impl KeySource for StoredKey {
    fn key_id(&self) -> Result<Option<String>> {
        secret::comment(&Account::PrivacyKey)
    }

    fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>> {
        use secrecy::ExposeSecret as _;
        let Some(text) = secret::get(&Account::PrivacyKey)? else {
            return Ok(None);
        };
        decode(text.expose_secret()).map(Some)
    }
}

fn decode(text: &str) -> Result<Zeroizing<[u8; 32]>> {
    let bytes = Zeroizing::new(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(text.trim())
            .context("the privacy key is not base64url")?,
    );
    let mut key = Zeroizing::new([0u8; 32]);
    if bytes.len() != 32 {
        bail!("the privacy key is not 32 bytes");
    }
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// That the stored key can be read and matches its ID.
pub fn check_stored() -> Result<()> {
    KeyWatch::new(Box::new(StoredKey)).current().map(|_| ()).map_err(|e| {
        e.context("the privacy key is stored but cannot be used; let protonctl read it, or replace it with `protonctl rotate-key`")
    })
}

/// A new key from the system's random source, stored with its ID; returns
/// the ID.
pub fn create() -> Result<String> {
    use ring::rand::SecureRandom as _;
    let mut key = Zeroizing::new([0u8; 32]);
    ring::rand::SystemRandom::new()
        .fill(key.as_mut())
        .map_err(|e| anyhow::anyhow!("the system's random source failed: {e}"))?;
    let id = Keys::derive(&key).id;
    let text = Secret::from(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.as_ref()));
    secret::set_with_comment(&Account::PrivacyKey, &text, &id)?;
    Ok(id)
}

/// The keys a running server uses, loaded again when the stored key ID
/// changes (`rotate-key`), and kept only when the loaded key derives that ID,
/// so a server never holds an old key under a new ID (R20).
pub struct KeyWatch {
    source: Box<dyn KeySource>,
    current: RwLock<Option<Arc<Keys>>>,
}

impl KeyWatch {
    pub fn new(source: Box<dyn KeySource>) -> Self {
        Self {
            source,
            current: RwLock::new(None),
        }
    }

    /// The keys for one call, `None` when no key is stored, or an error when
    /// one is stored and cannot be used.
    pub fn current(&self) -> Result<Option<Arc<Keys>>> {
        let Some(id) = self.source.key_id()? else {
            return Ok(None);
        };
        if let Some(keys) = self.cached().filter(|k| k.id == id) {
            return Ok(Some(keys));
        }
        let Some(key) = self.source.load()? else {
            return Ok(None);
        };
        let keys = Arc::new(Keys::derive(&key));
        // The Keychain writes the key and its ID in one update. The Secret
        // Service writes the key first and the ID second, so a read between
        // them sees the new key under the old ID: read the ID once more.
        // Beyond that, they disagree only when the item was changed by hand
        // or damaged.
        anyhow::ensure!(
            keys.id == id || self.source.key_id()?.as_deref() == Some(keys.id.as_str()),
            "the privacy key does not match the key ID in its comment"
        );
        *self
            .current
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&keys));
        Ok(Some(keys))
    }

    fn cached(&self) -> Option<Arc<Keys>> {
        self.current
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    /// R20, T14 (#12): every subkey is zeroed when the shared keys are
    /// dropped. The `Arc`'s counts come first in its block, so the
    /// allocator's list does not overwrite a subkey.
    #[cfg(target_os = "linux")]
    #[test]
    fn subkeys_are_wiped_on_drop() {
        use crate::secret::tests::{after_drop, after_drop_sees_freed_memory};
        after_drop_sees_freed_memory();
        let keys = Arc::new(Keys::derive(&[7; 32]));
        let left = after_drop(keys, |k| {
            [
                (k.alias.as_ptr().addr(), k.alias.len()),
                (k.reference.as_ptr().addr(), k.reference.len()),
                (k.handle.as_ptr().addr(), k.handle.len()),
                (k.digest.as_ptr().addr(), k.digest.len()),
            ]
        });
        for bytes in left {
            assert!(bytes.iter().all(|&b| b == 0), "{bytes:?}");
        }
    }

    /// A key source a test can change.
    #[derive(Default)]
    pub struct FakeKey(pub Mutex<Option<[u8; 32]>>);

    impl KeySource for Arc<FakeKey> {
        fn key_id(&self) -> Result<Option<String>> {
            Ok(self.0.lock().unwrap().map(|k| Keys::derive(&k).id))
        }
        fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>> {
            Ok(self.0.lock().unwrap().map(Zeroizing::new))
        }
    }

    #[test]
    fn subkeys_differ_and_are_stable() {
        let a = Keys::derive(&[7; 32]);
        let b = Keys::derive(&[7; 32]);
        assert_eq!(a.id, b.id);
        assert_eq!(a.alias.as_ref(), b.alias.as_ref());
        assert_ne!(a.alias.as_ref(), a.digest.as_ref());
        assert_ne!(a.reference[..32], a.handle[..32]);
        assert_ne!(Keys::derive(&[8; 32]).id, a.id);
        assert_eq!(a.id.len(), 32);
        assert!(!format!("{a:?}").contains(&hex::encode(a.alias.as_ref())));
    }

    #[test]
    fn a_rotated_key_is_picked_up_on_the_next_call() {
        let source = Arc::new(FakeKey::default());
        let watch = KeyWatch::new(Box::new(Arc::clone(&source)));
        assert!(watch.current().unwrap().is_none());
        *source.0.lock().unwrap() = Some([1; 32]);
        let first = watch.current().unwrap().unwrap();
        *source.0.lock().unwrap() = Some([2; 32]);
        let second = watch.current().unwrap().unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(second.id, Keys::derive(&[2; 32]).id);
    }

    /// The Secret Service writes a rotated key before its ID, so a call
    /// that reads the old ID and then the new key reads the ID again.
    #[test]
    fn a_key_read_between_its_two_writes_is_used() {
        /// Reports the old key's ID first, then the new one's, as a read
        /// between the two writes and the read after them would.
        struct Midway(Mutex<Vec<String>>);
        impl KeySource for Midway {
            fn key_id(&self) -> Result<Option<String>> {
                Ok(self.0.lock().unwrap().pop())
            }
            fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>> {
                Ok(Some(Zeroizing::new([2; 32])))
            }
        }
        let ids = vec![Keys::derive(&[2; 32]).id, Keys::derive(&[1; 32]).id];
        let keys = KeyWatch::new(Box::new(Midway(Mutex::new(ids))))
            .current()
            .unwrap()
            .unwrap();
        assert_eq!(keys.id, Keys::derive(&[2; 32]).id);
    }

    /// A stored key that cannot be read, or that does not match its ID, is
    /// an error rather than "no key", so it is not mistaken for one to make.
    #[test]
    fn a_stored_key_that_cannot_be_used_is_an_error() {
        struct Broken(&'static str);
        impl KeySource for Broken {
            fn key_id(&self) -> Result<Option<String>> {
                Ok(Some(self.0.to_string()))
            }
            fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>> {
                anyhow::ensure!(self.0 != "unreadable", "the store refused");
                Ok(Some(Zeroizing::new([1; 32])))
            }
        }
        for broken in ["unreadable", "another key's id"] {
            assert!(
                KeyWatch::new(Box::new(Broken(broken))).current().is_err(),
                "{broken}"
            );
        }
    }

    #[test]
    fn stored_text_decodes_only_as_32_bytes() {
        let text = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([9u8; 32]);
        assert_eq!(*decode(&text).unwrap(), [9u8; 32]);
        assert!(decode("c2hvcnQ").is_err());
        assert!(decode("not base64!").is_err());
    }
}
