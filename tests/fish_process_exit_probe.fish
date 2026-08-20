set -l trace_dir $argv[1]
set -g __hashai_probe_trace_dir $trace_dir
set -l expected_status 0
if test "$argv[2]" = interrupt
    command touch "$trace_dir/signal"
    command fish -c 'set -g trace $argv[2]; function stop --on-signal INT; echo INT >$trace; exit 0; end; while true; sleep 0.01; end' probe "$trace_dir/signal" &
else
    command sh -c 'exit 0' &
end
set -l worker $last_pid

function __hashai_probe_exit --on-process-exit $worker
    printf 'event:%s\n' $argv[3] >>$__hashai_probe_trace_dir/events
    set -g __hashai_probe_status $argv[3]
end

if test "$argv[2]" = interrupt
    sleep 0.05
    kill -INT $worker
end

set -l attempts 0
while not set -q __hashai_probe_status; and test $attempts -lt 1000
    sleep 0.01
    set attempts (math $attempts + 1)
end
if not set -q __hashai_probe_status
    set -g __hashai_probe_status 255
    kill -KILL $worker 2>/dev/null
end

jobs -p >$trace_dir/jobs
printf '%s\n' $status >$trace_dir/jobs-status
wait $worker 2>$trace_dir/wait-stderr
printf '%s\n' $status >$trace_dir/wait-status

test (count (string match -r "^event:$expected_status\$" < $trace_dir/events)) -eq 1
and test (cat $trace_dir/jobs-status) -eq 1
and not test -s $trace_dir/jobs
and test (cat $trace_dir/wait-status) -eq 0
and not test -s $trace_dir/wait-stderr
and begin; test "$argv[2]" != interrupt; or string match -q -- INT < $trace_dir/signal; end
or begin
    printf 'events='; cat $trace_dir/events
    printf 'jobs-status='; cat $trace_dir/jobs-status
    printf 'jobs='; cat $trace_dir/jobs
    printf 'wait-status='; cat $trace_dir/wait-status
    printf 'wait-stderr='; cat $trace_dir/wait-stderr
    return 1
end
