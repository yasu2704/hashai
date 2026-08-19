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
printf '%s' "${5-}" >"$HASHAI_REQUEST_FILE"
case ${HASHAI_TEST_MODE:-success} in
    success) printf '%s\n' "printf '日本語 😀  spaced'" ;;
    multiline) printf '%s\n' $'printf \'first\'\nprintf \'日本語 😀\'\n' ;;
    noauto) printf 'touch -- %q\n' "$HASHAI_AUTOEXEC_MARKER" ;;
    empty) exit 0 ;;
    malformed) printf malformed ;;
    failure) printf '%s\n' 'fake core failure' >&2; exit 1 ;;
    timeout) printf '%s\n' 'fake core timeout' >&2; exit 6 ;;
    cancel) printf '%s\n' 'fake core cancelled' >&2; exit 7 ;;
    status-[1-9]) exit "${HASHAI_TEST_MODE#status-}" ;;
    *) printf 'unknown mode\n' >&2; exit 2 ;;
esac
EOF
    chmod +x "$directory/hashai"
    : "$shell_name"
}
