//! SHA-256 and SHA-1 of saved content. SHA-256 is the digest to keep; SHA-1
//! is there because it is what Proton Drive stores for each file revision (the
//! uploader's claim, `claimedSha1`), so a copy on this Mac can be matched
//! against a file that is only in the cloud without downloading it.

use std::io::{Read, Write};
use std::path::Path;

use anyhow::Context as _;
use ring::digest::{Context, SHA1_FOR_LEGACY_USE_ONLY, SHA256};
use serde_json::{Value, json};

/// Read size for files.
const CHUNK: usize = 64 * 1024;

/// Both digests of one stream, fed in pieces.
struct Hasher {
    sha256: Context,
    sha1: Context,
}

/// Lower-case hex digests.
#[derive(Debug, PartialEq, Eq)]
pub struct Sums {
    pub sha256: String,
    pub sha1: String,
}

impl Hasher {
    fn new() -> Self {
        Self {
            sha256: Context::new(&SHA256),
            sha1: Context::new(&SHA1_FOR_LEGACY_USE_ONLY),
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.sha256.update(data);
        self.sha1.update(data);
    }

    fn finish(self) -> Sums {
        Sums {
            sha256: hex::encode(self.sha256.finish()),
            sha1: hex::encode(self.sha1.finish()),
        }
    }
}

impl Sums {
    /// Add `sha256` and `sha1` to `v`, an object, and `matchesClaimedSha1`
    /// when a claimed SHA-1 is known.
    pub fn add(&self, v: &mut Value, claimed_sha1: Option<&str>) {
        v["sha256"] = json!(self.sha256);
        v["sha1"] = json!(self.sha1);
        if let Some(claim) = claimed_sha1 {
            v["matchesClaimedSha1"] = json!(claim.eq_ignore_ascii_case(&self.sha1));
        }
    }
}

/// A pinned SHA-256 (R9): Bridge's certificate, or the Drive CLI on Linux
/// (Q33). The config holds it as 64 hex digits, parsed when it loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sha256(pub [u8; 32]);

impl Sha256 {
    pub fn of(data: &[u8]) -> Self {
        let mut out = [0; 32];
        out.copy_from_slice(ring::digest::digest(&SHA256, data).as_ref());
        Self(out)
    }

    /// The SHA-256 of a file, read to its end. It blocks, so callers run it
    /// off the async threads.
    pub fn of_file(path: &Path) -> anyhow::Result<Self> {
        let mut f =
            std::fs::File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
        Self::of_copy(&mut f, &mut std::io::sink())
            .with_context(|| format!("cannot read {}", path.display()))
    }

    /// Copy `from` to its end into `to`, and return the SHA-256 of the
    /// bytes copied: one read, so what is hashed is what is written.
    pub fn of_copy(from: &mut impl Read, to: &mut impl Write) -> std::io::Result<Self> {
        let mut h = Context::new(&SHA256);
        let mut buf = vec![0; CHUNK];
        loop {
            let n = from.read(&mut buf)?;
            if n == 0 {
                let mut out = [0; 32];
                out.copy_from_slice(h.finish().as_ref());
                return Ok(Self(out));
            }
            h.update(&buf[..n]);
            to.write_all(&buf[..n])?;
        }
    }
}

impl std::str::FromStr for Sha256 {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        let mut out = [0; 32];
        hex::decode_to_slice(s, &mut out).context("a SHA-256 is 64 hex digits")?;
        Ok(Self(out))
    }
}

impl std::fmt::Display for Sha256 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl<'de> serde::Deserialize<'de> for Sha256 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl serde::Serialize for Sha256 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

pub fn bytes(data: &[u8]) -> Sums {
    let mut h = Hasher::new();
    h.update(data);
    h.finish()
}

/// The digests of a file, read to its end. It blocks, so callers run it off
/// the async threads.
pub fn file(path: &Path) -> std::io::Result<Sums> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Hasher::new();
    let mut buf = vec![0; CHUNK];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(h.finish());
        }
        h.update(&buf[..n]);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const HELLO_SHA256: &str =
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    pub(crate) const HELLO_SHA1: &str = "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d";

    #[test]
    fn digests_match_known_values_and_files_match_their_bytes() {
        assert_eq!(
            bytes(b"hello"),
            Sums {
                sha256: HELLO_SHA256.into(),
                sha1: HELLO_SHA1.into()
            }
        );
        // Across several chunks, with a short one at the end.
        let data: Vec<u8> = (0..3 * CHUNK + 17)
            .map(|i| u8::try_from(i % 251).unwrap())
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data");
        std::fs::write(&path, &data).unwrap();
        assert_eq!(file(&path).unwrap(), bytes(&data));
        // The Drive CLI's copy (Q24): every byte written, and hashed once.
        let mut copy = Vec::new();
        let sha = Sha256::of_copy(&mut data.as_slice(), &mut copy).unwrap();
        assert_eq!((sha, copy), (Sha256::of(&data), data));
        let s = bytes(b"hello");
        let added = |claim: Option<&str>| {
            let mut v = json!({});
            s.add(&mut v, claim);
            v
        };
        assert_eq!(
            added(Some(&HELLO_SHA1.to_uppercase()))["matchesClaimedSha1"],
            true
        );
        assert_eq!(added(Some(HELLO_SHA256))["matchesClaimedSha1"], false);
        assert_eq!(
            added(None),
            json!({ "sha256": HELLO_SHA256, "sha1": HELLO_SHA1 })
        );
    }

    #[test]
    fn a_pinned_sha256_reads_only_as_64_hex_digits() {
        let hello = Sha256::of(b"hello");
        assert_eq!(hello.to_string(), HELLO_SHA256);
        assert_eq!(HELLO_SHA256.parse::<Sha256>().unwrap(), hello);
        assert_eq!(
            HELLO_SHA256.to_uppercase().parse::<Sha256>().unwrap(),
            hello
        );
        for bad in [
            "abc",
            &"zz".repeat(32),
            &format!("+{}", "0".repeat(63)),
            &format!("{HELLO_SHA256}00"),
        ] {
            assert!(bad.parse::<Sha256>().is_err(), "{bad}");
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cli");
        std::fs::write(&path, b"hello").unwrap();
        assert_eq!(Sha256::of_file(&path).unwrap(), hello);
    }
}
