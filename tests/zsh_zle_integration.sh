#!/usr/bin/env bash
# Zsh 5.8+ PTY contract tests for the generated Zsh artifact.
set -euo pipefail

: "${HASHAI_BIN:?set HASHAI_BIN to the compiled hashai binary}"
: "${HASHAI_ZSH_BIN:=zsh}"

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
export XDG_DATA_HOME="$test_dir/data"
"$HASHAI_BIN" integration generate --shell zsh >/dev/null
artifact="$XDG_DATA_HOME/hashai/integrations/hashai.zsh"
# The emitted marker must not appear verbatim in setup input: a terminal echoes
# typed setup before Zsh executes it, and the PTY runner waits on the output.
readiness_command="print -r -- '__HASHAI_PTY_''READY__'"

fake_bin="$test_dir/bin"
mkdir -p "$fake_bin"
cat >"$fake_bin/hashai" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s' "${5-}" >"$HASHAI_REQUEST_FILE"
case ${HASHAI_TEST_MODE:-success} in
    success) printf '%s\n' "printf '日本語 😀  spaced'" ;;
    multiline) printf '%s\n' $'printf \'first\'\nprintf \'日本語 😀\'\n' ;;
    noauto) printf 'touch -- %q\n' "$HASHAI_AUTOEXEC_MARKER" ;;
    failure) printf '%s\n' 'fake core failure' >&2; exit 6 ;;
    timeout) printf '%s\n' 'fake core timeout' >&2; exit 6 ;;
    cancel) printf '%s\n' 'fake core cancelled' >&2; exit 7 ;;
    empty) exit 0 ;;
    *) printf 'unknown mode\n' >&2; exit 2 ;;
esac
EOF
chmod +x "$fake_bin/hashai"

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
    local artifact_path=$1 mode=$2 line=$3 point=$4 keymap=$5 trigger=${6-}
    local commands="$test_dir/commands" buffer="$test_dir/buffer" cursor="$test_dir/cursor"
    local request="$test_dir/request" binding="$test_dir/binding"
    local line_length=${#line} move_count left_moves=
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
cat >"$commands" <<EOF
bindkey -$keymap
source '$artifact_path'
source '$artifact_path'
bindkey -M emacs '^G' >'$binding.emacs'
bindkey -M viins '^G' >'$binding.viins'
function __hashai_zsh_capture_buffer() {
    print -rn -- "\$BUFFER" >'$buffer'
    print -r -- "\$CURSOR" >'$cursor'
    BUFFER=exit
    zle accept-line
}
zle -N __hashai_zsh_capture_buffer
bindkey '^X' __hashai_zsh_capture_buffer
functions[__hashai_zsh_real]=\$functions[__hashai_zsh_replace_buffer]
function __hashai_zsh_replace_buffer() {
    __hashai_zsh_real
    print -u2 -r -- '__HASHAI_PTY_''READY__'
}
EOF
    printf '%s\n\0' "$readiness_command" >>"$commands"
    printf '%s%s\007\0\030' "$line" "$left_moves" >>"$commands"
    if ! PATH="$fake_bin:$PATH" HASHAI_TEST_MODE="$mode" HASHAI_REQUEST_FILE="$request" \
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
    grep -F '__hashai_zsh_replace_buffer' "$binding.emacs" >/dev/null
    grep -F '__hashai_zsh_replace_buffer' "$binding.viins" >/dev/null
    TTY_BUFFER=$buffer
    TTY_CURSOR=$cursor
    TTY_REQUEST=$request
    TTY_LOG="$test_dir/tty.log"
    TTY_BINDING_EMACS="$binding.emacs"
    TTY_BINDING_VIINS="$binding.viins"
}

run_binding_dispatch() {
    local request="$test_dir/dispatch-request" commands="$test_dir/dispatch-commands"
    : >"$request"
    printf "source '%s'\n%s\n%s\nzle -N __hashai_zsh_exit_widget\nbindkey '^X' __hashai_zsh_exit_widget\n%s\n%s\n\0" \
        "$artifact" 'functions[__hashai_zsh_real]=$functions[__hashai_zsh_replace_buffer]' \
        "function __hashai_zsh_replace_buffer() { __hashai_zsh_real; print -u2 -r -- '__HASHAI_PTY_''READY__'; }" \
        'function __hashai_zsh_exit_widget() { BUFFER=exit; zle accept-line; }' "$readiness_command" >"$commands"
    printf '%s\007\0\030' '# dispatch 日本語 😀' >>"$commands"
    if ! PATH="$fake_bin:$PATH" HASHAI_TEST_MODE=noauto HASHAI_REQUEST_FILE="$request" \
        HASHAI_AUTOEXEC_MARKER="$test_dir/autoexecuted" HASHAI_TRIGGER='# ' \
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
    PATH="$fake_bin:$PATH" HASHAI_TEST_MODE=success HASHAI_REQUEST_FILE="$request" \
        HASHAI_ZSH_BIN="$HASHAI_ZSH_BIN" "$HASHAI_ZSH_BIN" -f -c \
        "source '$artifact'; BUFFER=\"\$1\"; CURSOR=5; __hashai_zsh_replace_buffer; print -rn -- \"\$BUFFER\" >'$result'" \
        -- "$line"
    NONINTERACTIVE_BUFFER=$result
    NONINTERACTIVE_REQUEST=$request
}

run_interactive_non_tty() {
    local request="$test_dir/interactive-non-tty-request"
    : >"$request"
    PATH="$fake_bin:$PATH" HASHAI_TEST_MODE=success HASHAI_REQUEST_FILE="$request" \
        HASHAI_ZSH_BIN="$HASHAI_ZSH_BIN" "$HASHAI_ZSH_BIN" -f -i <<EOF
source '$artifact'
BUFFER='# interactive non-tty'
CURSOR=5
__hashai_zsh_replace_buffer
EOF
    INTERACTIVE_NON_TTY_REQUEST=$request
}

original=$'# 日本語 😀  \'quoted\'  $(echo no) !  whitespace '
success_line="printf '日本語 😀  spaced'"
success_point=${#success_line}
run_tty "$artifact" success "$original" 5 e '# '
printf '%s' "$success_line" >"$test_dir/expected-success-buffer"
printf '%s\n' "$success_point" >"$test_dir/expected-success-cursor"
assert_file_equals "$test_dir/expected-success-buffer" "$TTY_BUFFER"
assert_file_equals "$test_dir/expected-success-cursor" "$TTY_CURSOR"
printf '%s' "${original#"# "}" >"$test_dir/expected-request"
assert_file_equals "$test_dir/expected-request" "$TTY_REQUEST"

# Literal Ctrl+G reaches the installed widget. Ctrl+U clears the replacement
# rather than Enter executing it, so the fake Core's touch must not run.
printf '%s' 'dispatch 日本語 😀' >"$test_dir/expected-dispatch-request"
run_binding_dispatch
assert_file_equals "$test_dir/expected-dispatch-request" "$DISPATCH_REQUEST"
test ! -e "$test_dir/autoexecuted"

multiline_line=$'printf \'first\'\nprintf \'日本語 😀\'\n'
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
for mode in failure timeout cancel empty; do
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

# The installed Ctrl+G binding is also active in vi insert mode. The cursor
# begins in the middle of a UTF-8 buffer and reaches the command end only on success.
run_tty "$artifact" success "$original" 5 v '# '
assert_file_equals "$test_dir/expected-success-buffer" "$TTY_BUFFER"
assert_file_equals "$test_dir/expected-success-cursor" "$TTY_CURSOR"

# AC-8: structural success and failure mutations are caught by the same oracles.
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
