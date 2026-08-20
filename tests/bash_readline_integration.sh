#!/usr/bin/env bash
# Bash 5.2+ PTY contract tests for the generated Bash artifact.
set -euo pipefail

: "${HASHAI_BIN:?set HASHAI_BIN to the compiled hashai binary}"
: "${HASHAI_BASH_BIN:=bash}"
# shellcheck source=shell_contract_cases.sh
source tests/shell_contract_cases.sh

if (( BASH_VERSINFO[0] < 5 || (BASH_VERSINFO[0] == 5 && BASH_VERSINFO[1] < 2) )); then
    printf 'Bash 5.2+ is required; found %s\n' "$BASH_VERSION" >&2
    exit 1
fi

test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT
export XDG_DATA_HOME="$test_dir/data"
"$HASHAI_BIN" integration install --shell bash --trigger '@@ ' --keybinding ctrl-x >/dev/null
artifact="$XDG_DATA_HOME/hashai/integrations/hashai.bash"

# Artifact rendering must remain parseable for every supported trigger shape.
# This runs the public generator, not a template-only helper.
injection_marker="/tmp/hashai-trigger-injection-$$"
rm -f "$injection_marker"
trap 'rm -rf "$test_dir"; rm -f "$injection_marker"' EXIT
# shellcheck disable=SC1003 # literal quote/substitution corpus values
for corpus_trigger in "'" '"' '\\' "\$(touch '$injection_marker')" "\`touch '$injection_marker'\`" ';' $'\t' '日本語' '😀' ' leading' 'trailing '; do
    "$HASHAI_BIN" integration install --shell bash --trigger "$corpus_trigger" --keybinding ctrl-x >/dev/null
    "$HASHAI_BASH_BIN" -n "$artifact"
    printf '%s' "$corpus_trigger" >"$test_dir/expected-trigger"
    # shellcheck disable=SC2016 # expansion intentionally occurs in the child shell
    env -u HASHAI_TRIGGER "$HASHAI_BASH_BIN" --noprofile --norc -c \
        'source "$1"; printf %s "$__hashai_bash_trigger"' _ "$artifact" >"$test_dir/actual-trigger"
    cmp -s "$test_dir/expected-trigger" "$test_dir/actual-trigger"
    test ! -e "$injection_marker"
done
"$HASHAI_BIN" integration install --shell bash --trigger '@@ ' --keybinding ctrl-x >/dev/null

fake_bin="$test_dir/bin"
write_shell_contract_fake "$fake_bin" bash

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
    PATH="$fake_bin:$PATH" HASHAI_EXPECTED_SHELL=bash HASHAI_TEST_MODE="$mode" HASHAI_REQUEST_FILE="$request" HASHAI_INITIAL_LINE="$line" HASHAI_KEYBINDING=ctrl-g \
        HASHAI_TRIGGER="$trigger" \
        python3 tests/bash_readline_pty.py "$commands" >"$test_dir/tty.log"
    printf '%s\n%s\n%s\n' "$result" "$request" "$bindings"
}

run_binding_dispatch() {
    local artifact_path=${1:-$artifact} mode=${2:-noauto} dispatch_line=${3:-'@@ dispatch 日本語 😀'} point=${4:-5} trigger=${5:-'@@ '}
    local request="$test_dir/dispatch-request" result="$test_dir/dispatch-result" bindings="$test_dir/dispatch-bindings" commands="$test_dir/dispatch-commands"
    local characters moves=
    characters=${#dispatch_line}
    while (( characters > point )); do
        moves+='\e[D'
        ((characters--))
    done
    : >"$request"
    cat >"$commands" <<EOF
__hashai_capture() {
    printf '%s\\n%s\\n' "\$READLINE_LINE" "\$READLINE_POINT" >'$result'
    exit
}
source '$artifact_path'
bind -X >'$bindings'
bind -x '"\\C-a":__hashai_capture'
EOF
    printf '\0' >>"$commands"
    # Ctrl-X is the installed artifact binding; Ctrl-A is solely the test
    # capture binding and then Enter exits without executing the replacement.
    # The fake consumes stdin while Ctrl-X is running, so the capture key must
    # be a later PTY group rather than bytes queued behind the Core request.
    printf '%s%b\030\0\001' "$dispatch_line" "$moves" >>"$commands"
    if ! PATH="$fake_bin:$PATH" HASHAI_EXPECTED_SHELL=bash HASHAI_TEST_MODE="$mode" HASHAI_REQUEST_FILE="$request" HASHAI_KEYBINDING=ctrl-g \
        HASHAI_AUTOEXEC_MARKER="$test_dir/autoexecuted" HASHAI_TRIGGER="$trigger" \
        python3 tests/bash_readline_pty.py "$commands" >"$test_dir/dispatch.log"; then
        cat "$test_dir/dispatch.log" >&2
        return 1
    fi
    printf '%s\n%s\n%s\n' "$request" "$result" "$bindings"
}

run_non_tty() {
    local line=$1 point=$2 result="$test_dir/non-tty-result" request="$test_dir/non-tty-request"
    : >"$request"
    PATH="$fake_bin:$PATH" HASHAI_EXPECTED_SHELL=bash HASHAI_TEST_MODE=success HASHAI_REQUEST_FILE="$request" HASHAI_INITIAL_LINE="$line" \
        "$HASHAI_BASH_BIN" -c "source '$artifact'; READLINE_LINE=\"\$HASHAI_INITIAL_LINE\"; READLINE_POINT=$point; __hashai_bash_replace_line; printf '%s\\n%s\\n' \"\$READLINE_LINE\" \"\$READLINE_POINT\" >'$result'"
    printf '%s\n%s\n' "$result" "$request"
}

run_interactive_piped_stdio() {
    local line=$1 point=$2 result="$test_dir/interactive-pipe-result" request="$test_dir/interactive-pipe-request" stderr="$test_dir/interactive-pipe-stderr"
    : >"$request"
    printf "source '%s'\nREADLINE_LINE=\$HASHAI_INITIAL_LINE\nREADLINE_POINT=%s\n__hashai_bash_replace_line\nprintf '%%s\\n%%s\\n' \"\$READLINE_LINE\" \"\$READLINE_POINT\" >'%s'\nexit\n" "$artifact" "$point" "$result" |
        PATH="$fake_bin:$PATH" HASHAI_EXPECTED_SHELL=bash HASHAI_TEST_MODE=success HASHAI_REQUEST_FILE="$request" HASHAI_INITIAL_LINE="$line" \
        "$HASHAI_BASH_BIN" -i > /dev/null 2>"$stderr"
    printf '%s\n%s\n%s\n' "$result" "$request" "$stderr"
}

original=$HASHAI_CONTRACT_REQUEST
success_line=$HASHAI_CONTRACT_SUCCESS
success_point=${#success_line}
# The artifact bakes `@@ ` and Ctrl-X, while this PTY source environment
# supplies `HASHAI_TRIGGER='# '`. Literal Ctrl-X below therefore proves the
# enabled runtime trigger compatibility seam rather than a binding-text check.
readarray -t files < <(run_tty "$artifact" success "$original" 5 '# ' '\C-x')
printf '%s\n%s\n' "$success_line" "$success_point" >"$test_dir/expected-success"
assert_file_equals "$test_dir/expected-success" "${files[0]}"
printf '%s' "${original#"# "}" >"$test_dir/expected-request"
assert_file_equals "$test_dir/expected-request" "${files[1]}"
grep -F '__hashai_bash_replace_line' "${files[2]}" >/dev/null

# AC-1/AC-5: review and dangerous warnings remain visible on the interactive
# terminal while the command stays an editable replacement, never execution.
for mode in review dangerous; do
    readarray -t files < <(run_tty "$artifact" "$mode" "$original" 5 '# ' '\\C-x')
    assert_file_equals "$test_dir/expected-success" "${files[0]}"
    assert_file_equals "$test_dir/expected-request" "${files[1]}"
    case $mode in
        review) grep -F "$HASHAI_CONTRACT_REVIEW_WARNING" "$test_dir/tty.log" >/dev/null ;;
        dangerous) grep -F "$HASHAI_CONTRACT_DANGEROUS_WARNING" "$test_dir/tty.log" >/dev/null ;;
    esac
    test ! -e "$test_dir/autoexecuted"
done

# A literal Ctrl+X reaches the bind -x function in the PTY. Ctrl+U clears the
# replacement instead of Enter executing it; the fake Core's touch command
# therefore proves the artifact never auto-executes generated text.
printf '%s' 'dispatch 日本語 😀' >"$test_dir/expected-dispatch-request"
readarray -t files < <(run_binding_dispatch "$artifact" noauto '@@ dispatch 日本語 😀' 5 '@@ ')
if ! cmp -s "$test_dir/expected-dispatch-request" "${files[0]}"; then
    cat "$test_dir/dispatch.log" >&2
fi
assert_file_equals "$test_dir/expected-dispatch-request" "${files[0]}"
test ! -e "$test_dir/autoexecuted"

# A disabled artifact has no baked Ctrl-X binding. A direct production-widget
# invocation is still inert due to its own enabled guard: no Core call and no
# buffer/cursor mutation. This avoids treating unbound Ctrl-X's Readline prefix
# behavior as a test harness exit mechanism.
"$HASHAI_BIN" integration install --shell bash --keybinding ctrl-x --disable-trigger >/dev/null
readarray -t files < <(run_tty "$artifact" success '# disabled 日本語 😀' 5 '# ' '\C-x')
printf '%s\n%s\n' '# disabled 日本語 😀' 5 >"$test_dir/expected-disabled"
assert_file_equals "$test_dir/expected-disabled" "${files[0]}"
test ! -s "${files[1]}"
if grep -F '__hashai_bash_replace_line' "${files[2]}" >/dev/null; then
    printf 'disabled Bash artifact installed Ctrl-X\n' >&2
    exit 1
fi
"$HASHAI_BIN" integration install --shell bash --trigger '@@ ' --keybinding ctrl-x >/dev/null

# dispatch permutation mutation: literal Ctrl-X must keep input when a copied
# artifact dispatches to the wrong Core shell target.
dispatch_mutated="$test_dir/hashai.dispatch-mutated.bash"
test "$(grep -Fc -- '--shell bash' "$artifact")" -eq 1
sed 's/--shell bash/--shell zsh/' "$artifact" >"$dispatch_mutated"
test "$(grep -Fc -- '--shell bash' "$dispatch_mutated")" -eq 0
test "$(grep -Fc -- '--shell zsh' "$dispatch_mutated")" -eq 1
# This literal terminal input deliberately avoids Tab: Readline completion is
# an editor transformation before the installed Ctrl-G binding can observe it.
dispatch_original='# dispatch 日本語 😀'
readarray -t files < <(run_binding_dispatch "$dispatch_mutated" success "$dispatch_original" 5 '# ')
printf '%s\n%s\n' "$dispatch_original" 5 >"$test_dir/expected-dispatch-preserved"
assert_file_equals "$test_dir/expected-dispatch-preserved" "${files[1]}"
test ! -s "${files[0]}"

# A command payload may itself end in a newline. The record delimiter is
# removed once, while the command's own trailing newline remains in Readline.
multiline_line=$HASHAI_CONTRACT_MULTILINE
multiline_point=${#multiline_line}
readarray -t files < <(run_tty "$artifact" multiline "$original" 5 '# ' '\C-x')
printf '%s\n%s\n' "$multiline_line" "$multiline_point" >"$test_dir/expected-multiline"
assert_file_equals "$test_dir/expected-multiline" "${files[0]}"

# AC-1: non-matching input must never reach Core.
readarray -t files < <(run_tty "$artifact" success 'echo untouched' 3 '# ' '\C-x')
printf '%s\n%s\n' 'echo untouched' 3 >"$test_dir/expected-untouched"
assert_file_equals "$test_dir/expected-untouched" "${files[0]}"
test ! -s "${files[1]}"

# AC-3 parity: interactive Bash with piped stdin/stdout is still non-TTY, so
# sourcing/direct widget invocation cannot call Core or alter editor state.
printf '%s\n%s\n' "$original" 5 >"$test_dir/expected-non-tty"
readarray -t files < <(run_interactive_piped_stdio "$original" 5)
assert_file_equals "$test_dir/expected-non-tty" "${files[0]}"
test ! -s "${files[1]}"
if grep -F 'hashai:' "${files[2]}" >/dev/null; then
    printf 'interactive non-TTY Bash emitted an artifact diagnostic\n' >&2
    exit 1
fi

# AC-3: ordinary error, timeout, cancellation, and empty output retain every input byte and cursor.
for mode in failure timeout cancel empty status-{1..9}; do
    readarray -t files < <(run_tty "$artifact" "$mode" "$original" 5 '# ' '\C-x')
    printf '%s\n%s\n' "$original" 5 >"$test_dir/expected-preserved"
    assert_file_equals "$test_dir/expected-preserved" "${files[0]}"
done

# AC-5: a sourced artifact cannot call Core in a non-TTY process.
readarray -t files < <(run_non_tty "$original" 5)
printf '%s\n%s\n' "$original" 5 >"$test_dir/expected-non-tty"
assert_file_equals "$test_dir/expected-non-tty" "${files[0]}"
test ! -s "${files[1]}"

# AC-7: the existing trigger configuration seam can change without regeneration.
readarray -t files < <(run_tty "$artifact" success ',, 日本語 😀' 2 ',, ' '\C-x')
assert_file_equals "$test_dir/expected-success" "${files[0]}"

# AC-8: a structural success-path mutation makes the success assertion fail.
mutated="$test_dir/hashai.mutated.bash"
# The mutation must match generated Bash literals.
# shellcheck disable=SC2016
sed 's/READLINE_LINE=$command/READLINE_LINE=$original_line/' "$artifact" >"$mutated"
readarray -t files < <(run_tty "$mutated" success "$original" 5 '# ' '\C-x')
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
readarray -t files < <(run_tty "$failure_mutated" failure "$original" 5 '# ' '\C-x')
if cmp -s "$test_dir/expected-preserved" "${files[0]}"; then
    printf 'failure-path mutation was not detected\n' >&2
    exit 1
fi

printf 'Bash Readline PTY integration checks passed.\n'
