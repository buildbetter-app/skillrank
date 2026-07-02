package registry

import (
	"crypto/sha256"
	"encoding/hex"
	"strings"
)

// HashPrefix namespaces content hashes so the algorithm is explicit on the wire.
const HashPrefix = "sha256:"

// ComputeContentHash returns the canonical content hash of a skill's SKILL.md
// bytes. Canonicalization normalizes CRLF to LF and strips a trailing newline so
// the same logical content hashes identically across platforms and editors.
func ComputeContentHash(content string) string {
	normalized := strings.ReplaceAll(content, "\r\n", "\n")
	normalized = strings.TrimRight(normalized, "\n")
	sum := sha256.Sum256([]byte(normalized))
	return HashPrefix + hex.EncodeToString(sum[:])
}

// HashesEqual compares two content hashes tolerating a missing algorithm prefix
// on either side.
func HashesEqual(a, b string) bool {
	return strings.EqualFold(strings.TrimPrefix(a, HashPrefix), strings.TrimPrefix(b, HashPrefix))
}
