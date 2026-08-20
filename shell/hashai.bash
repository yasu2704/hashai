# Bash Readline integration for hashai.
#
# This file is embedded in the managed artifact. It does not run hashai until
# the user invokes the Readline binding.

__hashai_bash_trigger='# '
__hashai_bash_keybinding='\C-g'
__hashai_bash_enabled=1

__hashai_bash_progress_capabilities() {
    [[ ${TERM:-dumb} != dumb ]] || return 1
    command -v tput >/dev/null 2>&1 || return 1
    __hashai_bash_progress_cr=$(tput cr 2>/dev/null) || return 1
    __hashai_bash_progress_el=$(tput el 2>/dev/null) || return 1
}

__hashai_bash_progress_frames() {
    local locale_name=${LC_ALL:-${LC_CTYPE:-${LANG:-}}}
    locale_name=${locale_name^^}
    if [[ $locale_name == *UTF-8* || $locale_name == *UTF8* ]]; then
        __hashai_bash_progress_frames=(⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏)
    else
        __hashai_bash_progress_frames=('|' / - '\')
    fi
}

__hashai_bash_progress_draw() {
    local frame=$1
    printf '%s%s%s generating…' "$__hashai_bash_progress_cr" "$__hashai_bash_progress_el" "$frame" >&2
}

__hashai_bash_progress_clear() {
    [[ ${__hashai_bash_progress_visible:-0} == 1 ]] || return 0
    printf '%s%s' "$__hashai_bash_progress_cr" "$__hashai_bash_progress_el" >&2
    __hashai_bash_progress_visible=0
}

__hashai_bash_replace_line() {
    local trigger=${HASHAI_TRIGGER:-$__hashai_bash_trigger}
    local request command output error worker status frame_index original_int_trap

    # The binding installer is normally the only entry point, but retaining
    # this guard also makes an explicitly disabled generated artifact inert if
    # its function is invoked directly by another Readline customization.
    [[ $__hashai_bash_enabled == 1 && ${HASHAI_TRIGGER_ENABLED:-true} != false ]] || return 0

    # bind -x is meaningful only for an interactive terminal. In particular,
    # do not run Core when a sourced artifact is used by a script or a pipe.
    [[ -t 0 && -t 1 && -t 2 ]] || return 0
    [[ $READLINE_LINE == "$trigger"* ]] || return 0

    if ! __hashai_bash_progress_capabilities; then
        printf '%s\n' 'hashai: terminal progress display unavailable; input preserved' >&2
        return 0
    fi

    request=${READLINE_LINE#"$trigger"}
    if ! output=$(mktemp "${TMPDIR:-/tmp}/hashai-readline-out.XXXXXX"); then
        printf '%s\n' 'hashai: could not prepare command output; input preserved' >&2
        return 0
    fi
    if ! error=$(mktemp "${TMPDIR:-/tmp}/hashai-readline-err.XXXXXX"); then
        rm -f -- "$output"
        printf '%s\n' 'hashai: could not prepare command output; input preserved' >&2
        return 0
    fi
    if [[ ! -f $output || -L $output || ! -f $error || -L $error ]]; then
        rm -f -- "$output" "$error"
        printf '%s\n' 'hashai: could not prepare command output; input preserved' >&2
        return 0
    fi

    command hashai generate --shell bash -- "$request" >"$output" 2>"$error" &
    worker=$!
    __hashai_bash_progress_frames
    __hashai_bash_progress_visible=1
    printf '\n' >&2
    frame_index=0
    __hashai_bash_progress_draw "${__hashai_bash_progress_frames[$frame_index]}"
    original_int_trap=$(trap -p INT)
    __hashai_bash_interrupted=0
    trap '__hashai_bash_interrupted=1' INT
    while kill -0 "$worker" 2>/dev/null; do
        if [[ $__hashai_bash_interrupted == 1 ]]; then
            kill -INT "$worker" 2>/dev/null || true
            __hashai_bash_interrupted=2
        fi
        sleep 0.1 || true
        kill -0 "$worker" 2>/dev/null || break
        frame_index=$(( (frame_index + 1) % ${#__hashai_bash_progress_frames[@]} ))
        __hashai_bash_progress_draw "${__hashai_bash_progress_frames[$frame_index]}"
    done
    wait "$worker"
    status=$?
    if [[ -n $original_int_trap ]]; then
        # Bash exposes an existing handler only as reusable shell syntax.
        # Re-source that trusted shell-owned serialization without adding a
        # second command-string interpretation primitive to the artifact.
        builtin source /dev/stdin <<<"$original_int_trap"
    else
        trap - INT
    fi
    __hashai_bash_progress_clear
    if [[ -s $error ]]; then
        cat -- "$error" >&2 || printf '%s\n' 'hashai: could not forward command diagnostic; input preserved' >&2
    fi
    if (( status != 0 )); then
        rm -f -- "$output" "$error"
        printf '%s\n' 'hashai: command generation failed; input preserved' >&2
        return 0
    fi

    # `hashai generate` writes one record newline. Appending a sentinel before
    # command substitution retains all command newlines; remove only that one
    # record delimiter afterwards. Bash variables cannot represent NUL, which
    # is outside the Core command-string contract.
    command=$(cat -- "$output"; printf x)
    rm -f -- "$output" "$error"
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
