#!/usr/bin/env sh
# SkillRank installer — installs the standalone `skillrank` CLI, then offers to
# also install BuildBetter ZeroShot (optional; SkillRank works fully without it).
#
#   curl -fsSL skillrank.dev | sh
#
# Env:
#   SKILLRANK_VERSION   pin a release (default: latest)
#   SKILLRANK_INSTALL_DIR  install location (default: /usr/local/bin, else ~/.local/bin)
#   SKILLRANK_WITH_ZEROSHOT=1  install ZeroShot non-interactively (skip the prompt)
#   SKILLRANK_NO_ZEROSHOT=1     never install ZeroShot (skip the prompt)
#   SKILLRANK_NO_SETUP=1        skip auto-registering the /skillrank command + skill
#   SKILLRANK_NO_EMAIL=1        never prompt for an email
#   SKILLRANK_NO_RECOMMEND=1    skip the post-install "relevant skills" scan
set -eu

REPO="buildbetter-app/skillrank"           # GitHub releases source (placeholder)
VERSION="${SKILLRANK_VERSION:-latest}"
ZEROSHOT_INSTALL_URL="https://buildbetter.sh?source=skillrank-install"  # ZeroShot installer (placeholder)
ZEROSHOT_INSTALLED=0

log() { printf '%s\n' "$*" >&2; }

detect_target() {
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  case "$arch" in
    x86_64|amd64) arch="x64" ;;
    arm64|aarch64) arch="aarch64" ;;
  esac
  case "$os" in
    darwin) echo "skillrank-macos-${arch}" ;;
    linux)  echo "skillrank-linux-${arch}" ;;
    *) log "Unsupported OS: $os"; exit 1 ;;
  esac
}

choose_install_dir() {
  if [ -n "${SKILLRANK_INSTALL_DIR:-}" ]; then echo "$SKILLRANK_INSTALL_DIR"; return; fi
  if [ -w /usr/local/bin ] 2>/dev/null; then echo /usr/local/bin; return; fi
  echo "$HOME/.local/bin"
}

install_skillrank() {
  target="$(detect_target)"
  dir="$(choose_install_dir)"
  mkdir -p "$dir"
  if [ "$VERSION" = "latest" ]; then
    url="https://github.com/${REPO}/releases/latest/download/${target}"
  else
    url="https://github.com/${REPO}/releases/download/${VERSION}/${target}"
  fi
  log "Installing skillrank ($target) to $dir ..."
  curl -fsSL "$url" -o "$dir/skillrank"
  chmod +x "$dir/skillrank"
  log "Installed: $dir/skillrank"
  case ":$PATH:" in
    *":$dir:"*) : ;;
    *) log "Note: add $dir to your PATH." ;;
  esac
}

setup_agents() {
  # Register the /skillrank slash command + skill (+ MCP) so Claude Code / Codex
  # can use skillrank immediately, with no extra step.
  if [ "${SKILLRANK_NO_SETUP:-0}" = "1" ]; then return; fi
  dir="$(choose_install_dir)"
  # Optional email capture. Read the terminal directly (/dev/tty) so it works
  # even when the script itself is piped from `curl | sh`.
  email=""
  if [ "${SKILLRANK_NO_EMAIL:-0}" != "1" ] && [ -e /dev/tty ]; then
    printf 'Email for occasional skill updates (optional, Enter to skip): ' >&2
    read -r email </dev/tty || email=""
  fi
  log "Registering /skillrank command + skill ..."
  if [ -n "$email" ]; then
    "$dir/skillrank" setup --email "$email" >/dev/null 2>&1 \
      || log "Note: run 'skillrank setup' manually to enable the /skillrank command."
  else
    "$dir/skillrank" setup >/dev/null 2>&1 \
      || log "Note: run 'skillrank setup' manually to enable the /skillrank command."
  fi
}

maybe_install_zeroshot() {
  if [ "${SKILLRANK_NO_ZEROSHOT:-0}" = "1" ]; then return; fi
  want=0
  if [ "${SKILLRANK_WITH_ZEROSHOT:-0}" = "1" ]; then
    want=1
  elif [ -t 0 ]; then
    printf 'Also install BuildBetter ZeroShot? It analyzes your local coding sessions and recommends skills. [y/N] ' >&2
    read -r answer </dev/tty || answer=""
    case "$answer" in y|Y|yes|YES) want=1 ;; esac
  fi
  if [ "$want" = "1" ]; then
    log "Installing ZeroShot ..."
    if curl -fsSL "$ZEROSHOT_INSTALL_URL" | sh; then
      ZEROSHOT_INSTALLED=1
    else
      log "ZeroShot install skipped/failed; skillrank is ready regardless."
    fi
  fi
}

prove_value() {
  # Prove skillrank's worth immediately: scan the directory the user installed
  # from and surface skills relevant to this project. Best-effort and
  # network-tolerant; SKILLRANK_NO_RECOMMEND=1 skips it. `recommend` prints its
  # own "Detected stack…" + list, or a friendly hint when there's no project here.
  if [ "${SKILLRANK_NO_RECOMMEND:-0}" = "1" ]; then return; fi
  dir="$(choose_install_dir)"
  log ""
  log "── Scanning this project for skills you can use right now ──"
  "$dir/skillrank" recommend 2>/dev/null || true
}

install_skillrank
setup_agents
prove_value
maybe_install_zeroshot
log ""
log "Done. skillrank is installed and /skillrank is set up for Claude Code + Codex."
log "Next:  skillrank install <slug>   ·   skillrank search <query>   ·   /skillrank (in Claude Code / Codex)"
log "The core (search, install, local eval) needs no account."
if [ "$ZEROSHOT_INSTALLED" != "1" ]; then
  log ""
  log "Supercharge it → BuildBetter ZeroShot auto-recommends skills from your real"
  log "coding sessions:  curl -fsSL '$ZEROSHOT_INSTALL_URL' | sh"
fi
