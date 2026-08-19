# Bash Readline integration for hashai.
#
# This file is embedded in the managed artifact. It does not run hashai until
# the user invokes the Readline binding.

__hashai_bash_trigger='# '
__hashai_bash_keybinding='\C-g'
__hashai_bash_enabled=1

__hashai_bash_replace_line() {
    local trigger=${HASHAI_TRIGGER:-$__hashai_bash_trigger}
    local request command output

    # bind -x is meaningful only for an interactive terminal. In particular,
    # do not run Core when a sourced artifact is used by a script or a pipe.
    [[ -t 0 && -t 1 && -t 2 ]] || return 0
    [[ $READLINE_LINE == "$trigger"* ]] || return 0

    request=${READLINE_LINE#"$trigger"}
    if ! output=$(mktemp "${TMPDIR:-/tmp}/hashai-readline.XXXXXX"); then
        printf '%s\n' 'hashai: could not prepare command output; input preserved' >&2
        return 0
    fi

    if ! command hashai generate --shell bash -- "$request" >"$output"; then
        rm -f -- "$output"
        printf '%s\n' 'hashai: command generation failed; input preserved' >&2
        return 0
    fi

    # `hashai generate` writes one record newline. Appending a sentinel before
    # command substitution retains all command newlines; remove only that one
    # record delimiter afterwards. Bash variables cannot represent NUL, which
    # is outside the Core command-string contract.
    command=$(cat -- "$output"; printf x)
    rm -f -- "$output"
    command=${command%x}
    if [[ $command != *$'\n' ]]; then
        printf '%s\n' 'hashai: malformed command output; input preserved' >&2
        return 0
    fi
    command=${command%$'\n'}

    if [[ -z $command ]]; then
        printf '%s\n' 'hashai: empty command; input preserved' >&2
        return 0
    fi

    READLINE_LINE=$command
    READLINE_POINT=${#READLINE_LINE}
}

__hashai_bash_install_binding() {
    # Keep this renderer-local seam separate from the public configuration
    # surface until a keybinding setting is specified.
    local keybinding=$__hashai_bash_keybinding
    [[ $__hashai_bash_enabled == 1 && ${HASHAI_TRIGGER_ENABLED:-true} != false ]] || return 0
    [[ -t 0 && -t 1 && -t 2 ]] || return 0
    bind -x "\"${keybinding}\":__hashai_bash_replace_line"
}

__hashai_bash_install_binding
