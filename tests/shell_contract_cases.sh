#!/usr/bin/env bash
# Shared fake-Core contract for the Bash, Zsh, and Fish integration harnesses.
# The public Core boundary is exactly: generate --shell <shell> -- <request>.
# Requests are the shell's exposed editor-buffer bytes after its trigger has
# been removed; the PTY-specific harness owns any editor normalization before
# that boundary. Success output is UTF-8 and intentionally includes Japanese,
# emoji, repeated whitespace, and a command-owned trailing newline variant.
HASHAI_CONTRACT_REQUEST=$'# 日本語 😀  \'quoted\'  $(echo no) !\twhitespace '
HASHAI_CONTRACT_SUCCESS="printf '日本語 😀  spaced'"
HASHAI_CONTRACT_MULTILINE=$'printf \'first\'\nprintf \'日本語 😀\'\n'
HASHAI_CONTRACT_REVIEW_WARNING='hashai: warning: generated command risk=review; inspect before execution'
HASHAI_CONTRACT_DANGEROUS_WARNING='hashai: warning: generated command risk=dangerous; inspect carefully before execution'
# Fish expands a literal tab in its editor before commandline exposure.
HASHAI_CONTRACT_FISH_EXPOSED_REQUEST=$'# 日本語 😀  \'quoted\' $(echo no) !  whitespace '

write_shell_contract_fake() {
    local directory=$1 shell_name=$2
    mkdir -p "$directory"
    cat >"$directory/hashai" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
test "$#" -eq 5
test "$1" = generate
test "$2" = --shell
test "$3" = "$HASHAI_EXPECTED_SHELL"
test "$4" = --
if [[ ${HASHAI_BASH_FOREGROUND_HANDOFF:-} == 1 && ${HASHAI_TEST_HANDOFF_ACTIVE:-} != 1 ]]; then
    exec python3 "${0%/*}/hashai-handoff.py" "$0" "$@"
fi
printf '%s' "${5-}" >"$HASHAI_REQUEST_FILE"
if [[ -n ${HASHAI_WORKER_TRACE_FILE:-} ]]; then
    printf '%s %s\n' "$$" "$(ps -o pgid= -p "$$" | tr -d ' ')" >"$HASHAI_WORKER_TRACE_FILE"
fi
case ${HASHAI_TEST_MODE:-success} in
    success) printf '%s\n' "printf '日本語 😀  spaced'" ;;
    review)
        printf '%s\n' 'hashai: warning: generated command risk=review; inspect before execution' >&2
        printf '%s\n' "printf '日本語 😀  spaced'"
        ;;
    dangerous)
        printf '%s\n' 'hashai: warning: generated command risk=dangerous; inspect carefully before execution' >&2
        printf '%s\n' "printf '日本語 😀  spaced'"
        ;;
    multiline) printf '%s\n' $'printf \'first\'\nprintf \'日本語 😀\'\n' ;;
    noauto) printf 'touch -- %q\n' "$HASHAI_AUTOEXEC_MARKER" ;;
    empty) exit 0 ;;
    malformed) printf malformed ;;
    failure) printf '%s\n' 'fake core failure' >&2; exit 1 ;;
    timeout) printf '%s\n' 'fake core timeout' >&2; exit 6 ;;
    cancel) printf '%s\n' 'fake core cancelled' >&2; exit 7 ;;
    blocking)
        : "${HASHAI_PROGRESS_RELEASE_FILE:?}"
        while [[ ! -e $HASHAI_PROGRESS_RELEASE_FILE ]]; do
            sleep 0.02
        done
        printf '%s\n' "printf '日本語 😀  spaced'"
        ;;
    interruptible)
        : "${HASHAI_SIGNAL_FILE:?}"
        trap 'printf "INT\n" >>"$HASHAI_SIGNAL_FILE"; printf "%s\n" "fake core cancelled" >&2; if [[ ${HASHAI_INTERRUPT_SUCCESS:-} == 1 ]]; then printf "%s\n" "printf '\''must not install cancelled output'\''"; exit 0; fi; exit 7' INT
        while :; do
            sleep 0.02
        done
        ;;
    status-[1-9]) exit "${HASHAI_TEST_MODE#status-}" ;;
    *) printf 'unknown mode\n' >&2; exit 2 ;;
esac
EOF
    cat >"$directory/hashai-handoff.py" <<'PY'
#!/usr/bin/env python3
import os
import signal
import subprocess
import sys

terminal = 0
original_group = os.tcgetpgrp(terminal)
os.setpgid(0, 0)
previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTTOU})
try:
    os.tcsetpgrp(terminal, os.getpgrp())
finally:
    signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
signal.signal(signal.SIGINT, lambda _signum, _frame: None)
environment = os.environ.copy()
environment["HASHAI_TEST_HANDOFF_ACTIVE"] = "1"
child = subprocess.Popen(sys.argv[1:], env=environment)
try:
    status = child.wait()
finally:
    previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTTOU})
    try:
        os.tcsetpgrp(terminal, original_group)
    finally:
        signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
raise SystemExit(status)
PY
    chmod +x "$directory/hashai"
    : "$shell_name"
}
