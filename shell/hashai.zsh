# Zsh ZLE integration for hashai.
#
# This file is embedded in the managed artifact. It does not run hashai until
# the user invokes the ZLE binding.

__hashai_zsh_replace_buffer() {
    emulate -L zsh
    local trigger=${HASHAI_TRIGGER:-'# '}
    local request output command

    # A managed artifact can be sourced by a script. ZLE changes are meaningful
    # only in an interactive terminal, so never start Core in any other mode.
    [[ -o interactive && -t 0 && -t 1 && -t 2 ]] || return 0
    [[ $BUFFER == "$trigger"* ]] || return 0

    request=${BUFFER#"$trigger"}
    if ! output=$(mktemp "${TMPDIR:-/tmp}/hashai-zle.XXXXXX"); then
        print -u2 -- 'hashai: could not prepare command output; input preserved'
        return 0
    fi

    if ! command hashai generate --shell zsh -- "$request" >"$output"; then
        rm -f -- "$output"
        print -u2 -- 'hashai: command generation failed; input preserved'
        return 0
    fi

    # Command substitution drops trailing newlines. Appending a sentinel lets
    # us remove only Core's record delimiter while retaining command newlines.
    command=$(cat -- "$output"; printf x)
    rm -f -- "$output"
    command=${command%x}
    if [[ $command != *$'\n' ]]; then
        print -u2 -- 'hashai: malformed command output; input preserved'
        return 0
    fi
    command=${command%$'\n'}

    if [[ -z $command ]]; then
        print -u2 -- 'hashai: empty command; input preserved'
        return 0
    fi

    BUFFER=$command
    CURSOR=${#BUFFER}
}

__hashai_zsh_install_binding() {
    # Keep this renderer-local seam separate from the public configuration
    # surface until a keybinding setting is specified.
    local keybinding='^G'
    [[ -o interactive && -t 0 && -t 1 && -t 2 ]] || return 0
    zle -N __hashai_zsh_replace_buffer
    bindkey -M emacs "$keybinding" __hashai_zsh_replace_buffer
    bindkey -M viins "$keybinding" __hashai_zsh_replace_buffer
}

__hashai_zsh_install_binding
