#!/usr/bin/env bash
set -euo pipefail
: "${HASHAI_BIN:?}"; : "${HASHAI_FISH_BIN:=fish}"
source tests/shell_contract_cases.sh
"$HASHAI_FISH_BIN" --version | grep -Eq 'fish, version ([4-9]|3\.[6-9])'
d=$(mktemp -d); trap 'rm -rf "$d"' EXIT; export XDG_DATA_HOME="$d/data"
"$HASHAI_BIN" integration generate --shell fish >/dev/null; a="$XDG_DATA_HOME/hashai/integrations/hashai.fish"
mkdir "$d/bin"
write_shell_contract_fake "$d/bin" fish
printf '%s\n' '#!/usr/bin/env bash' "printf '%s' \$'# first natural language\\n日本語 😀 second line' >\"\$1\"" >"$d/editor"; chmod +x "$d/editor"
run() { local mode=$1; local b=$2; local c=$3; local map=${4:-default}; local trigger=${5:-'# '}; local artifact=${6:-$a}; local mode_setup= length moves=; [[ $map == insert ]] && mode_setup=$'fish_vi_key_bindings\nset -g fish_bind_mode insert'; length=$("$HASHAI_FISH_BIN" -c 'string length -- "$argv[1]"' -- "$b"); while (( length > c )); do moves+=$'\e[D'; ((length--)); done; : >"$d/request"; cat >"$d/cmd" <<EOF
$mode_setup
bind -M $map \\cg __hashai_fish_replace_buffer
source '$artifact'
source '$artifact'
bind -M default \\cg >'$d/binding.default'
bind -M insert \\cg >'$d/binding.insert'
functions -c __hashai_fish_replace_buffer __hashai_fish_real
function __hashai_fish_replace_buffer; set -l raw (commandline --current-buffer | string collect -N); string match -rq '^(?<exposed>(?s:.*))\\n\\z' -- "\$raw"; printf %s "\$exposed" >'$d/exposed'; __hashai_fish_real; echo '__HASHAI_FISH_'READY__ >&2; end
function __fish_capture; set -l raw (commandline | string collect -N); string match -rq '^(?<captured>(?s:.*))\\n\\z' -- "\$raw"; printf %s "\$captured" >'$d/buffer'; commandline --cursor >'$d/cursor'; commandline -r exit; commandline -f execute; end
bind \\cx __fish_capture
bind -M insert \\cx __fish_capture
function __fish_edit_buffer; edit_command_buffer; echo '__HASHAI_FISH_'READY__ >&2; end
bind \\cy __fish_edit_buffer
echo '__HASHAI_FISH_'READY__
EOF
if [[ $b == __NORMALIZED_MULTILINE__ ]]; then printf '\031\0\007\0\030' >>"$d/cmd"; else printf '%s%s\007\0\030' "$b" "$moves" >>"$d/cmd"; fi
if ! PATH="$d/bin:$PATH" HASHAI_EXPECTED_SHELL=fish HASHAI_TEST_MODE="$mode" HASHAI_REQUEST_FILE="$d/request" HASHAI_TRIGGER="$trigger" HASHAI_AUTOEXEC_MARKER="$d/auto" VISUAL="$d/editor" EDITOR="$d/editor" HASHAI_FISH_BIN="$HASHAI_FISH_BIN" python3 tests/fish_pty.py "$d/cmd" >"$d/log"; then
    cat "$d/log" >&2
    return 1
fi
}
# Fish's editor normalizes literal tabs before widget dispatch; the shared
# fixture documents the raw request while this PTY supplement tests exposed bytes.
original=$HASHAI_CONTRACT_FISH_EXPOSED_REQUEST
run success "$original" 5 default; printf '%s' "$HASHAI_CONTRACT_SUCCESS" >"$d/expected"; cmp "$d/expected" "$d/buffer"; "$HASHAI_FISH_BIN" -c "string length -- \"$HASHAI_CONTRACT_SUCCESS\"" >"$d/cursor-expected"; cmp "$d/cursor-expected" "$d/cursor"; tail -c +3 "$d/exposed" >"$d/expected-request"; cmp "$d/expected-request" "$d/request"
# Fish owns editor normalization before this contract boundary; request bytes
# are compared to the captured exposed buffer above.
run success __NORMALIZED_MULTILINE__ 0 default; printf '%s' $'# first natural language\n日本語 😀 second line' >"$d/expected-exposed"; cmp "$d/expected-exposed" "$d/exposed"; printf '%s' $'first natural language\n日本語 😀 second line' >"$d/expected-request"; cmp "$d/expected-request" "$d/request"
run success "$original" 5 insert; cmp "$d/expected" "$d/buffer"
grep -F '__hashai_fish_replace_buffer' "$d/binding.default" "$d/binding.insert" >/dev/null
run multiline "$original" 5 default; printf '%s' "$HASHAI_CONTRACT_MULTILINE" >"$d/expected"; cmp "$d/expected" "$d/buffer"
run noauto "$original" 5 default; test ! -e "$d/auto"
run success 'echo untouched' 3 default; printf '%s' 'echo untouched' >"$d/expected"; cmp "$d/expected" "$d/buffer"; test ! -s "$d/request"
 # dispatch permutation mutation: installed literal Ctrl-G keeps the exposed input.
dispatch_mutated="$d/hashai.dispatch-mutated.fish"
test "$(grep -Fc -- '--shell fish' "$a")" -eq 1
sed 's/--shell fish/--shell bash/' "$a" >"$dispatch_mutated"
test "$(grep -Fc -- '--shell fish' "$dispatch_mutated")" -eq 0
test "$(grep -Fc -- '--shell bash' "$dispatch_mutated")" -eq 1
run success "$original" 5 default '# ' "$dispatch_mutated"; printf '%s' "$original" >"$d/expected"; cmp "$d/expected" "$d/buffer"; cmp <(printf '5\n') "$d/cursor"; test ! -s "$d/request"
for mode in failure timeout cancel empty malformed status-{1..9}; do run "$mode" "$original" 5 default; printf '%s' "$original" >"$d/expected"; cmp "$d/expected" "$d/buffer"; cmp <(printf '5\n') "$d/cursor"; [[ $mode == empty || $mode == malformed ]] || grep -F 'command generation failed' "$d/log"; done
run success ',, 日本語 😀' 2 default ',, '; printf '%s' "printf '日本語 😀  spaced'" >"$d/expected"; cmp "$d/expected" "$d/buffer"
# AC-5: sourcing noninteractive or interactive-without-a-TTY never invokes Core.
: >"$d/request"
PATH="$d/bin:$PATH" HASHAI_EXPECTED_SHELL=fish HASHAI_REQUEST_FILE="$d/request" "$HASHAI_FISH_BIN" --no-config -c "source '$a'"
test ! -s "$d/request"
: >"$d/request"
PATH="$d/bin:$PATH" HASHAI_EXPECTED_SHELL=fish HASHAI_REQUEST_FILE="$d/request" "$HASHAI_FISH_BIN" --no-config --interactive <<EOF >/dev/null 2>&1
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
