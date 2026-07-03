// Package command is the standalone CLI harness for skillrank: command context,
// flag parsing, and output formatting. It has zero external dependencies so the
// skillrank binary stays small and portable.
package command

import (
	"io"
	"log/slog"
	"net/http"
)

// Context is passed to every command handler.
type Context struct {
	Stdin  io.Reader
	Stdout io.Writer
	Stderr io.Writer

	Version    string
	HTTPClient *http.Client
	Logger     *slog.Logger
}
