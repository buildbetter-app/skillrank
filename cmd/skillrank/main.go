// Command skillrank is the standalone, open-source SkillRank CLI: find, install,
// evaluate, rate, review, and publish agent skills. It works entirely on its own
// (the core — search, install, local eval — needs no account) and integrates
// seamlessly with BuildBetter ZeroShot when that is also installed.
package main

import (
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"os"

	"github.com/buildbetter/skillrank/internal/api"
	"github.com/buildbetter/skillrank/internal/command"
	skillscmd "github.com/buildbetter/skillrank/internal/skillscmd"
)

var version = "dev"

func main() {
	os.Exit(run(os.Args[1:], os.Stdin, os.Stdout, os.Stderr))
}

func run(args []string, stdin io.Reader, stdout io.Writer, stderr io.Writer) int {
	if len(args) > 0 && args[0] == "--" {
		args = args[1:]
	}
	ctx := command.Context{
		Stdin:      stdin,
		Stdout:     stdout,
		Stderr:     stderr,
		Version:    version,
		HTTPClient: http.DefaultClient,
		Logger:     slog.New(slog.NewTextHandler(stderr, &slog.HandlerOptions{Level: slog.LevelWarn})),
	}

	if len(args) == 0 {
		return skillscmd.Run(nil, ctx)
	}
	switch args[0] {
	case "--version", "-V", "version":
		fmt.Fprintf(stdout, "skillrank %s\n", version)
		return 0
	case "--help", "-h", "help":
		return skillscmd.Run([]string{"help"}, ctx)
	case "login":
		return runLogin(args[1:], ctx)
	case "logout":
		return runLogout(ctx)
	case "whoami":
		return runWhoami(ctx)
	default:
		return skillscmd.Run(args, ctx)
	}
}

// runLogin stores a registry token so publish/rate/review can authenticate. The
// core CLI (search/install/eval) never needs this. A full browser/device flow is
// a follow-up; for now a token (from the registry web UI) is accepted.
func runLogin(args []string, ctx command.Context) int {
	flags := command.ParseFlags(args)
	token := flags.Values["token"]
	if token == "" {
		token = os.Getenv("SKILLRANK_TOKEN")
	}
	if token == "" {
		fmt.Fprintln(ctx.Stdout, "Publishing and reviewing require a registry token.")
		fmt.Fprintln(ctx.Stdout, "Get one from your registry account, then run:")
		fmt.Fprintln(ctx.Stdout, "  skillrank login --token <token>")
		fmt.Fprintln(ctx.Stdout, "\n(Search, install, and local eval need no account.)")
		return 1
	}
	if err := api.SaveToken(token); err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	fmt.Fprintln(ctx.Stdout, "Saved. You can now publish and review skills.")
	return 0
}

func runLogout(ctx command.Context) int {
	if err := api.SaveToken(""); err != nil {
		fmt.Fprintf(ctx.Stderr, "error: %s\n", err)
		return 1
	}
	fmt.Fprintln(ctx.Stdout, "Signed out.")
	return 0
}

func runWhoami(ctx command.Context) int {
	if os.Getenv("SKILLRANK_TOKEN") != "" {
		fmt.Fprintln(ctx.Stdout, "Authenticated via SKILLRANK_TOKEN.")
		return 0
	}
	fmt.Fprintln(ctx.Stdout, "Not signed in (reads and local eval still work).")
	return 0
}
