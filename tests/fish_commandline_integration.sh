#!/usr/bin/env bash
set -Eeuo pipefail
trap 'printf "Fish integration failed at line %s: %s\n" "$LINENO" "$BASH_COMMAND" >&2' ERR
: "${HASHAI_BIN:?}"; : "${HASHAI_FISH_BIN:=fish}"
source tests/shell_contract_cases.sh
fish_pty=$PWD/tests/fish_pty.py
inject_unsupported_call() {
    awk '/set -l core_status/ { print "    while jobs -p | string match -qx -- \"$worker\""; print "        wait $worker"; print "    end" } { print }' "$1"
}
"$HASHAI_FISH_BIN" --version | grep -Eq 'fish, version ([4-9]|3\.[6-9])'
d=$(mktemp -d); d=$(cd "$d" && pwd -P); trap 'rm -rf "$d"' EXIT; mkdir "$d/tmp"; export XDG_DATA_HOME="$d/data" XDG_CONFIG_HOME="$d/config"
mkdir "$d/process-exit-probe"
"$HASHAI_FISH_BIN" --no-config tests/fish_process_exit_probe.fish "$d/process-exit-probe"
"$HASHAI_BIN" integration install --shell fish --trigger '@@ ' --keybinding ctrl-x >/dev/null; a="$XDG_DATA_HOME/hashai/integrations/hashai.fish"
test "$(grep -Fc 'string match -qx -- "$worker"' "$a")" -eq 0
test "$(grep -Fc 'disown $worker' "$a")" -eq 0
cat >"$d/fake-codex" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case ${1:-} in
    --version) echo 'codex 9.9.9' ;;
    login)
        if [[ ${2:-} == --help ]]; then echo status; else echo 'logged in'; fi
        ;;
    exec)
        if [[ ${2:-} == --help ]]; then
            echo 'exec --ephemeral --ignore-user-config --ignore-rules --model --config --sandbox --disable --skip-git-repo-check --output-schema --output-last-message'
            exit 0
        fi
        out=
        while (($#)); do
            if [[ $1 == --output-last-message ]]; then out=$2; shift 2; else shift; fi
        done
        cat >/dev/null
        printf '%s' '{"command":"printf ok","risk":"safe"}' >"$out"
        ;;
    *) exit 1 ;;
esac
EOF
chmod +x "$d/fake-codex"
HASHAI_CODEX_BIN="$d/fake-codex" HASHAI_DOCTOR_FISH_BIN="$HASHAI_FISH_BIN" HASHAI_TRIGGER='@@ ' HASHAI_KEYBINDING=ctrl-x \
    "$HASHAI_BIN" doctor --live --format json --shell fish >"$d/doctor.json"
python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); c={x["id"]:x for x in r["checks"]}; assert r["schema_version"] == 2, r; assert c["integration.artifact"]["message"] == "current", r; assert c["integration.startup_loader"]["status"] == "PASS", r; assert c["integration.startup_activation"]["message"] == "current-and-active" and c["integration.startup_activation"]["status"] == "PASS", r' "$d/doctor.json"
# A prior tracked artifact may contain the broken call while still matching its
# manifest. The public update path must replace it with the current embedding.
inject_unsupported_call "$a" >"$d/prior.fish"
mv "$d/prior.fish" "$a"
python3 -c 'import hashlib,json,sys; artifact,manifest=sys.argv[1:]; data=json.load(open(manifest)); data["artifact"]["sha256"]=hashlib.sha256(open(artifact,"rb").read()).hexdigest(); open(manifest,"w").write(json.dumps(data,indent=2)+"\n")' "$a" "$XDG_DATA_HOME/hashai/integrations/fish.manifest.json"
"$HASHAI_BIN" integration update --trigger '@@ ' --keybinding ctrl-x >/dev/null
test "$(grep -Fc 'string match -qx -- "$worker"' "$a")" -eq 0
injection_marker="/tmp/hashai-trigger-injection-$$"
rm -f "$injection_marker"
trap 'rm -rf "$d"; rm -f "$injection_marker"' EXIT
# shellcheck disable=SC1003 # literal quote/substitution corpus values
for corpus_trigger in "'" '"' '\\' "\$(touch '$injection_marker')" "\`touch '$injection_marker'\`" ';' $'\t' '日本語' '😀' ' leading' 'trailing '; do
    "$HASHAI_BIN" integration install --shell fish --trigger "$corpus_trigger" --keybinding ctrl-x >/dev/null
    "$HASHAI_FISH_BIN" -n "$a"
    printf '%s' "$corpus_trigger" >"$d/expected-trigger"
    env -u HASHAI_TRIGGER "$HASHAI_FISH_BIN" --no-config -c \
        'source $argv[1]; printf %s $__hashai_fish_trigger' -- "$a" >"$d/actual-trigger"
    cmp -s "$d/expected-trigger" "$d/actual-trigger"
    test ! -e "$injection_marker"
done
"$HASHAI_BIN" integration install --shell fish --trigger '@@ ' --keybinding ctrl-x >/dev/null
mkdir "$d/bin"
write_shell_contract_fake "$d/bin" fish
printf '%s\n' '#!/usr/bin/env bash' "printf '%s' \$'# first natural language\\n日本語 😀 second line' >\"\$1\"" >"$d/editor"; chmod +x "$d/editor"
run() { local mode=$1; local b=$2; local c=$3; local map=${4:-default}; local trigger=${5:-'# '}; local artifact=${6:-$a}; local setup_binding=; local mode_setup= length moves=; local -a progress_env=(); [[ ${7:-} == disabled ]] || setup_binding="bind -M $map \\cx __hashai_fish_replace_buffer"; [[ $map == insert ]] && mode_setup=$'fish_vi_key_bindings\nset -g fish_bind_mode insert'; length=$("$HASHAI_FISH_BIN" -c 'string length -- "$argv[1]"' -- "$b"); while (( length > c )); do moves+=$'\e[D'; ((length--)); done; : >"$d/request"; : >"$d/worker-trace"; if [[ $mode == blocking || $mode == interruptible ]]; then progress_env+=(HASHAI_PROGRESS_RELEASE_FILE="$d/progress-release"); rm -f "$d/progress-release"; fi; if [[ $mode == interruptible ]]; then progress_env+=(HASHAI_PROGRESS_CANCEL=1 HASHAI_SIGNAL_FILE="$d/signal-relay"); : >"$d/signal-relay"; fi; cat >"$d/cmd" <<EOF
$mode_setup
$setup_binding
source '$artifact'
source '$artifact'
bind -M default \\cx >'$d/binding.default'
bind -M insert \\cx >'$d/binding.insert'
functions -c __hashai_fish_replace_buffer __hashai_fish_real
function __hashai_fish_replace_buffer; set -l raw (commandline --current-buffer | string collect -N); string match -rq '^(?<exposed>(?s:.*))\\n\\z' -- "\$raw"; printf %s "\$exposed" >'$d/exposed'; __hashai_fish_real; jobs -p >'$d/jobs'; echo '__HASHAI_FISH_'READY__ >&2; end
function __fish_capture; set -l raw (commandline | string collect -N); string match -rq '^(?<captured>(?s:.*))\\n\\z' -- "\$raw"; printf %s "\$captured" >'$d/buffer'; commandline --cursor >'$d/cursor'; commandline -r exit; commandline -f execute; end
bind \\ct __fish_capture
bind -M insert \\ct __fish_capture
function __fish_edit_buffer; edit_command_buffer; echo '__HASHAI_FISH_'READY__ >&2; end
bind \\cy __fish_edit_buffer
echo '__HASHAI_FISH_'READY__
EOF
if [[ $b == __NORMALIZED_MULTILINE__ ]]; then printf '\031\0\030\0\024' >>"$d/cmd"; else printf '%s%s\030\0\024' "$b" "$moves" >>"$d/cmd"; fi
if ! (cd "$d" && env ${progress_env[@]+"${progress_env[@]}"} TERM=xterm-256color LANG=C.UTF-8 TMPDIR="$d/tmp" PATH="$d/bin:$PATH" HASHAI_EXPECTED_SHELL=fish HASHAI_TEST_MODE="$mode" HASHAI_REQUEST_FILE="$d/request" HASHAI_WORKER_TRACE_FILE="$d/worker-trace" HASHAI_TRIGGER="$trigger" HASHAI_KEYBINDING=ctrl-g HASHAI_AUTOEXEC_MARKER="$d/auto" VISUAL="$d/editor" EDITOR="$d/editor" HASHAI_FISH_BIN="$HASHAI_FISH_BIN" python3 "$fish_pty" "$d/cmd") >"$d/log"; then
    cat "$d/log" >&2
    return 1
fi
test ! -s "$d/jobs"
if [[ -s $d/worker-trace ]]; then
    read -r worker_pid worker_pgid <"$d/worker-trace"
fi
for _cleanup_probe in {1..40}; do
    leaked_temp=$(find "$d/tmp" -maxdepth 1 -name 'hashai-*' -print)
    worker_alive=0; group_alive=0
    if [[ -s $d/worker-trace ]]; then
        if kill -0 "$worker_pid" 2>/dev/null; then worker_alive=1; fi
        if pgrep -g "$worker_pgid" >/dev/null 2>&1; then group_alive=1; fi
    fi
    if [[ -z $leaked_temp && $worker_alive -eq 0 && $group_alive -eq 0 ]]; then break; fi
    sleep 0.05
done
if [[ -n $leaked_temp ]]; then printf 'Fish temp file leaked: %s\n' "$leaked_temp" >&2; cat "$d/log" >&2; return 1; fi
if [[ $worker_alive -ne 0 ]]; then printf 'Fish worker %s remains after widget return\n' "$worker_pid" >&2; return 1; fi
if [[ $group_alive -ne 0 ]]; then printf 'Fish worker process group %s remains after widget return\n' "$worker_pgid" >&2; ps -eo pid,ppid,pgid,stat,command | awk -v pgid="$worker_pgid" '$3 == pgid'; return 1; fi
}
# Fish's editor normalizes literal tabs before widget dispatch; the shared
# fixture documents the raw request while this PTY supplement tests exposed bytes.
original=$HASHAI_CONTRACT_FISH_EXPOSED_REQUEST
# The generated artifact bakes `@@ `/Ctrl-X; `run` supplies a `# ` runtime
# trigger and sends literal Ctrl-X, proving the enabled compatibility override.
run success "$original" 5 default; printf '%s' "$HASHAI_CONTRACT_SUCCESS" >"$d/expected"; cmp "$d/expected" "$d/buffer"; "$HASHAI_FISH_BIN" -c "string length -- \"$HASHAI_CONTRACT_SUCCESS\"" >"$d/cursor-expected"; cmp "$d/cursor-expected" "$d/cursor"; tail -c +3 "$d/exposed" >"$d/expected-request"; cmp "$d/expected-request" "$d/request"
if grep -F 'string match:' "$d/log" >/dev/null; then
    printf 'safe success leaked a Fish string-match diagnostic\n' >&2
    exit 1
fi
if grep -E 'unknown option| has ended|hashai: warning:|fake core|command generation failed' "$d/log" >/dev/null; then printf 'safe success stderr exceeded its allowlist\n' >&2; exit 1; fi
unsupported_mutated="$d/hashai.unsupported-mutated.fish"
inject_unsupported_call "$a" >"$unsupported_mutated"
test "$(grep -Fc 'string match -qx -- "$worker"' "$unsupported_mutated")" -eq 1
run success "$original" 5 default '# ' "$unsupported_mutated"
grep -F 'string match:' "$d/log" >/dev/null
# AC-1/AC-5: review and dangerous stderr warnings are visible through the
# real Fish PTY while command replacement, cursor, request, and no-auto hold.
for mode in review dangerous; do
    run "$mode" "$original" 5 default
    cmp "$d/expected" "$d/buffer"
    cmp "$d/cursor-expected" "$d/cursor"
    cmp "$d/expected-request" "$d/request"
    case $mode in
        review) test "$(grep -Fc "$HASHAI_CONTRACT_REVIEW_WARNING" "$d/log")" -eq 1 ;;
        dangerous) test "$(grep -Fc "$HASHAI_CONTRACT_DANGEROUS_WARNING" "$d/log")" -eq 1 ;;
    esac
    if grep -E 'string match:|unknown option| has ended|command generation failed' "$d/log" >/dev/null; then printf '%s stderr exceeded its allowlist\n' "$mode" >&2; exit 1; fi
    test ! -e "$d/auto"
done
# Fish owns editor normalization before this contract boundary; request bytes
# are compared to the captured exposed buffer above.
run success __NORMALIZED_MULTILINE__ 0 default; printf '%s' $'# first natural language\n日本語 😀 second line' >"$d/expected-exposed"; cmp "$d/expected-exposed" "$d/exposed"; printf '%s' $'first natural language\n日本語 😀 second line' >"$d/expected-request"; cmp "$d/expected-request" "$d/request"
run success "$original" 5 insert; cmp "$d/expected" "$d/buffer"
grep -F '__hashai_fish_replace_buffer' "$d/binding.default" "$d/binding.insert" >/dev/null
run multiline "$original" 5 default; printf '%s' "$HASHAI_CONTRACT_MULTILINE" >"$d/expected"; cmp "$d/expected" "$d/buffer"
run noauto "$original" 5 default; test ! -e "$d/auto"
# AC-1: the blocking fake is released only after two ordered suffix frames.
run blocking "$original" 5 default
test -e "$d/progress-release"
printf '%s' "$HASHAI_CONTRACT_SUCCESS" >"$d/expected"
cmp "$d/expected" "$d/buffer"
grep -F 'generating…' "$d/log" >/dev/null
# AC-6: literal Ctrl-C relays exactly one SIGINT and restores editor state.
run interruptible "$original" 5 default
printf '%s' "$original" >"$d/expected"
cmp "$d/expected" "$d/buffer"
cmp <(printf '5\n') "$d/cursor"
test "$(grep -c '^INT$' "$d/signal-relay")" -eq 1
python3 -c 'import sys; data=open(sys.argv[1],"rb").read(); core=b"fake core cancelled"; generic=b"hashai: command generation failed; input preserved"; assert data.count(core)==1 and data.count(generic)==1 and data.index(core)<data.index(generic), data' "$d/log"
# Disabled generation does not install either editor-mode Ctrl-X binding;
# literal Ctrl-X followed by the separate Ctrl-T capture leaves state intact
# and never reaches the fake Core.
"$HASHAI_BIN" integration install --shell fish --keybinding ctrl-x --disable-trigger >/dev/null
run success "$original" 5 default '# ' "$a" disabled; printf '%s' "$original" >"$d/expected"; cmp "$d/expected" "$d/buffer"; cmp <(printf '5\n') "$d/cursor"; test ! -s "$d/request"
if grep -F '__hashai_fish_replace_buffer' "$d/binding.default" "$d/binding.insert" >/dev/null; then printf 'disabled Fish artifact installed Ctrl-X\n' >&2; exit 1; fi
"$HASHAI_BIN" integration install --shell fish --trigger '@@ ' --keybinding ctrl-x >/dev/null
run success 'echo untouched' 3 default; printf '%s' 'echo untouched' >"$d/expected"; cmp "$d/expected" "$d/buffer"; test ! -s "$d/request"
 # dispatch permutation mutation: installed literal Ctrl-G keeps the exposed input.
dispatch_mutated="$d/hashai.dispatch-mutated.fish"
test "$(grep -Fc -- '--shell fish' "$a")" -eq 1
sed 's/--shell fish/--shell bash/' "$a" >"$dispatch_mutated"
test "$(grep -Fc -- '--shell fish' "$dispatch_mutated")" -eq 0
test "$(grep -Fc -- '--shell bash' "$dispatch_mutated")" -eq 1
run success "$original" 5 default '# ' "$dispatch_mutated"; printf '%s' "$original" >"$d/expected"; cmp "$d/expected" "$d/buffer"; cmp <(printf '5\n') "$d/cursor"; test ! -s "$d/request"
for mode in failure timeout cancel empty malformed status-{1..9}; do run "$mode" "$original" 5 default; printf '%s' "$original" >"$d/expected"; cmp "$d/expected" "$d/buffer"; cmp <(printf '5\n') "$d/cursor"; [[ $mode == empty || $mode == malformed ]] || grep -F 'command generation failed' "$d/log"; done
run failure "$original" 5 default
python3 -c 'import sys; data=open(sys.argv[1],"rb").read(); core=b"fake core failure"; generic=b"hashai: command generation failed; input preserved"; assert data.count(core)==1 and data.count(generic)==1 and data.index(core)<data.index(generic), data' "$d/log"
run success ',, 日本語 😀' 2 default ',, '; printf '%s' "printf '日本語 😀  spaced'" >"$d/expected"; cmp "$d/expected" "$d/buffer"
# AC-5: sourcing noninteractive or interactive-without-a-TTY never invokes Core.
: >"$d/request"
PATH="$d/bin:$PATH" HASHAI_EXPECTED_SHELL=fish HASHAI_REQUEST_FILE="$d/request" "$HASHAI_FISH_BIN" --no-config -c "source '$a'"
test ! -s "$d/request"
: >"$d/request"
PATH="$d/bin:$PATH" HASHAI_EXPECTED_SHELL=fish HASHAI_REQUEST_FILE="$d/request" "$HASHAI_FISH_BIN" --no-config --interactive <<EOF >/dev/null 2>&1
source '$a'
set -g __hashai_fish_enabled 0
__hashai_fish_replace_buffer
EOF
test ! -s "$d/request"
mutated="$d/hashai.success-mutated.fish"
sed 's/commandline --replace -- "\$generated"/commandline --replace -- corrupted/' "$a" >"$mutated"
test "$(grep -Fc 'commandline --replace -- corrupted' "$mutated")" -eq 1
run success "$original" 5 default '# ' "$mutated"; printf '%s' "printf '日本語 😀  spaced'" >"$d/expected"; if cmp -s "$d/expected" "$d/buffer"; then printf 'success mutation was not detected\n' >&2; exit 1; fi
failure_mutated="$d/hashai.failure-mutated.fish"
sed "s/echo 'hashai: command generation failed; input preserved' >&2/commandline --replace -- corrupted; commandline --cursor 0; echo 'hashai: command generation failed; input preserved' >&2/" "$a" >"$failure_mutated"
test "$(grep -Fc 'commandline --replace -- corrupted; commandline --cursor 0' "$failure_mutated")" -eq 1
run failure "$original" 5 default '# ' "$failure_mutated"; printf '%s' "$original" >"$d/expected"; if cmp -s "$d/expected" "$d/buffer"; then printf 'failure mutation was not detected\n' >&2; exit 1; fi
printf 'Fish commandline PTY integration checks passed.\n'
