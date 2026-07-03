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
set -eu

REPO="buildbetter-app/skillrank"           # GitHub releases source (placeholder)
VERSION="${SKILLRANK_VERSION:-latest}"
ZEROSHOT_INSTALL_URL="https://buildbetter.sh"  # ZeroShot installer (placeholder)

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
    curl -fsSL "$ZEROSHOT_INSTALL_URL" | sh || log "ZeroShot install skipped/failed; skillrank is ready regardless."
  else
    log "Skipped ZeroShot. Install later any time: curl -fsSL $ZEROSHOT_INSTALL_URL | sh"
  fi
}

install_skillrank
maybe_install_zeroshot
log ""
log "Done. Try:  skillrank search playwright"
log "The core (search, install, local eval) needs no account."
