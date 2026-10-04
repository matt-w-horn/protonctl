//! SHA-256 and SHA-1 of saved content. SHA-256 is the digest to keep; SHA-1
//! is there because it is what Proton Drive stores for each file revision (the
//! uploader's claim, `claimedSha1`), so a copy on this Mac can be matched
//! against a file that is only in the cloud without downloading it.

use std::io::Read;
use std::path::Path;

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
}
