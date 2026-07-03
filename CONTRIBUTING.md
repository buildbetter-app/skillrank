# Contributing to SkillRank

Thanks for helping build an honest, measured skill registry.

## Development

Rust stable (edition 2021):

```sh
cargo build          # workspace: skillrank-core (lib) + skillrank (bin)
cargo test           # unit + integration tests
cargo fmt --all      # formatting (CI enforces --check)
cargo clippy --all-targets -- -D warnings
```

CI runs fmt, clippy (`-D warnings`), build, and test on every PR — please run
them locally first.

## Layout

- `crates/skillrank-core` — the library (registry client, install, eval runner).
  Keep it dependency-light and agent-agnostic: it is embedded by other tools
  (BuildBetter ZeroShot) as a crate.
- `crates/skillrank` — the CLI binary (commands, `serve`, `setup`, `mcp`).

## Adding a skill to the local seed catalog

The `serve` command ships a starter catalog at
`crates/skillrank/src/seed_catalog.json`. Add an entry (`slug`, `display_name`,
`category`, `stacks`, `summary`, `content` = the full SKILL.md). Content hashes
are computed at serve time, and `install` verifies against them.

## Guidelines

- Add a focused test with any behavior change (see the existing `#[cfg(test)]`
  modules — they cover parsing, install hash-verify, lockfile round-trips, the
  eval runner's arm construction and verifier isolation, and config merging).
- Prefer small, dependency-free additions. New crates need a clear justification.
- Keep the wire types in `skillrank-core::types` in sync with the registry
  contract; they are shared across the CLI, the local server, and ZeroShot.

## License

By contributing you agree that your contributions are licensed under the MIT
License (see [LICENSE](LICENSE)).
