#!/bin/sh
# Samples Quay once per second; prints mean CPU %, idle wakeups per minute and last
# memory reading, for the app process and for its WebKit renderer.
# top reports IDLEW as a running total, so wakeups/min = (last - first) / seconds * 60.
#
# Two things make Quay's numbers easy to misread:
#  - The metrics and radar loops are visibility-gated, so popover-open and
#    popover-closed are different measurements. Say which one a number came from.
#  - Idle wakeups only mean something with the popover CLOSED. While it is open the
#    loops keep the process busy enough that it never idles, so IDLEW reads ~0 no
#    matter how much work is going on.
#
# Usage: scripts/cpu-check.sh [samples]        (default 600 = 10 minutes)
#        web=<pid> scripts/cpu-check.sh 60     (pin the renderer pid explicitly)
#
# The release bundle runs as `Quay`, `npm run tauri dev` runs as `quay`. Debug builds
# read several times higher — only compare like with like.
set -eu

# caffeinate: a display or idle sleep mid-run would truncate the sample, and holding
# the popover open for a measurement needs the screen awake anyway. Re-exec once.
if [ -z "${CPU_CHECK_AWAKE:-}" ]; then
	CPU_CHECK_AWAKE=1
	export CPU_CHECK_AWAKE
	exec caffeinate -di "$0" "$@"
fi

samples="${1:-600}"
pid="$(pgrep -nx Quay || pgrep -nx quay)" || { echo "Quay is not running" >&2; exit 1; }

# The webview renders in a separate WebKit process, so `top -pid <quay>` alone cannot
# see it — CSS animation cost, for one, lands entirely there. WebKit XPC services are
# reparented to launchd, so ownership is inferred from start time: the first
# WebContent to launch after Quay did. It is a heuristic, and it can pick another
# app's renderer if one started in the same window — pass `web=<pid>` to override.
epoch_of() { date -j -f '%a %b %d %T %Y' "$1" +%s 2>/dev/null || echo 0; }
if [ -z "${web:-}" ]; then
	app_started="$(epoch_of "$(ps -o lstart= -p "$pid")")"
	web=""
	best=0
	for candidate in $(pgrep -x com.apple.WebKit.WebContent); do
		started="$(epoch_of "$(ps -o lstart= -p "$candidate" 2>/dev/null)")"
		[ "$started" -ge "$app_started" ] || continue
		if [ "$best" -eq 0 ] || [ "$started" -lt "$best" ]; then best="$started"; web="$candidate"; fi
	done
fi

# One `top` per process, run concurrently — sequential runs would double the wall time.
sample_pid() {
	top -pid "$2" -stats cpu,idlew,mem -l "$((samples + 1))" -s 1 | awk -v label="$1" '
		$1 ~ /^[0-9.]+$/ {
			if (seen++) { cpu += $1; count++ } else { firstWakeups = $2 }
			lastWakeups = $2
			memory = $3
		}
		END {
			if (count == 0) { printf "%-11s no samples collected (did it exit?)\n", label ":"; exit 0 }
			sub(/[+-]$/, "", memory)
			printf "%-11s samples=%d mean_cpu=%.2f%% idle_wakeups_per_min=%.1f memory=%s\n",
				label ":", count, cpu / count, (lastWakeups - firstWakeups) / count * 60, memory
		}'
}

sample_pid quay "$pid" &
[ -n "$web" ] && sample_pid webcontent "$web" &
wait
