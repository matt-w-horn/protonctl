//! Identifiers (RFC section 6, API specification "Wire formats"): aliases
//! and keyed digests are one-way, as words; refs, handles and sealed page
//! tokens are AES-SIV, which protonctl alone can open. All are stable for
//! one key, with no state.

use aes_siv::KeyInit as _;
use aes_siv::siv::Aes256Siv;
use base64::Engine as _;
use ring::hmac;

use super::key::Keys;
use super::words;
use crate::tool::Tool;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// An entity's type, with its one-byte tag for refs; it serializes as its
/// name in `entities` tables.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, strum::IntoStaticStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EntityType {
    Person,
    Organization,
    Location,
    Address,
    Email,
    Phone,
    Card,
    Iban,
    Domain,
    Ip,
    Secret,
    NationalId,
    Account,
}

impl EntityType {
    const ALL: [Self; 13] = [
        Self::Person,
        Self::Organization,
        Self::Location,
        Self::Address,
        Self::Email,
        Self::Phone,
        Self::Card,
        Self::Iban,
        Self::Domain,
        Self::Ip,
        Self::Secret,
        Self::NationalId,
        Self::Account,
    ];

    pub fn tag(self) -> u8 {
        match self {
            Self::Person => 0x01,
            Self::Organization => 0x02,
            Self::Location => 0x03,
            Self::Address => 0x04,
            Self::Email => 0x05,
            Self::Phone => 0x06,
            Self::Card => 0x07,
            Self::Iban => 0x08,
            Self::Domain => 0x0A,
            Self::Ip => 0x0B,
            Self::Secret => 0x0C,
            Self::NationalId => 0x0D,
            Self::Account => 0x0E,
        }
    }

    pub fn from_tag(tag: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.tag() == tag)
    }

    /// The alias's class: one for every kind of name, so a detector that
    /// retypes a name keeps its alias (RFC Q19).
    pub fn class(self) -> AliasClass {
        match self {
            Self::Person | Self::Organization | Self::Location => AliasClass::Name,
            other => AliasClass::Of(other),
        }
    }
}

/// What an alias is computed over beside the value: one class for names,
/// and each other type its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AliasClass {
    Name,
    Of(EntityType),
}

impl AliasClass {
    /// The class's label in the alias HMAC's input, part of the alias
    /// format (`v1`): "name", or the type's name.
    fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Of(t) => t.into(),
        }
    }
}

/// A digest's algorithm. Its name is part of the keyed digest's input, so
/// the same bytes under two algorithms never meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
pub enum Algorithm {
    Sha256,
    Sha1,
}

/// What a handle names, with its one-byte kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Message,
    Thread,
    Event,
    DrivePath,
}

impl ItemKind {
    fn tag(self) -> u8 {
        match self {
            Self::Message => 0x01,
            Self::Thread => 0x02,
            Self::Event => 0x03,
            Self::DrivePath => 0x04,
        }
    }

    /// A message ID has a fixed 16 characters; the rest vary, so they are
    /// padded, lest a handle's length give the value's.
    fn padded(self) -> bool {
        self != Self::Message
    }
}

/// What a sealed page token continues, with its one-byte kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    MailCursor,
    Offset,
    Tree,
    CalendarWindow,
    ThreadPage,
}

impl TokenKind {
    /// What `tool`'s `nextPageToken` continues: the pipeline seals it as
    /// this kind, and the tool opens it as this kind.
    pub fn of(tool: Tool) -> Self {
        match tool {
            Tool::SearchThreads => Self::MailCursor,
            Tool::GetThread => Self::ThreadPage,
            Tool::ListEvents | Tool::SearchEvents => Self::CalendarWindow,
            Tool::ListDriveTree => Self::Tree,
            _ => Self::Offset,
        }
    }

    fn tag(self) -> u8 {
        match self {
            Self::MailCursor => 0x11,
            Self::Offset => 0x12,
            Self::Tree => 0x13,
            Self::CalendarWindow => 0x14,
            Self::ThreadPage => 0x15,
        }
    }

    /// A tree token carries a Drive path.
    fn padded(self) -> bool {
        self == Self::Tree
    }
}

/// A value that did not open: tampered, from another key, or another kind.
/// Why does not matter, and never leaves.
#[derive(Debug, PartialEq, Eq)]
pub struct Unopened;

impl From<base64::DecodeError> for Unopened {
    fn from(_: base64::DecodeError) -> Self {
        Self
    }
}

impl From<aes_siv::Error> for Unopened {
    fn from(_: aes_siv::Error) -> Self {
        Self
    }
}

impl From<std::string::FromUtf8Error> for Unopened {
    fn from(_: std::string::FromUtf8Error) -> Self {
        Self
    }
}

fn pad(mut v: Vec<u8>) -> Vec<u8> {
    v.push(0x80);
    while !v.len().is_multiple_of(32) {
        v.push(0);
    }
    v
}

fn unpad(mut v: Vec<u8>) -> Result<Vec<u8>, Unopened> {
    while v.last() == Some(&0) {
        v.pop();
    }
    if v.pop() == Some(0x80) {
        Ok(v)
    } else {
        Err(Unopened)
    }
}

fn seal(key: &[u8; 64], ad: &[u8], plaintext: &[u8]) -> String {
    let mut siv = Aes256Siv::new_from_slice(key).expect("an AES-256-SIV key is 64 bytes");
    let sealed = siv
        .encrypt([ad], plaintext)
        .expect("AES-SIV encrypts any length that fits in memory");
    B64.encode(sealed)
}

fn open(key: &[u8; 64], ad: &[u8], text: &str) -> Result<Vec<u8>, Unopened> {
    let bytes = B64.decode(text)?;
    let mut siv = Aes256Siv::new_from_slice(key).expect("an AES-256-SIV key is 64 bytes");
    Ok(siv.decrypt([ad], &bytes)?)
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> hmac::Tag {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    let mut ctx = hmac::Context::with_key(&key);
    for p in parts {
        ctx.update(p);
    }
    ctx.sign()
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(bytes[at..at + 8].try_into().expect("8 bytes"))
}

impl Keys {
    /// The alias of a canonical value: three words, then `extra` more for a
    /// collision inside one result.
    pub fn alias(&self, t: EntityType, canonical: &str, extra: usize) -> String {
        let input = [
            b"v1".as_slice(),
            &[0],
            t.class().label().as_bytes(),
            &[0],
            canonical.as_bytes(),
        ];
        let first = hmac(self.alias.as_ref(), &input);
        let n = words::count();
        let mut out = words::words(u64_at(first.as_ref(), 0) % (n * n * n), 3);
        // Further words from the next 8-byte blocks, then from HMACs over the
        // input and a counter.
        let mut stream: Vec<u8> = first.as_ref()[8..].to_vec();
        let mut counter = 0u8;
        for _ in 0..extra {
            if stream.len() < 8 {
                counter += 1;
                let mut more = input.to_vec();
                let tail = [0, counter];
                more.push(&tail);
                stream.extend_from_slice(hmac(self.alias.as_ref(), &more).as_ref());
            }
            out.push(words::word(u64_at(&stream, 0) % n));
            stream.drain(..8);
        }
        out.join("-")
    }

    pub fn reference(&self, t: EntityType, canonical: &str) -> String {
        let mut plain = vec![t.tag()];
        plain.extend_from_slice(canonical.as_bytes());
        seal(&self.reference, b"protonctl ref v1", &pad(plain))
    }

    pub fn open_ref(&self, text: &str) -> Result<(EntityType, String), Unopened> {
        let plain = unpad(open(&self.reference, b"protonctl ref v1", text)?)?;
        let (&tag, value) = plain.split_first().ok_or(Unopened)?;
        let t = EntityType::from_tag(tag).ok_or(Unopened)?;
        Ok((t, String::from_utf8(value.to_vec())?))
    }

    pub fn handle(&self, kind: ItemKind, id: &str) -> String {
        let mut plain = vec![kind.tag()];
        plain.extend_from_slice(id.as_bytes());
        let plain = if kind.padded() { pad(plain) } else { plain };
        seal(&self.handle, b"protonctl handle v1", &plain)
    }

    pub fn open_handle(&self, kind: ItemKind, text: &str) -> Result<String, Unopened> {
        let plain = open(&self.handle, b"protonctl handle v1", text)?;
        let plain = if kind.padded() { unpad(plain)? } else { plain };
        match plain.split_first() {
            Some((&tag, id)) if tag == kind.tag() => Ok(String::from_utf8(id.to_vec())?),
            _ => Err(Unopened),
        }
    }

    pub fn seal_token(&self, kind: TokenKind, token: &str) -> String {
        let mut plain = vec![kind.tag()];
        plain.extend_from_slice(token.as_bytes());
        let plain = if kind.padded() { pad(plain) } else { plain };
        seal(&self.handle, b"protonctl token v1", &plain)
    }

    pub fn open_token(&self, kind: TokenKind, text: &str) -> Result<String, Unopened> {
        let plain = open(&self.handle, b"protonctl token v1", text)?;
        let plain = if kind.padded() { unpad(plain)? } else { plain };
        match plain.split_first() {
            Some((&tag, t)) if tag == kind.tag() => Ok(String::from_utf8(t.to_vec())?),
            _ => Err(Unopened),
        }
    }

    /// A digest under the digest key, as five words (64 bits), so equal files
    /// still match while the raw digest, which links to copies elsewhere,
    /// never leaves (R17).
    pub fn keyed_digest(&self, algorithm: Algorithm, raw: &[u8]) -> String {
        let algorithm: &str = algorithm.into();
        let h = hmac(self.digest.as_ref(), &[algorithm.as_bytes(), &[0], raw]);
        words::words(u64_at(h.as_ref(), 0), 5).join("-")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(n: u8) -> Keys {
        Keys::derive(&[n; 32])
    }

    /// The labels are part of the alias format: changing one changes every
    /// alias of its type.
    #[test]
    fn class_labels_are_the_format_s() {
        let labels: Vec<&str> = EntityType::ALL.iter().map(|t| t.class().label()).collect();
        assert_eq!(
            labels,
            [
                "name",
                "name",
                "name",
                "address",
                "email",
                "phone",
                "card",
                "iban",
                "domain",
                "ip",
                "secret",
                "national_id",
                "account"
            ]
        );
    }

    #[test]
    fn aliases_are_stable_words_and_names_share_a_class() {
        let k = keys(1);
        let a = k.alias(EntityType::Person, "alice chen", 0);
        assert_eq!(a, k.alias(EntityType::Person, "alice chen", 0));
        assert_eq!(a.split('-').count(), 3);
        assert_eq!(
            a,
            k.alias(EntityType::Organization, "alice chen", 0),
            "names share a tag"
        );
        assert_ne!(a, k.alias(EntityType::Email, "alice chen", 0));
        assert_ne!(a, keys(2).alias(EntityType::Person, "alice chen", 0));
        let longer = k.alias(EntityType::Person, "alice chen", 5);
        assert!(longer.starts_with(&a));
        assert_eq!(longer.split('-').count(), 8);
    }

    #[test]
    fn refs_handles_and_tokens_round_trip_and_refuse_tampering() {
        let (k, other) = (keys(1), keys(2));
        let reference = k.reference(EntityType::Email, "a@b.test");
        assert_eq!(reference.len(), 64);
        assert_eq!(
            k.open_ref(&reference).unwrap(),
            (EntityType::Email, "a@b.test".into())
        );
        assert_eq!(
            reference,
            k.reference(EntityType::Email, "a@b.test"),
            "deterministic"
        );
        assert!(other.open_ref(&reference).is_err());
        let mut bad = reference.clone().into_bytes();
        bad[10] = if bad[10] == b'A' { b'B' } else { b'A' };
        assert!(k.open_ref(&String::from_utf8(bad).unwrap()).is_err());

        let message = k.handle(ItemKind::Message, "Qm3vT8pLx2NaK7cD");
        assert_eq!(message.len(), 44);
        assert_eq!(
            k.open_handle(ItemKind::Message, &message).unwrap(),
            "Qm3vT8pLx2NaK7cD"
        );
        assert!(
            k.open_handle(ItemKind::Thread, &message).is_err(),
            "another kind"
        );
        let path = k.handle(ItemKind::DrivePath, "/a/b");
        assert_eq!(
            path.len(),
            k.handle(ItemKind::DrivePath, "/a/bcdefgh").len(),
            "padded"
        );
        assert_eq!(k.open_handle(ItemKind::DrivePath, &path).unwrap(), "/a/b");

        let token = k.seal_token(TokenKind::Offset, "40");
        assert_eq!(k.open_token(TokenKind::Offset, &token).unwrap(), "40");
        assert!(k.open_token(TokenKind::MailCursor, &token).is_err());
        assert!(k.open_token(TokenKind::Offset, "40").is_err());
    }

    #[test]
    fn padding_keeps_trailing_zeros() {
        let v = b"ends\0\0".to_vec();
        assert_eq!(unpad(pad(v.clone())).unwrap(), v);
        assert_eq!(pad(v).len() % 32, 0);
        assert!(unpad(vec![1, 0, 0]).is_err());
    }

    proptest::proptest! {
        /// R15, R16: any value's ref, and any ID's handle, opens to that
        /// value under its key, and is refused with one byte changed, under
        /// another key, or as another kind of handle.
        #[test]
        fn any_ref_or_handle_opens_to_its_value_and_nothing_else(
            value in "\\PC{1,80}",
            at in proptest::prelude::any::<proptest::sample::Index>(),
        ) {
            let (k, other) = (keys(1), keys(2));
            let changed = |s: &str| {
                let mut b = s.as_bytes().to_vec();
                let i = at.index(b.len());
                b[i] = if b[i] == b'A' { b'B' } else { b'A' };
                String::from_utf8(b).unwrap()
            };
            let reference = k.reference(EntityType::Person, &value);
            proptest::prop_assert_eq!(k.open_ref(&reference), Ok((EntityType::Person, value.clone())));
            proptest::prop_assert!(other.open_ref(&reference).is_err());
            proptest::prop_assert!(k.open_ref(&changed(&reference)).is_err());
            for kind in [ItemKind::Message, ItemKind::Thread, ItemKind::Event, ItemKind::DrivePath] {
                let handle = k.handle(kind, &value);
                proptest::prop_assert_eq!(k.open_handle(kind, &handle), Ok(value.clone()));
                proptest::prop_assert!(other.open_handle(kind, &handle).is_err());
                proptest::prop_assert!(k.open_handle(kind, &changed(&handle)).is_err());
                let another = if kind == ItemKind::Message { ItemKind::Thread } else { ItemKind::Message };
                proptest::prop_assert!(k.open_handle(another, &handle).is_err());
            }
        }

        /// Text that no key sealed opens as nothing.
        #[test]
        fn garbage_opens_as_nothing(text in "[A-Za-z0-9_-]{0,90}") {
            let k = keys(1);
            proptest::prop_assert!(k.open_ref(&text).is_err());
            proptest::prop_assert!(k.open_handle(ItemKind::DrivePath, &text).is_err());
        }
    }

    #[test]
    fn digests_are_keyed() {
        let k = keys(1);
        let d = k.keyed_digest(Algorithm::Sha256, &[0xAB; 32]);
        // The names are part of the format.
        assert_eq!(<&str>::from(Algorithm::Sha256), "sha256");
        assert_eq!(<&str>::from(Algorithm::Sha1), "sha1");
        assert_eq!(d.split('-').count(), 5);
        assert_ne!(d, keys(2).keyed_digest(Algorithm::Sha256, &[0xAB; 32]));
        assert_ne!(d, k.keyed_digest(Algorithm::Sha1, &[0xAB; 32]));
    }
}
