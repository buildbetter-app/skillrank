//! Canonical content hashing for skills. Must match the Go implementation and the
//! server so `install` verifies correctly across languages.

use sha2::{Digest, Sha256};

pub const HASH_PREFIX: &str = "sha256:";

/// Raw SHA-256 of arbitrary bytes as lowercase hex, with no algorithm prefix and
/// no normalization. The one digest primitive in this workspace: skill content
/// hashing (below) and release-asset verification (`skillrank::update`) both go
/// through it, so there is a single implementation to audit rather than one per
/// caller.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Canonical content hash of a skill's SKILL.md bytes. Normalizes CRLF to LF and
/// strips trailing newlines so the same logical content hashes identically across
/// platforms and editors.
pub fn compute_content_hash(content: &str) -> String {
    let normalized = content.replace("\r\n", "\n");
    let normalized = normalized.trim_end_matches('\n');
    format!("{HASH_PREFIX}{}", sha256_hex(normalized.as_bytes()))
}

/// Compare two content hashes, tolerating a missing algorithm prefix on either side.
pub fn hashes_equal(a: &str, b: &str) -> bool {
    a.trim_start_matches(HASH_PREFIX)
        .eq_ignore_ascii_case(b.trim_start_matches(HASH_PREFIX))
}

/// Split "slug@version" into its parts.
pub fn split_ref(reference: &str) -> (String, String) {
    let reference = reference.trim();
    if let Some(idx) = reference.rfind('@') {
        if idx > 0 {
            return (
                reference[..idx].to_string(),
                reference[idx + 1..].to_string(),
            );
        }
    }
    (reference.to_string(), String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_digests() {
        // Known answers, so this stays a plain SHA-256 of the exact bytes: the
        // release-asset check compares against a digest produced by `shasum -a
        // 256`, and anything that normalizes (as `compute_content_hash` does)
        // would silently stop matching it.
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abc\n"),
            "edeaaff3f1774ad2888673770c6d64097e391bc362d7d6fb34982ddf0efd18cb"
        );
        assert_eq!(
            sha256_hex(&[]),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn content_hash_is_the_prefixed_digest_of_the_normalized_text() {
        assert_eq!(
            compute_content_hash("abc\n"),
            format!("{HASH_PREFIX}{}", sha256_hex(b"abc"))
        );
    }

    #[test]
    fn normalizes_line_endings_and_trailing_newline() {
        let a = compute_content_hash("---\nname: x\n---\nBody line\n");
        let b = compute_content_hash("---\r\nname: x\r\n---\r\nBody line");
        assert_eq!(a, b);
        assert_ne!(a, compute_content_hash("different"));
    }

    #[test]
    fn hashes_equal_tolerates_prefix() {
        let h = compute_content_hash("abc");
        let bare = h.trim_start_matches(HASH_PREFIX);
        assert!(hashes_equal(&h, bare));
        assert!(!hashes_equal(&h, &compute_content_hash("xyz")));
    }

    #[test]
    fn split_ref_works() {
        let (slug, version) = split_ref("owner/skill@sha256:deadbeef");
        assert_eq!(slug, "owner/skill");
        assert_eq!(version, "sha256:deadbeef");
        let (slug, version) = split_ref("just-a-slug");
        assert_eq!(slug, "just-a-slug");
        assert_eq!(version, "");
    }
}
