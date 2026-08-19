#!/usr/bin/env bash
set -euo pipefail
: "${HASHAI_BIN:?}"; : "${HASHAI_FISH_BIN:=fish}"
"$HASHAI_FISH_BIN" --version | grep -Eq 'fish, version ([4-9]|3\.[6-9])'
d=$(mktemp -d); trap 'rm -rf "$d"' EXIT; export XDG_DATA_HOME="$d/data"
"$HASHAI_BIN" integration generate --shell fish >/dev/null; a="$XDG_DATA_HOME/hashai/integrations/hashai.fish"
mkdir "$d/bin"
printf '%s\n' '#!/usr/bin/env bash' 'test "$#" -eq 5' 'test "$1" = generate' 'test "$2" = --shell' 'test "$3" = fish' 'test "$4" = --' 'printf "%s" "${5-}" >"$HASHAI_REQUEST_FILE"' "case \${HASHAI_TEST_MODE:-success} in success) printf '%s\\n' \"printf '日本語 😀  spaced'\";; multiline) printf '%s' \$'printf first\\nprintf 日本語 😀\\n\\n';; noauto) printf 'touch -- %q\\n' \"\$HASHAI_AUTOEXEC_MARKER\";; empty) :;; malformed) printf malformed;; failure|timeout|cancel) printf fake-failure >&2; exit 6;; esac" >"$d/bin/hashai"; chmod +x "$d/bin/hashai"
run() { local mode=$1; local b=$2; local c=$3; local map=${4:-default}; local trigger=${5:-'# '}; local artifact=${6:-$a}; local length moves=; length=$("$HASHAI_FISH_BIN" -c 'string length -- "$argv[1]"' -- "$b"); while (( length > c )); do moves+=$'\e[D'; ((length--)); done; : >"$d/request"; cat >"$d/cmd" <<EOF
bind -M $map \\cg __hashai_fish_replace_buffer
source '$artifact'
source '$artifact'
functions -c __hashai_fish_replace_buffer __hashai_fish_real
function __hashai_fish_replace_buffer; __hashai_fish_real; echo '__HASHAI_FISH_'READY__ >&2; end
function __fish_capture; set -l raw (commandline | string collect -N); string match -rq '^(?<captured>(?s:.*))\\n\\z' -- "\$raw"; printf %s "\$captured" >'$d/buffer'; commandline --cursor >'$d/cursor'; commandline -r exit; commandline -f execute; end
bind \\cx __fish_capture
echo '__HASHAI_FISH_'READY__
EOF
printf '%s%s\007\0\030' "$b" "$moves" >>"$d/cmd"
if ! PATH="$d/bin:$PATH" HASHAI_TEST_MODE="$mode" HASHAI_REQUEST_FILE="$d/request" HASHAI_TRIGGER="$trigger" HASHAI_AUTOEXEC_MARKER="$d/auto" HASHAI_FISH_BIN="$HASHAI_FISH_BIN" python3 tests/fish_pty.py "$d/cmd" >"$d/log"; then
    cat "$d/log" >&2
    return 1
fi
}
original=$'# 日本語 😀  '\''quoted'\'' $(echo no) !  whitespace '
run success "$original" 5 default; printf '%s' "printf '日本語 😀  spaced'" >"$d/expected"; cmp "$d/expected" "$d/buffer"; "$HASHAI_FISH_BIN" -c "string length -- \"printf '日本語 😀  spaced'\"" >"$d/cursor-expected"; cmp "$d/cursor-expected" "$d/cursor"; printf '%s' "${original#'# '}" >"$d/expected-request"; cmp "$d/expected-request" "$d/request"
run success "$original" 5 insert; cmp "$d/expected" "$d/buffer"
run multiline "$original" 5 default; printf '%s' $'printf first\nprintf 日本語 😀\n' >"$d/expected"; cmp "$d/expected" "$d/buffer"
run noauto "$original" 5 default; test ! -e "$d/auto"
run success 'echo untouched' 3 default; printf '%s' 'echo untouched' >"$d/expected"; cmp "$d/expected" "$d/buffer"; test ! -s "$d/request"
for mode in failure timeout cancel empty malformed; do run "$mode" "$original" 5 default; printf '%s' "$original" >"$d/expected"; cmp "$d/expected" "$d/buffer"; cmp <(printf '5\n') "$d/cursor"; [[ $mode == empty || $mode == malformed ]] || grep -F 'command generation failed' "$d/log"; done
run success ',, 日本語 😀' 2 default ',, '; printf '%s' "printf '日本語 😀  spaced'" >"$d/expected"; cmp "$d/expected" "$d/buffer"
# AC-5: sourcing noninteractive or interactive-without-a-TTY never invokes Core.
: >"$d/request"
PATH="$d/bin:$PATH" HASHAI_REQUEST_FILE="$d/request" "$HASHAI_FISH_BIN" --no-config -c "source '$a'"
test ! -s "$d/request"
: >"$d/request"
PATH="$d/bin:$PATH" HASHAI_REQUEST_FILE="$d/request" "$HASHAI_FISH_BIN" --no-config --interactive <<EOF >/dev/null 2>&1
source '$a'
set -g __hashai_fish_enabled 0
__hashai_fish_replace_buffer
EOF
test ! -s "$d/request"
mutated="$d/hashai.success-mutated.fish"
sed 's/commandline --replace -- "\$generated"/commandline --replace -- corrupted/' "$a" >"$mutated"
test "$(grep -Fc 'commandline --replace -- corrupted' "$mutated")" -eq 1
run success "$original" 5 default '# ' "$mutated"; printf '%s' "printf '日本語 😀  spaced'" >"$d/expected"; if cmp -s "$d/expected" "$d/buffer"; then printf 'success mutation was not detected\n' >&2; exit 1; fi
failure_mutated="$d/hashai.failure-mutated.fish"
sed "s/echo 'hashai: command generation failed; input preserved' >&2/commandline --replace -- corrupted; commandline --cursor 0; echo 'hashai: command generation failed; input preserved' >&2/" "$a" >"$failure_mutated"
test "$(grep -Fc 'commandline --replace -- corrupted; commandline --cursor 0' "$failure_mutated")" -eq 1
run failure "$original" 5 default '# ' "$failure_mutated"; printf '%s' "$original" >"$d/expected"; if cmp -s "$d/expected" "$d/buffer"; then printf 'failure mutation was not detected\n' >&2; exit 1; fi
printf 'Fish commandline PTY integration checks passed.\n'
