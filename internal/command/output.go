package command

import (
	"encoding/json"
	"io"
)

// WriteJSON encodes value as indented JSON (HTML escaping off).
func WriteJSON(stdout io.Writer, value any) {
	encoder := json.NewEncoder(stdout)
	encoder.SetEscapeHTML(false)
	encoder.SetIndent("", "  ")
	_ = encoder.Encode(value)
}

// WriteOutput writes value as JSON (the only structured format the standalone CLI
// ships). Text output is handled by each command's own human-readable branch.
func WriteOutput(stdout io.Writer, flags Flags, value any) {
	WriteJSON(stdout, value)
}
