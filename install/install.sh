#!/bin/sh
# Solarbox: fetch the installer for this computer and run it.
#   sh -c "$(curl -fsSL https://get.solarbox.workers.dev/install.sh)"
# macOS, Linux, Android (Termux). The installer itself checks the signature of
# everything it puts on the router.
set -e
RELAY=https://get.solarbox.workers.dev

case "$(uname -s)" in
  Darwin) os=macos ;;
  Linux) if [ -n "$TERMUX_VERSION" ] || [ -n "$ANDROID_ROOT" ]; then os=android; else os=linux; fi ;;
  *) echo "Эта система не поддерживается. На Windows: irm $RELAY/install.ps1 | iex" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  aarch64|arm64) arch=arm64 ;;
  *) echo "Процессор $(uname -m) не поддерживается" >&2; exit 1 ;;
esac
t=$os-$arch

said=
while :; do
  r=$(curl -fsS "$RELAY/v1/installer/$t") || { echo "Сервер сборок недоступен" >&2; exit 1; }
  case "$r" in
    *'"status":"ready"'*) break ;;
    *'"status":"building"'*)
      [ -n "$said" ] || echo "Готовлю установщик для $t (≈3–5 мин, один раз для всех)…"
      said=1; sleep 15 ;;
    *) echo "Сервер сборок ответил: $r" >&2; exit 1 ;;
  esac
done
url=$(printf '%s' "$r" | sed -n 's/.*"bin":{"url":"\([^"]*\)".*/\1/p')

dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
curl -fsSL -o "$dir/solarbox-install" "$url"
chmod +x "$dir/solarbox-install"
"$dir/solarbox-install" "$@" </dev/tty
