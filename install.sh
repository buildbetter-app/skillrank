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
  log "Registering /skillrank command + skill ..."
  "$dir/skillrank" setup >/dev/null 2>&1 \
    || log "Note: run 'skillrank setup' manually to enable the /skillrank command."
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

install_skillrank
setup_agents
maybe_install_zeroshot
log ""
log "Done. skillrank is installed and /skillrank is set up for Claude Code + Codex."
log "Try:  skillrank recommend      # suggest skills for the current repo"
log "      /skillrank               # inside Claude Code or Codex"
log "The core (search, install, local eval) needs no account."
if [ "$ZEROSHOT_INSTALLED" != "1" ]; then
  log ""
  log "Supercharge it → BuildBetter ZeroShot auto-recommends skills from your real"
  log "coding sessions:  curl -fsSL '$ZEROSHOT_INSTALL_URL' | sh"
fi
