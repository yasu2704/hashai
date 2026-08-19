#!/usr/bin/env bash
# Zsh 5.8+ PTY contract tests for the generated Zsh artifact.
set -euo pipefail

: "${HASHAI_BIN:?set HASHAI_BIN to the compiled hashai binary}"
: "${HASHAI_ZSH_BIN:=zsh}"
source tests/shell_contract_cases.sh

if ! command -v "$HASHAI_ZSH_BIN" >/dev/null 2>&1; then
    printf 'Zsh 5.8+ is required, but %q was not found\n' "$HASHAI_ZSH_BIN" >&2
    exit 1
fi
zsh_version=$("$HASHAI_ZSH_BIN" -fc 'print -r -- $ZSH_VERSION')
IFS=. read -r zsh_major zsh_minor _ <<<"$zsh_version"
if (( zsh_major < 5 || (zsh_major == 5 && zsh_minor < 8) )); then
    printf 'Zsh 5.8+ is required; found %s\n' "$zsh_version" >&2
    exit 1
fi

test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
canonical_request="$test_dir/canonical-request"
printf '%s' "$HASHAI_CONTRACT_REQUEST" >"$canonical_request"
export XDG_DATA_HOME="$test_dir/data"
"$HASHAI_BIN" integration generate --shell zsh --trigger '@@ ' --keybinding ctrl-x >/dev/null
artifact="$XDG_DATA_HOME/hashai/integrations/hashai.zsh"
injection_marker="/tmp/hashai-trigger-injection-$$"
rm -f "$injection_marker"
trap 'rm -rf "$test_dir"; rm -f "$injection_marker"' EXIT
# shellcheck disable=SC1003 # literal quote/substitution corpus values
for corpus_trigger in "'" '"' '\\' "\$(touch '$injection_marker')" "\`touch '$injection_marker'\`" ';' $'\t' '日本語' '😀' ' leading' 'trailing '; do
    "$HASHAI_BIN" integration generate --shell zsh --trigger "$corpus_trigger" --keybinding ctrl-x >/dev/null
    "$HASHAI_ZSH_BIN" -n "$artifact"
    printf '%s' "$corpus_trigger" >"$test_dir/expected-trigger"
    env -u HASHAI_TRIGGER "$HASHAI_ZSH_BIN" -dfc \
        'source "$1"; print -rn -- "$__hashai_zsh_trigger"' _ "$artifact" >"$test_dir/actual-trigger"
    cmp -s "$test_dir/expected-trigger" "$test_dir/actual-trigger"
    test ! -e "$injection_marker"
done
"$HASHAI_BIN" integration generate --shell zsh --trigger '@@ ' --keybinding ctrl-x >/dev/null
# The emitted marker must not appear verbatim in setup input: a terminal echoes
# typed setup before Zsh executes it, and the PTY runner waits on the output.
readiness_command="print -r -- '__HASHAI_PTY_''READY__'"

fake_bin="$test_dir/bin"
write_shell_contract_fake "$fake_bin" zsh

assert_file_equals() {
    local expected=$1 actual=$2
    if ! cmp -s "$expected" "$actual"; then
        if [[ -n ${TTY_BINDING_EMACS:-} ]]; then
            printf '%s\n' 'Zsh emacs Ctrl-G binding:' >&2
            cat "$TTY_BINDING_EMACS" >&2
            printf '%s\n' 'Zsh viins Ctrl-G binding:' >&2
            cat "$TTY_BINDING_VIINS" >&2
            printf '%s\n' 'Fake Core request bytes:' >&2
            wc -c <"$TTY_REQUEST" >&2
            printf '%s\n' 'Fake Core request:' >&2
            cat "$TTY_REQUEST" >&2
        fi
        if [[ -n ${TTY_LOG:-} && -f $TTY_LOG ]]; then
            printf '%s\n' 'Zsh PTY log follows:' >&2
            cat "$TTY_LOG" >&2
        fi
        diff -u "$expected" "$actual" >&2 || true
        exit 1
    fi
}

run_tty() {
    local artifact_path=$1 mode=$2 line=$3 point=$4 keymap=$5 trigger=${6-} load_from_file=${7-} expect_binding=${8:-yes}
    local commands="$test_dir/commands" setup="$test_dir/zle-setup" buffer="$test_dir/buffer" cursor="$test_dir/cursor"
    local request="$test_dir/request" binding="$test_dir/binding" setup_buffer="$test_dir/setup-buffer"
    local line_length=${#line} move_count left_moves=
    if [[ $line == "$HASHAI_CONTRACT_REQUEST" ]]; then
        load_from_file=$canonical_request
    fi
    : >"$request"
    if (( point > line_length )); then
        printf 'test cursor %s exceeds line length %s\n' "$point" "$line_length" >&2
        exit 1
    fi
    move_count=$((line_length - point))
    while (( move_count > 0 )); do
        left_moves+=$'\e[D'
        ((move_count--))
    done
# Keep declarations out of the terminal input stream. macOS ZLE can redraw
# while a long interactive paste is still arriving; source is atomic shell
# input, and the test still drives the installed binding with literal Ctrl-G.
cat >"$setup" <<EOF
bindkey -$keymap
source '$artifact_path'
source '$artifact_path'
bindkey -M emacs '^X' >'$binding.emacs'
bindkey -M viins '^X' >'$binding.viins'
function __hashai_zsh_capture_buffer() {
    print -rn -- "\$BUFFER" >'$buffer'
    print -r -- "\$CURSOR" >'$cursor'
    BUFFER=exit
    zle accept-line
}
zle -N __hashai_zsh_capture_buffer
bindkey '^T' __hashai_zsh_capture_buffer
function __hashai_zsh_load_contract_buffer() {
    BUFFER="\$(<'$load_from_file')"
    CURSOR=$point
    print -rn -- "\$BUFFER" >'$setup_buffer'
    zle redisplay
    print -u2 -r -- '__HASHAI_PTY_''READY__'
}
zle -N __hashai_zsh_load_contract_buffer
bindkey '^Y' __hashai_zsh_load_contract_buffer
functions[__hashai_zsh_real]=\$functions[__hashai_zsh_replace_buffer]
function __hashai_zsh_replace_buffer() {
    __hashai_zsh_real
    print -u2 -r -- '__HASHAI_PTY_''READY__'
}
EOF
    if [[ $expect_binding == no ]]; then
        printf "bindkey '^Y' __hashai_zsh_replace_buffer\n" >>"$setup"
    fi
    printf "source '%s'\n%s\n\0" "$setup" "$readiness_command" >"$commands"
    if [[ -n $load_from_file ]]; then
        # Ctrl-Y is a test-only setup widget. It loads the canonical bytes,
        # including Tab, then literal Ctrl-G invokes the installed artifact.
        printf '\031\0\030\0\024' >>"$commands"
    elif [[ $expect_binding == no ]]; then
        # Invoke the real guarded production widget through a test-only key;
        # never depend on platform-specific behavior for an unbound Ctrl-X.
        printf '%s%s\031\0\024' "$line" "$left_moves" >>"$commands"
    else
        printf '%s%s\030\0\024' "$line" "$left_moves" >>"$commands"
    fi
    if ! PATH="$fake_bin:$PATH" HASHAI_EXPECTED_SHELL=zsh HASHAI_TEST_MODE="$mode" HASHAI_REQUEST_FILE="$request" HASHAI_KEYBINDING=ctrl-g \
        HASHAI_TRIGGER="$trigger" HASHAI_ZSH_BIN="$HASHAI_ZSH_BIN" \
        python3 tests/zsh_zle_pty.py "$commands" >"$test_dir/tty.log"; then
        printf '%s\n' 'Zsh PTY runner failed; log follows:' >&2
        cat "$test_dir/tty.log" >&2
        return 1
    fi
    if [[ ! -f $buffer || ! -f $cursor ]]; then
        printf '%s\n' 'Zsh PTY did not produce a capture manifest; log follows:' >&2
        cat "$test_dir/tty.log" >&2
        return 1
    fi
    if [[ $expect_binding == yes ]]; then
        grep -F '__hashai_zsh_replace_buffer' "$binding.emacs" >/dev/null
        grep -F '__hashai_zsh_replace_buffer' "$binding.viins" >/dev/null
    fi
    TTY_BUFFER=$buffer
    TTY_CURSOR=$cursor
    TTY_REQUEST=$request
    TTY_LOG="$test_dir/tty.log"
    TTY_BINDING_EMACS="$binding.emacs"
    TTY_BINDING_VIINS="$binding.viins"
    TTY_SETUP_BUFFER=$setup_buffer
    if [[ -n $load_from_file ]]; then
        assert_file_equals "$load_from_file" "$setup_buffer"
    fi
}

run_binding_dispatch() {
    local request="$test_dir/dispatch-request" commands="$test_dir/dispatch-commands"
    : >"$request"
    printf "source '%s'\n%s\n%s\nzle -N __hashai_zsh_exit_widget\nbindkey '^T' __hashai_zsh_exit_widget\n%s\n%s\n\0" \
        "$artifact" 'functions[__hashai_zsh_real]=$functions[__hashai_zsh_replace_buffer]' \
        "function __hashai_zsh_replace_buffer() { __hashai_zsh_real; print -u2 -r -- '__HASHAI_PTY_''READY__'; }" \
        'function __hashai_zsh_exit_widget() { BUFFER=exit; zle accept-line; }' "$readiness_command" >"$commands"
    printf '%s\030\0\024' '@@ dispatch 日本語 😀' >>"$commands"
    if ! PATH="$fake_bin:$PATH" HASHAI_EXPECTED_SHELL=zsh HASHAI_TEST_MODE=noauto HASHAI_REQUEST_FILE="$request" \
        HASHAI_AUTOEXEC_MARKER="$test_dir/autoexecuted" HASHAI_TRIGGER='@@ ' \
        HASHAI_ZSH_BIN="$HASHAI_ZSH_BIN" \
        python3 tests/zsh_zle_pty.py "$commands" >"$test_dir/dispatch.log"; then
        printf '%s\n' 'Zsh dispatch PTY runner failed; log follows:' >&2
        cat "$test_dir/dispatch.log" >&2
        return 1
    fi
    DISPATCH_REQUEST=$request
}

run_noninteractive() {
    local line=$1 result="$test_dir/non-tty-result" request="$test_dir/non-tty-request"
    : >"$request"
    PATH="$fake_bin:$PATH" HASHAI_EXPECTED_SHELL=zsh HASHAI_TEST_MODE=success HASHAI_REQUEST_FILE="$request" \
        HASHAI_ZSH_BIN="$HASHAI_ZSH_BIN" "$HASHAI_ZSH_BIN" -f -c \
        "source '$artifact'; BUFFER=\"\$1\"; CURSOR=5; __hashai_zsh_replace_buffer; print -rn -- \"\$BUFFER\" >'$result'" \
        -- "$line"
    NONINTERACTIVE_BUFFER=$result
    NONINTERACTIVE_REQUEST=$request
}

run_interactive_non_tty() {
    local request="$test_dir/interactive-non-tty-request"
    : >"$request"
    PATH="$fake_bin:$PATH" HASHAI_EXPECTED_SHELL=zsh HASHAI_TEST_MODE=success HASHAI_REQUEST_FILE="$request" \
        HASHAI_ZSH_BIN="$HASHAI_ZSH_BIN" "$HASHAI_ZSH_BIN" -f -i <<EOF
source '$artifact'
BUFFER='# interactive non-tty'
CURSOR=5
__hashai_zsh_replace_buffer
EOF
    INTERACTIVE_NON_TTY_REQUEST=$request
}

original=$HASHAI_CONTRACT_REQUEST
success_line=$HASHAI_CONTRACT_SUCCESS
success_point=${#success_line}
# The generated artifact bakes `@@ `/Ctrl-X. `run_tty` overrides only the
# runtime trigger to `# ` and then sends literal Ctrl-X through ZLE.
run_tty "$artifact" success "$original" 5 e '# '
printf '%s' "$success_line" >"$test_dir/expected-success-buffer"
printf '%s\n' "$success_point" >"$test_dir/expected-success-cursor"
assert_file_equals "$test_dir/expected-success-buffer" "$TTY_BUFFER"
assert_file_equals "$test_dir/expected-success-cursor" "$TTY_CURSOR"
printf '%s' "${original#"# "}" >"$test_dir/expected-request"
assert_file_equals "$test_dir/expected-request" "$TTY_REQUEST"
# The test-only setup widget was actually used before literal Ctrl-G and
# retained the canonical tab-containing bytes exactly in ZLE's BUFFER.
assert_file_equals "$canonical_request" "$TTY_SETUP_BUFFER"

# Literal Ctrl+X reaches the installed widget. Ctrl+U clears the replacement
# rather than Enter executing it, so the fake Core's touch must not run.
printf '%s' 'dispatch 日本語 😀' >"$test_dir/expected-dispatch-request"
run_binding_dispatch
assert_file_equals "$test_dir/expected-dispatch-request" "$DISPATCH_REQUEST"
test ! -e "$test_dir/autoexecuted"

# Disabled artifacts register no Ctrl-X widget. Directly invoking the
# production widget inside an interactive ZLE session remains inert because
# its runtime enabled guard is unset; no request or buffer mutation occurs.
"$HASHAI_BIN" integration generate --shell zsh --keybinding ctrl-x --disable-trigger >/dev/null
run_tty "$artifact" success '# disabled 日本語 😀' 5 e '# ' '' no
printf '%s' '# disabled 日本語 😀' >"$test_dir/expected-disabled-buffer"
printf '%s\n' 5 >"$test_dir/expected-disabled-cursor"
assert_file_equals "$test_dir/expected-disabled-buffer" "$TTY_BUFFER"
assert_file_equals "$test_dir/expected-disabled-cursor" "$TTY_CURSOR"
test ! -s "$TTY_REQUEST"
if grep -F '__hashai_zsh_replace_buffer' "$TTY_BINDING_EMACS" "$TTY_BINDING_VIINS" >/dev/null; then
    printf 'disabled Zsh artifact installed Ctrl-X\n' >&2
    exit 1
fi
"$HASHAI_BIN" integration generate --shell zsh --trigger '@@ ' --keybinding ctrl-x >/dev/null

multiline_line=$HASHAI_CONTRACT_MULTILINE
multiline_point=${#multiline_line}
run_tty "$artifact" multiline "$original" 5 e '# '
printf '%s' "$multiline_line" >"$test_dir/expected-multiline-buffer"
printf '%s\n' "$multiline_point" >"$test_dir/expected-multiline-cursor"
assert_file_equals "$test_dir/expected-multiline-buffer" "$TTY_BUFFER"
assert_file_equals "$test_dir/expected-multiline-cursor" "$TTY_CURSOR"

# AC-1: non-matching input must never reach Core.
run_tty "$artifact" success 'echo untouched' 3 e '# '
printf '%s' 'echo untouched' >"$test_dir/expected-untouched-buffer"
printf '%s\n' 3 >"$test_dir/expected-untouched-cursor"
assert_file_equals "$test_dir/expected-untouched-buffer" "$TTY_BUFFER"
assert_file_equals "$test_dir/expected-untouched-cursor" "$TTY_CURSOR"
test ! -s "$TTY_REQUEST"

# AC-3: errors, timeout, cancellation, and empty output preserve buffer and cursor.
for mode in failure timeout cancel empty status-{1..9}; do
    run_tty "$artifact" "$mode" "$original" 5 e '# '
    printf '%s' "$original" >"$test_dir/expected-preserved-buffer"
    printf '%s\n' 5 >"$test_dir/expected-preserved-cursor"
    assert_file_equals "$test_dir/expected-preserved-buffer" "$TTY_BUFFER"
    assert_file_equals "$test_dir/expected-preserved-cursor" "$TTY_CURSOR"
    if [[ $mode != empty ]]; then
        grep -F 'hashai: command generation failed; input preserved' "$TTY_LOG" >/dev/null
    fi
done

# AC-5: neither noninteractive nor interactive non-TTY sourcing can call Core.
run_noninteractive "$original"
printf '%s' "$original" >"$test_dir/expected-non-tty"
assert_file_equals "$test_dir/expected-non-tty" "$NONINTERACTIVE_BUFFER"
test ! -s "$NONINTERACTIVE_REQUEST"
run_interactive_non_tty
test ! -s "$INTERACTIVE_NON_TTY_REQUEST"

# AC-7: a trigger seam may change without regenerating the artifact.
run_tty "$artifact" success ',, 日本語 😀' 2 e ',, '
assert_file_equals "$test_dir/expected-success-buffer" "$TTY_BUFFER"

# The installed Ctrl+X binding is also active in vi insert mode. The cursor
# begins in the middle of a UTF-8 buffer and reaches the command end only on success.
run_tty "$artifact" success "$original" 5 v '# '
assert_file_equals "$test_dir/expected-success-buffer" "$TTY_BUFFER"
assert_file_equals "$test_dir/expected-success-cursor" "$TTY_CURSOR"

# dispatch permutation mutation: installed literal Ctrl-G keeps input when a
# copied artifact dispatches to the wrong Core shell target.
dispatch_mutated="$test_dir/hashai.dispatch-mutated.zsh"
test "$(grep -Fc -- '--shell zsh' "$artifact")" -eq 1
sed 's/--shell zsh/--shell fish/' "$artifact" >"$dispatch_mutated"
test "$(grep -Fc -- '--shell zsh' "$dispatch_mutated")" -eq 0
test "$(grep -Fc -- '--shell fish' "$dispatch_mutated")" -eq 1
run_tty "$dispatch_mutated" success "$original" 5 e '# '
printf '%s' "$original" >"$test_dir/expected-dispatch-preserved"
assert_file_equals "$test_dir/expected-dispatch-preserved" "$TTY_BUFFER"
test ! -s "$TTY_REQUEST"
mutated="$test_dir/hashai.mutated.zsh"
sed 's/BUFFER=$generated/BUFFER=corrupted/' "$artifact" >"$mutated"
grep -F 'BUFFER=corrupted' "$mutated" >/dev/null
run_tty "$mutated" success "$original" 5 e '# '
if cmp -s "$test_dir/expected-success-buffer" "$TTY_BUFFER"; then
    printf 'success-path mutation was not detected\n' >&2
    exit 1
fi

failure_mutated="$test_dir/hashai.failure-mutated.zsh"
sed "s/print -u2 -- 'hashai: command generation failed; input preserved'/BUFFER=corrupted; CURSOR=0; print -u2 -- 'hashai: command generation failed; input preserved'/" \
    "$artifact" >"$failure_mutated"
grep -F 'BUFFER=corrupted; CURSOR=0' "$failure_mutated" >/dev/null
run_tty "$failure_mutated" failure "$original" 5 e '# '
if cmp -s "$test_dir/expected-preserved-buffer" "$TTY_BUFFER"; then
    if cmp -s "$test_dir/expected-preserved-cursor" "$TTY_CURSOR"; then
        printf 'failure-path mutation was not detected\n' >&2
        exit 1
    fi
fi

printf 'Zsh ZLE PTY integration checks passed.\n'
