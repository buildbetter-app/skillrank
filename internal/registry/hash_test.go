package registry

import "testing"

func TestComputeContentHashNormalizesLineEndingsAndTrailingNewline(t *testing.T) {
	a := ComputeContentHash("---\nname: x\n---\nBody line\n")
	b := ComputeContentHash("---\r\nname: x\r\n---\r\nBody line")
	if a != b {
		t.Fatalf("expected CRLF and trailing-newline normalization to hash equal:\n%s\n%s", a, b)
	}
	if a == ComputeContentHash("different") {
		t.Fatal("different content must not collide")
	}
}

func TestHashesEqualToleratesPrefix(t *testing.T) {
	h := ComputeContentHash("abc")
	bare := h[len(HashPrefix):]
	if !HashesEqual(h, bare) {
		t.Fatal("expected prefixed and bare hashes to compare equal")
	}
	if HashesEqual(h, ComputeContentHash("xyz")) {
		t.Fatal("distinct content must not compare equal")
	}
}

func TestSplitRef(t *testing.T) {
	slug, version := SplitRef("owner/skill@sha256:deadbeef")
	if slug != "owner/skill" || version != "sha256:deadbeef" {
		t.Fatalf("unexpected split: %q %q", slug, version)
	}
	slug, version = SplitRef("just-a-slug")
	if slug != "just-a-slug" || version != "" {
		t.Fatalf("unexpected split without version: %q %q", slug, version)
	}
}
