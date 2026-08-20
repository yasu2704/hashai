# Fish commandline integration for hashai.

set -g __hashai_fish_trigger '# '
set -g __hashai_fish_keybinding \cg
set -g __hashai_fish_enabled_config 1

function __hashai_fish_replace_buffer
    test "$__hashai_fish_enabled" = 1; or return 0
    set -q __hashai_fish_worker_active; and return 0
    set -g __hashai_fish_worker_active 1
    set -l trigger $__hashai_fish_trigger
    if set -q HASHAI_TRIGGER
        set trigger $HASHAI_TRIGGER
    end
    set -l raw_buffer (commandline --current-buffer | string collect -N)
    string match -rq '^(?<buffer>(?s:.*))\n\z' -- "$raw_buffer"; or begin; set -e __hashai_fish_worker_active; return 0; end
    test (string sub -s 1 -l (string length -- "$trigger") -- "$buffer") = "$trigger"; or begin; set -e __hashai_fish_worker_active; return 0; end

    set -l raw_request (string sub -s (math (string length -- "$trigger") + 1) -- "$buffer" | string collect -N)
    string match -rq '^(?<request>(?s:.*))\n\z' -- "$raw_request"; or begin; set -e __hashai_fish_worker_active; return 0; end
    set -l original_cursor (commandline --cursor)
    if test "$TERM" = dumb; or not type -q tput
        echo 'hashai: terminal progress display unavailable; input preserved' >&2
        set -e __hashai_fish_worker_active
        return 0
    end
    set -l progress_cr (tput cr 2>/dev/null | string collect -N)
    set -l cr_status $pipestatus[1]
    set -l progress_el (tput el 2>/dev/null | string collect -N)
    set -l el_status $pipestatus[1]
    if test "$cr_status" -ne 0; or test "$el_status" -ne 0
        echo 'hashai: terminal progress display unavailable; input preserved' >&2
        set -e __hashai_fish_worker_active
        return 0
    end
    set -l temp_dir /tmp
    set -q TMPDIR; and set temp_dir "$TMPDIR"
    set -l stdout_file (mktemp "$temp_dir/hashai-fish-out.XXXXXX")
    or begin
        echo 'hashai: could not prepare command output; input preserved' >&2
        set -e __hashai_fish_worker_active
        return 0
    end
    set -l stderr_file (mktemp "$temp_dir/hashai-fish-err.XXXXXX")
    or begin
        command rm -f -- "$stdout_file"
        echo 'hashai: could not prepare command output; input preserved' >&2
        set -e __hashai_fish_worker_active
        return 0
    end
    if not test -f "$stdout_file"; or test -L "$stdout_file"; or not test -f "$stderr_file"; or test -L "$stderr_file"
        command rm -f -- "$stdout_file" "$stderr_file"
        echo 'hashai: could not prepare command output; input preserved' >&2
        set -e __hashai_fish_worker_active
        return 0
    end

    set -l locale_name "$LANG"
    set -q LC_CTYPE; and set locale_name "$LC_CTYPE"
    set -q LC_ALL; and set locale_name "$LC_ALL"
    set -l frames
    if string match -riq 'UTF-?8' -- "$locale_name"
        set frames ⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏
    else
        set frames '|' / - '\\'
    end

    command hashai generate --shell fish -- "$request" >"$stdout_file" 2>"$stderr_file" &
    set -l worker $last_pid
    set -g __hashai_fish_stdout_file "$stdout_file"
    set -g __hashai_fish_stderr_file "$stderr_file"
    set -g __hashai_fish_progress_cr "$progress_cr"
    set -g __hashai_fish_progress_el "$progress_el"
    set -g __hashai_fish_worker_pid $worker
    set -g __hashai_fish_int_relayed 0
    set -e __hashai_fish_cancel_cleanup_done
    functions -e __hashai_fish_worker_int 2>/dev/null
    function __hashai_fish_worker_int --on-signal INT
        if set -q __hashai_fish_worker_pid; and test "$__hashai_fish_int_relayed" = 0
            kill -INT $__hashai_fish_worker_pid 2>/dev/null
            set -g __hashai_fish_int_relayed 1
        end
    end
    set -e __hashai_fish_worker_status
    functions -e __hashai_fish_worker_exit 2>/dev/null
    function __hashai_fish_worker_exit --on-process-exit $worker
        set -g __hashai_fish_worker_status $argv[3]
        if test "$__hashai_fish_int_relayed" = 1
            # If Ctrl-C was relayed before this event, cancellation wins even
            # when the child exits 0 with complete output.
            printf '%s%s' "$__hashai_fish_progress_cr" "$__hashai_fish_progress_el" >&2
            if test -s "$__hashai_fish_stderr_file"
                command cat -- "$__hashai_fish_stderr_file" >&2
                or echo 'hashai: could not forward command diagnostic; input preserved' >&2
            end
            command rm -f -- "$__hashai_fish_stdout_file" "$__hashai_fish_stderr_file"
            echo 'hashai: command generation failed; input preserved' >&2
            set -g __hashai_fish_cancel_cleanup_done 1
            set -e -g __hashai_fish_worker_active
            functions -e __hashai_fish_worker_int
            functions -e __hashai_fish_worker_exit
            set -e -g __hashai_fish_worker_status
            set -e -g __hashai_fish_worker_pid __hashai_fish_int_relayed
            set -e -g __hashai_fish_stdout_file __hashai_fish_stderr_file
            set -e -g __hashai_fish_progress_cr __hashai_fish_progress_el
        end
    end
    set -l frame_index 1
    printf '\n%s%s%s generating…' "$progress_cr" "$progress_el" "$frames[$frame_index]" >&2
    while not set -q __hashai_fish_worker_status; and not set -q __hashai_fish_cancel_cleanup_done
        sleep 0.1
        set -q __hashai_fish_worker_status; and break
        set frame_index (math "$frame_index % "(count $frames)" + 1")
        printf '%s%s%s generating…' "$progress_cr" "$progress_el" "$frames[$frame_index]" >&2
    end
    if set -q __hashai_fish_cancel_cleanup_done
        functions -e __hashai_fish_worker_exit
        functions -e __hashai_fish_worker_int
        set -e __hashai_fish_worker_status __hashai_fish_cancel_cleanup_done
        set -e __hashai_fish_worker_pid __hashai_fish_int_relayed
        set -e __hashai_fish_stdout_file __hashai_fish_stderr_file
        set -e __hashai_fish_progress_cr __hashai_fish_progress_el
        printf '%s%s' "$progress_cr" "$progress_el" >&2
        commandline -f repaint
        return 0
    end
    # The exit event is the status source of truth. Waiting once on this known,
    # completed child is silent across supported Fish versions.
    wait $worker 2>/dev/null
    if test "$__hashai_fish_int_relayed" = 1
        printf '%s%s' "$progress_cr" "$progress_el" >&2
        if test -s "$stderr_file"
            command cat -- "$stderr_file" >&2
            or echo 'hashai: could not forward command diagnostic; input preserved' >&2
        end
        command rm -f -- "$stdout_file" "$stderr_file"
        echo 'hashai: command generation failed; input preserved' >&2
        functions -e __hashai_fish_worker_exit
        functions -e __hashai_fish_worker_int
        set -e -g __hashai_fish_worker_status
        set -e -g __hashai_fish_worker_pid __hashai_fish_int_relayed
        set -e -g __hashai_fish_stdout_file __hashai_fish_stderr_file
        set -e -g __hashai_fish_progress_cr __hashai_fish_progress_el
        set -e -g __hashai_fish_worker_active
        commandline -f repaint
        return 0
    end
    set -l core_status $__hashai_fish_worker_status
    functions -e __hashai_fish_worker_exit
    functions -e __hashai_fish_worker_int
    set -e __hashai_fish_worker_status
    set -e __hashai_fish_worker_pid __hashai_fish_int_relayed
    set -e __hashai_fish_stdout_file __hashai_fish_stderr_file
    set -e __hashai_fish_progress_cr __hashai_fish_progress_el
    printf '%s%s' "$progress_cr" "$progress_el" >&2
    if test -s "$stderr_file"
        command cat -- "$stderr_file" >&2
        or echo 'hashai: could not forward command diagnostic; input preserved' >&2
    end
    if test "$core_status" -ne 0
        command rm -f -- "$stdout_file" "$stderr_file"
        echo 'hashai: command generation failed; input preserved' >&2
        commandline -f repaint
        set -e __hashai_fish_worker_active
        return 0
    end
    set -l output (command cat -- "$stdout_file" | string collect -N)
    set -l read_status $pipestatus[1]
    command rm -f -- "$stdout_file" "$stderr_file"
    if test "$read_status" -ne 0
        echo 'hashai: could not read command output; input preserved' >&2
        commandline -f repaint
        set -e __hashai_fish_worker_active
        return 0
    end
    string match -rq -- '(?s)^(?<generated>.*)\n\z' "$output"; or begin
        echo 'hashai: malformed command output; input preserved' >&2
        commandline -f repaint
        set -e __hashai_fish_worker_active
        return 0
    end
    test -n "$generated"; or begin
        echo 'hashai: empty command; input preserved' >&2
        commandline -f repaint
        set -e __hashai_fish_worker_active
        return 0
    end
    commandline --replace -- "$generated"
    commandline --cursor (string length -- "$generated")
    commandline -f repaint
    set -e __hashai_fish_worker_active
end

function __hashai_fish_install_binding
    status is-interactive; or return 0
    test -t 0; and test -t 1; and test -t 2; or return 0
    test $__hashai_fish_enabled_config = 1; and test "$HASHAI_TRIGGER_ENABLED" != false; or return 0
    bind --mode default $__hashai_fish_keybinding __hashai_fish_replace_buffer
    bind --mode insert $__hashai_fish_keybinding __hashai_fish_replace_buffer
    set -g __hashai_fish_enabled 1
end

__hashai_fish_install_binding
