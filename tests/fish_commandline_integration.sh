#!/usr/bin/env bash
set -euo pipefail
: "${HASHAI_BIN:?}"; : "${HASHAI_FISH_BIN:=fish}"
"$HASHAI_FISH_BIN" --version | grep -Eq 'fish, version ([4-9]|3\.[6-9])'
d=$(mktemp -d); trap 'rm -rf "$d"' EXIT; export XDG_DATA_HOME="$d/data"
"$HASHAI_BIN" integration generate --shell fish >/dev/null; a="$XDG_DATA_HOME/hashai/integrations/hashai.fish"
mkdir "$d/bin"
printf '%s\n' '#!/usr/bin/env bash' 'printf "%s" "${5-}" >"$HASHAI_REQUEST_FILE"' "case \${HASHAI_TEST_MODE:-success} in success) printf '%s\\n' \"printf '日本語 😀  spaced'\";; failure) exit 6;; esac" >"$d/bin/hashai"; chmod +x "$d/bin/hashai"
run() { local mode=$1; local b=$2; local c=$3; : >"$d/request"; cat >"$d/cmd" <<EOF
source '$a'
functions -c __hashai_fish_replace_buffer __hashai_fish_real
function __hashai_fish_replace_buffer; __hashai_fish_real; echo __HASHAI_FISH_READY__ >&2; end
function __fish_capture; commandline --current-buffer | string collect -N >'$d/buffer'; commandline --cursor >'$d/cursor'; commandline -r exit; commandline -f execute; end
bind \\cx __fish_capture
echo __HASHAI_FISH_READY__
EOF
printf '%s\007\0\030' "$b" >>"$d/cmd"
if ! PATH="$d/bin:$PATH" HASHAI_TEST_MODE="$mode" HASHAI_REQUEST_FILE="$d/request" HASHAI_FISH_BIN="$HASHAI_FISH_BIN" python3 tests/fish_pty.py "$d/cmd" >"$d/log"; then
    cat "$d/log" >&2
    return 1
fi
}
orig=$'# 日本語 😀 quote'; run success "$orig" 5
printf "%s\n" "printf '日本語 😀  spaced'" >"$d/expected"; cmp "$d/expected" "$d/buffer"
"$HASHAI_FISH_BIN" -c "string length -- \"printf '日本語 😀  spaced'\"" >"$d/cursor-expected"; cmp "$d/cursor-expected" "$d/cursor"
run failure "$orig" 5; printf '%s\n' "$orig" >"$d/expected"; cmp "$d/expected" "$d/buffer"; grep -F 'command generation failed' "$d/log"
printf 'Fish commandline PTY integration checks passed.\n'
