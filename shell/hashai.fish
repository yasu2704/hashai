# Fish commandline integration for hashai.

function __hashai_fish_replace_buffer
    test "$__hashai_fish_enabled" = 1; or return 0
    set -l trigger {{TRIGGER}}
    if set -q HASHAI_TRIGGER
        set trigger $HASHAI_TRIGGER
    end
    set -l raw_buffer (commandline --current-buffer | string collect -N)
    string match -rq '^(?<buffer>(?s:.*))\n\z' -- "$raw_buffer"; or return 0
    test (string sub -s 1 -l (string length -- "$trigger") -- "$buffer") = "$trigger"; or return 0

    set -l raw_request (string sub -s (math (string length -- "$trigger") + 1) -- "$buffer" | string collect -N)
    string match -rq '^(?<request>(?s:.*))\n\z' -- "$raw_request"; or return 0
    set -l output (command hashai generate --shell fish -- "$request" | string collect -N)
    set -l core_status $pipestatus[1]
    if test "$core_status" -ne 0
        echo 'hashai: command generation failed; input preserved' >&2
        return 0
    end
    string match -rq -- '(?s)^(?<generated>.*)\n\z' "$output"; or begin
        echo 'hashai: malformed command output; input preserved' >&2
        return 0
    end
    test -n "$generated"; or begin
        echo 'hashai: empty command; input preserved' >&2
        return 0
    end
    commandline --replace -- "$generated"
    commandline --cursor (string length -- "$generated")
end

function __hashai_fish_install_binding
    status is-interactive; or return 0
    test -t 0; and test -t 1; and test -t 2; or return 0
    test {{ENABLED}} = 1; and test "$HASHAI_TRIGGER_ENABLED" != false; or return 0
    bind --mode default {{KEYBINDING}} __hashai_fish_replace_buffer
    bind --mode insert {{KEYBINDING}} __hashai_fish_replace_buffer
    set -g __hashai_fish_enabled 1
end

__hashai_fish_install_binding
