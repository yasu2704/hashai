#!/usr/bin/env bash
# Bash 5.2+ PTY contract tests for the generated Bash artifact.
set -euo pipefail

: "${HASHAI_BIN:?set HASHAI_BIN to the compiled hashai binary}"
: "${HASHAI_BASH_BIN:=bash}"

if (( BASH_VERSINFO[0] < 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] < 2) )); then
    printf 'Bash 5.2+ is required; found %s\n' "$BASH_VERSION" >&2
    exit 1
fi

test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
export XDG_DATA_HOME="$test_dir/data"
"$HASHAI_BIN" integration generate --shell bash >/dev/null
artifact="$XDG_DATA_HOME/hashai/integrations/hashai.bash"

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
        diff -u "$expected" "$actual" >&2 || true
        exit 1
    fi
}

run_tty() {
    local artifact_path=$1 mode=$2 line=$3 point=$4 trigger=${5-}
    local commands="$test_dir/commands" result="$test_dir/result" request="$test_dir/request" bindings="$test_dir/bindings"
    : >"$request"
    cat >"$commands" <<EOF
source '$artifact_path'
source '$artifact_path'
bind -X >'$bindings'
READLINE_LINE=\$HASHAI_INITIAL_LINE
READLINE_POINT=$point
__hashai_bash_replace_line
printf '%s\\n%s\\n' "\$READLINE_LINE" "\$READLINE_POINT" >'$result'
exit
EOF
    PATH="$fake_bin:$PATH" HASHAI_TEST_MODE="$mode" HASHAI_REQUEST_FILE="$request" HASHAI_INITIAL_LINE="$line" \
        HASHAI_TRIGGER="$trigger" \
        python3 tests/bash_readline_pty.py "$commands" >"$test_dir/tty.log"
    printf '%s\n%s\n%s\n' "$result" "$request" "$bindings"
}

run_binding_dispatch() {
    local dispatch_line='# dispatch 日本語 😀'
    local request="$test_dir/dispatch-request" commands="$test_dir/dispatch-commands"
    : >"$request"
    printf "source '%s'\n\0" "$artifact" >"$commands"
    printf '%s\007\0\025exit\n' "$dispatch_line" >>"$commands"
    PATH="$fake_bin:$PATH" HASHAI_TEST_MODE=noauto HASHAI_REQUEST_FILE="$request" \
        HASHAI_AUTOEXEC_MARKER="$test_dir/autoexecuted" HASHAI_TRIGGER='# ' \
        python3 tests/bash_readline_pty.py "$commands" >"$test_dir/dispatch.log"
    printf '%s\n' "$request"
}

run_non_tty() {
    local line=$1 point=$2 result="$test_dir/non-tty-result" request="$test_dir/non-tty-request"
    : >"$request"
    PATH="$fake_bin:$PATH" HASHAI_TEST_MODE=success HASHAI_REQUEST_FILE="$request" HASHAI_INITIAL_LINE="$line" \
        "$HASHAI_BASH_BIN" -c "source '$artifact'; READLINE_LINE=\"\$HASHAI_INITIAL_LINE\"; READLINE_POINT=$point; __hashai_bash_replace_line; printf '%s\\n%s\\n' \"\$READLINE_LINE\" \"\$READLINE_POINT\" >'$result'"
    printf '%s\n%s\n' "$result" "$request"
}

original=$'# 日本語 😀  \'quoted\'  $(echo no) !\twhitespace '
success_line="printf '日本語 😀  spaced'"
success_point=${#success_line}
readarray -t files < <(run_tty "$artifact" success "$original" 5 '# ' '\C-g')
printf '%s\n%s\n' "$success_line" "$success_point" >"$test_dir/expected-success"
assert_file_equals "$test_dir/expected-success" "${files[0]}"
printf '%s' "${original#"# "}" >"$test_dir/expected-request"
assert_file_equals "$test_dir/expected-request" "${files[1]}"
grep -F '__hashai_bash_replace_line' "${files[2]}" >/dev/null

# A literal Ctrl+G reaches the bind -x function in the PTY. Ctrl+U clears the
# replacement instead of Enter executing it; the fake Core's touch command
# therefore proves the artifact never auto-executes generated text.
printf '%s' 'dispatch 日本語 😀' >"$test_dir/expected-dispatch-request"
readarray -t files < <(run_binding_dispatch)
if ! cmp -s "$test_dir/expected-dispatch-request" "${files[0]}"; then
    cat "$test_dir/dispatch.log" >&2
fi
assert_file_equals "$test_dir/expected-dispatch-request" "${files[0]}"
test ! -e "$test_dir/autoexecuted"

# A command payload may itself end in a newline. The record delimiter is
# removed once, while the command's own trailing newline remains in Readline.
multiline_line=$'printf \'first\'\nprintf \'日本語 😀\'\n'
multiline_point=${#multiline_line}
readarray -t files < <(run_tty "$artifact" multiline "$original" 5 '# ' '\C-g')
printf '%s\n%s\n' "$multiline_line" "$multiline_point" >"$test_dir/expected-multiline"
assert_file_equals "$test_dir/expected-multiline" "${files[0]}"

# AC-1: non-matching input must never reach Core.
readarray -t files < <(run_tty "$artifact" success 'echo untouched' 3 '# ' '\C-g')
printf '%s\n%s\n' 'echo untouched' 3 >"$test_dir/expected-untouched"
assert_file_equals "$test_dir/expected-untouched" "${files[0]}"
test ! -s "${files[1]}"

# AC-3: ordinary error, timeout, cancellation, and empty output retain every input byte and cursor.
for mode in failure timeout cancel empty; do
    readarray -t files < <(run_tty "$artifact" "$mode" "$original" 5 '# ' '\C-g')
    printf '%s\n%s\n' "$original" 5 >"$test_dir/expected-preserved"
    assert_file_equals "$test_dir/expected-preserved" "${files[0]}"
done

# AC-5: a sourced artifact cannot call Core in a non-TTY process.
readarray -t files < <(run_non_tty "$original" 5)
printf '%s\n%s\n' "$original" 5 >"$test_dir/expected-non-tty"
assert_file_equals "$test_dir/expected-non-tty" "${files[0]}"
test ! -s "${files[1]}"

# AC-7: the existing trigger configuration seam can change without regeneration.
readarray -t files < <(run_tty "$artifact" success ',, 日本語 😀' 2 ',, ' '\C-g')
assert_file_equals "$test_dir/expected-success" "${files[0]}"

# AC-8: a structural success-path mutation makes the success assertion fail.
mutated="$test_dir/hashai.mutated.bash"
# The mutation must match generated Bash literals.
# shellcheck disable=SC2016
sed 's/READLINE_LINE=$command/READLINE_LINE=$original_line/' "$artifact" >"$mutated"
readarray -t files < <(run_tty "$mutated" success "$original" 5 '# ' '\C-g')
if cmp -s "$test_dir/expected-success" "${files[0]}"; then
    printf 'success-path mutation was not detected\n' >&2
    exit 1
fi

# AC-8: a structural failure-path mutation must break the same byte-for-byte
# buffer and cursor preservation oracle used for Core failures.
failure_mutated="$test_dir/hashai.failure-mutated.bash"
# The mutation must match generated Bash literals.
# shellcheck disable=SC2016
sed 's/return 0/READLINE_LINE=corrupted; READLINE_POINT=0; return 0/' "$artifact" >"$failure_mutated"
readarray -t files < <(run_tty "$failure_mutated" failure "$original" 5 '# ' '\C-g')
if cmp -s "$test_dir/expected-preserved" "${files[0]}"; then
    printf 'failure-path mutation was not detected\n' >&2
    exit 1
fi

printf 'Bash Readline PTY integration checks passed.\n'
