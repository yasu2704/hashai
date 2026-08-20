set -l trace_dir $argv[1]
set -g __hashai_probe_trace_dir $trace_dir
command sh -c 'exit 0' &
set -l worker $last_pid

function __hashai_probe_exit --on-process-exit $worker
    printf 'event:%s\n' $argv[3] >>$__hashai_probe_trace_dir/events
    set -g __hashai_probe_status $argv[3]
end

while not set -q __hashai_probe_status
    sleep 0.01
end

jobs -p >$trace_dir/jobs
printf '%s\n' $status >$trace_dir/jobs-status
wait $worker 2>$trace_dir/wait-stderr
printf '%s\n' $status >$trace_dir/wait-status

test (count (string match -r '^event:0$' < $trace_dir/events)) -eq 1
and test (cat $trace_dir/jobs-status) -eq 1
and not test -s $trace_dir/jobs
and test (cat $trace_dir/wait-status) -eq 0
and not test -s $trace_dir/wait-stderr
