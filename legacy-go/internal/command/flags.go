package command

import "strings"

// Flags is a minimal parsed-args container: --key value / --key=value / --bool /
// -abc shorthand booleans, plus positionals.
type Flags struct {
	Values      map[string]string
	BoolValues  map[string]bool
	Positionals []string
}

// ParseFlags parses argv into Flags. `--json` implies `--format json`.
func ParseFlags(args []string) Flags {
	flags := Flags{Values: map[string]string{}, BoolValues: map[string]bool{}}
	for i := 0; i < len(args); i++ {
		arg := args[i]
		if arg == "-" || arg == "--" {
			flags.Positionals = append(flags.Positionals, arg)
			continue
		}
		if strings.HasPrefix(arg, "-") && !strings.HasPrefix(arg, "--") {
			for _, shorthand := range strings.TrimPrefix(arg, "-") {
				flags.BoolValues[string(shorthand)] = true
			}
			continue
		}
		if !strings.HasPrefix(arg, "--") {
			flags.Positionals = append(flags.Positionals, arg)
			continue
		}
		nameValue := strings.TrimPrefix(arg, "--")
		if nameValue == "" {
			continue
		}
		if strings.Contains(nameValue, "=") {
			parts := strings.SplitN(nameValue, "=", 2)
			flags.Values[parts[0]] = parts[1]
			flags.BoolValues[parts[0]] = true
			continue
		}
		if i+1 < len(args) && !strings.HasPrefix(args[i+1], "--") {
			flags.Values[nameValue] = args[i+1]
			flags.BoolValues[nameValue] = true
			i++
			continue
		}
		flags.BoolValues[nameValue] = true
	}
	if flags.BoolValues["json"] && flags.Values["format"] == "" {
		flags.Values["format"] = "json"
	}
	return flags
}

// OutputFormat returns "json" | "text".
func (flags Flags) OutputFormat() string {
	switch strings.ToLower(strings.TrimSpace(flags.Values["format"])) {
	case "json":
		return "json"
	default:
		return "text"
	}
}

// WantsStructuredOutput reports whether JSON output was requested.
func (flags Flags) WantsStructuredOutput() bool {
	return flags.OutputFormat() == "json"
}
