# Zsh ZLE integration for hashai.
#
# This file is embedded in the managed artifact. It does not run hashai until
# the user invokes the ZLE binding.

__hashai_zsh_replace_buffer() {
    emulate -L zsh
    # ZLE gives the widget its own output plumbing. Disable multio's helper so
    # Core's redirected record is complete before this widget reads it.
    local trigger=${HASHAI_TRIGGER:-'# '}
    local request output generated

    # ZLE widgets run with stdin/stdout redirected even when their editor is
    # interactive. The install path below establishes the TTY boundary first;
    # this flag then permits only that installed widget to invoke Core.
    [[ ${__hashai_zsh_zle_enabled:-} == 1 ]] || return 0
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
    generated=$(cat -- "$output"; printf x)
    rm -f -- "$output"
    generated=${generated%x}
    if [[ $generated != *$'\n' ]]; then
        print -u2 -- 'hashai: malformed command output; input preserved'
        return 0
    fi
    generated=${generated%$'\n'}

    if [[ -z $generated ]]; then
        print -u2 -- 'hashai: empty command; input preserved'
        return 0
    fi

    BUFFER=$generated
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
    typeset -g __hashai_zsh_zle_enabled=1
}

__hashai_zsh_install_binding
