# Zsh ZLE integration for hashai.
#
# This file is embedded in the managed artifact. It does not run hashai until
# the user invokes the ZLE binding.

typeset -g __hashai_zsh_trigger='# '
typeset -g __hashai_zsh_keybinding='^G'
typeset -g __hashai_zsh_enabled=1

__hashai_zsh_replace_buffer() {
    emulate -L zsh
    setopt localtraps
    local trigger=${HASHAI_TRIGGER:-$__hashai_zsh_trigger}
    local request output error generated worker core_status original_buffer original_cursor
    local locale_name frame_index interrupted=0
    local -a frames

    # ZLE widgets run with stdin/stdout redirected even when their editor is
    # interactive. The install path below establishes the TTY boundary first;
    # this flag then permits only that installed widget to invoke Core.
    [[ ${__hashai_zsh_zle_enabled:-} == 1 ]] || return 0
    [[ $BUFFER == "$trigger"* ]] || return 0

    original_buffer=$BUFFER
    original_cursor=$CURSOR
    request=${BUFFER#"$trigger"}
    if ! output=$(mktemp "${TMPDIR:-/tmp}/hashai-zle-out.XXXXXX"); then
        print -u2 -- 'hashai: could not prepare command output; input preserved'
        return 0
    fi
    if ! error=$(mktemp "${TMPDIR:-/tmp}/hashai-zle-err.XXXXXX"); then
        rm -f -- "$output"
        print -u2 -- 'hashai: could not prepare command output; input preserved'
        return 0
    fi
    if [[ ! -f $output || -L $output || ! -f $error || -L $error ]]; then
        rm -f -- "$output" "$error"
        print -u2 -- 'hashai: could not prepare command output; input preserved'
        return 0
    fi

    locale_name=${LC_ALL:-${LC_CTYPE:-${LANG:-}}}
    locale_name=${(U)locale_name}
    if [[ $locale_name == *UTF-8* || $locale_name == *UTF8* ]]; then
        frames=($'\u280b' $'\u2819' $'\u2839' $'\u2838' $'\u283c' $'\u2834' $'\u2826' $'\u2827' $'\u2807' $'\u280f')
    else
        frames=('|' / - '\\')
    fi

    command hashai generate --shell zsh -- "$request" >"$output" 2>"$error" &
    worker=$!
    trap 'if (( ! interrupted )); then kill -INT "$worker" 2>/dev/null; interrupted=1; fi' INT
    frame_index=1
    BUFFER="$original_buffer  ${frames[$frame_index]} generating…"
    CURSOR=${#BUFFER}
    zle redisplay
    zle -R
    while kill -0 "$worker" 2>/dev/null; do
        sleep 0.1
        kill -0 "$worker" 2>/dev/null || break
        frame_index=$(( frame_index % ${#frames[@]} + 1 ))
        BUFFER="$original_buffer  ${frames[$frame_index]} generating…"
        CURSOR=${#BUFFER}
        zle redisplay
        zle -R
    done
    wait "$worker"
    core_status=$?
    BUFFER=$original_buffer
    CURSOR=$original_cursor
    if [[ -s $error ]]; then
        cat -- "$error" >&2 || print -u2 -- 'hashai: could not forward command diagnostic; input preserved'
    fi
    if (( core_status != 0 )); then
        rm -f -- "$output" "$error"
        print -u2 -- 'hashai: command generation failed; input preserved'
        zle redisplay
        return 0
    fi

    # Command substitution drops trailing newlines. Appending a sentinel lets
    # us remove only Core's record delimiter while retaining command newlines.
    generated=$(cat -- "$output"; printf x)
    rm -f -- "$output" "$error"
    generated=${generated%x}
    if [[ $generated != *$'\n' ]]; then
        print -u2 -- 'hashai: malformed command output; input preserved'
        return 0
    fi
    generated=${generated%$'\n'}

    if [[ -z $generated ]]; then
        print -u2 -- 'hashai: empty command; input preserved'
        zle redisplay
        return 0
    fi

    BUFFER=$generated
    CURSOR=${#BUFFER}
    zle redisplay
}

__hashai_zsh_install_binding() {
    # Keep this renderer-local seam separate from the public configuration
    # surface until a keybinding setting is specified.
    local keybinding=$__hashai_zsh_keybinding
    [[ $__hashai_zsh_enabled == 1 && ${HASHAI_TRIGGER_ENABLED:-true} != false ]] || return 0
    [[ -o interactive && -t 0 && -t 1 && -t 2 ]] || return 0
    zle -N __hashai_zsh_replace_buffer
    bindkey -M emacs "$keybinding" __hashai_zsh_replace_buffer
    bindkey -M viins "$keybinding" __hashai_zsh_replace_buffer
    typeset -g __hashai_zsh_zle_enabled=1
}

__hashai_zsh_install_binding
