//! Canonical content hashing for skills. Must match the Go implementation and the
//! server so `install` verifies correctly across languages.

use sha2::{Digest, Sha256};

pub const HASH_PREFIX: &str = "sha256:";

/// Canonical content hash of a skill's SKILL.md bytes. Normalizes CRLF to LF and
/// strips trailing newlines so the same logical content hashes identically across
/// platforms and editors.
pub fn compute_content_hash(content: &str) -> String {
    let normalized = content.replace("\r\n", "\n");
    let normalized = normalized.trim_end_matches('\n');
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    format!("{HASH_PREFIX}{:x}", digest)
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
