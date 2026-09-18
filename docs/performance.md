# Performance / energy

Quay sits in the menubar all day, so its cost while you are *not* looking at it is the
number that matters. This page says how to measure that and what the current readings
are.

## Measuring

```sh
scripts/cpu-check.sh 600     # 10 minutes, 1 Hz
scripts/cpu-check.sh 60      # quick check
web=<pid> scripts/cpu-check.sh 60   # pin the renderer pid if the heuristic picks wrong
```

It prints mean CPU %, idle wakeups per minute and the last memory reading for the app
process **and** its WebKit renderer, sampled concurrently. The script re-execs itself
under `caffeinate -di`, so a display or idle sleep can't truncate a run.

Three traps, all of which produce confidently wrong numbers:

- **`cpu-check.sh` cannot measure the screens-off state.** It re-execs under
  `caffeinate -di`, and the `-d` is exactly what prevents display sleep. To check the
  power gate, run the app with `QUAY_TRACE=1` and use `caffeinate -i` (no `-d`) plus
  `pmset displaysleepnow` — the trace lines are the evidence, not a CPU sample.
- **Popover open and closed are different measurements.** The metrics and radar loops
  are visibility-gated; closed, they block on a condvar and do nothing. Always say
  which state a number came from.
- **Idle wakeups only mean something with the popover closed.** While it's open the
  loops keep the process busy enough that it never idles, so `IDLEW` reads ~0 however
  much work is happening. Open-state cost shows up in CPU %, not wakeups.
- **The webview is a separate process.** `top -pid <quay>` cannot see it, so anything
  in the frontend — a CSS animation, for instance — is invisible unless you sample
  `com.apple.WebKit.WebContent` too. WebKit XPC services are reparented to launchd, so
  the script infers ownership from start time; pass `web=<pid>` when that guesses
  wrong.

Debug builds (`npm run tauri dev`) read several times higher than the release bundle.
Compare like with like.

## Current readings

Debug build, 2026-09-18, measured before and after the energy work (see
[architecture](architecture.md) for what the loops do).

| State | Build | quay CPU | idle wakeups/min |
|---|---|---|---|
| Popover closed | before | 1.13 % | 77.0 |
| Popover closed | **after** | **0.04 %** | **2.0** |
| Popover open | before | 1.08 % | n/a — never idles |
| Popover open | after | 1.10 % | n/a — never idles |

The renderer reads 0.00 % with the popover closed, which is the `animation-play-state`
pause doing its job (the popover is hidden, not unmounted, so its `animate-pulse` dots
would otherwise composite forever).

**Closed-state cost fell ~28× in CPU and ~38× in idle wakeups.** That came from:
batching the per-item `brew`/`docker` forks in the always-on poll loop, rate-limiting
the `ps`-forking orphan sweep, and replacing two 500 ms idle ticks with condvar waits.

Since then the agent pass has been rebuilt on hook state files: per pass it no longer
forks `ps`, no longer refreshes `sysinfo` over every tty-attached process twice, and no
longer sleeps 200 ms to get a CPU delta it then displayed. Re-measure before quoting a
number — the figures below predate it.

**Open-state cost is unchanged**, and a 40 s `sample` profile shows why nothing obvious
is left: every thread is parked in a kernel wait, and the real cost is the `lsof`/`ps`
forks, which land as system time in short-lived children rather than in Quay's own
stacks. Reducing it further means running the passes less often, which is what
`agentIntervalSec` and `metricsIntervalSec` are for. The popover is open for seconds a
day, so this is the right place to stop.

## The floor is conditional

The always-on loop parks entirely while every display is asleep or the Mac is locked
(see [architecture](architecture.md#power-gating)). Overnight, with the lid shut, Quay
does no work at all rather than ~1,200 status passes per hour. There is no knob for
this and nothing to tune — it is the default.

Verify it rather than assume it:

```sh
QUAY_TRACE=1 npm run tauri dev
caffeinate -i -t 120 &     # keep the system awake, let the display sleep
pmset displaysleepnow
sleep 12
caffeinate -u -t 3         # simulate user activity to wake the display back up
```

Expect five lines. On a Mac set to require a password, display sleep also **locks** the
session, so the two signals interleave — and the middle of the sequence is the part
worth reading:

```
power gate: reconciled     — polling running    startup state, read not assumed
power gate: screens asleep — polling parked
power gate: locked         — polling parked
power gate: screens awake  — polling parked     screens back, but still locked
power gate: unlocked       — polling running    both clear, so work resumes
```

The fourth line is the whole point of keeping the two flags independent: the display
came back before the lock cleared, and polling correctly stayed parked. If your Mac
does not lock on display sleep you will see three lines instead, which is also correct.

Two honest limits: gating the *next* iteration does not cancel a pass already in
flight, so what this buys is eventual quiescence rather than an instant stop; and
resume is bounded by notification delivery, not instantaneous.

## If you need it lower still

In rough order of effect, all in [configuration](configuration.md):

1. **`pollIntervalSec`** — the only always-on loop, and only while a screen is lit.
   Raising it is the one change that affects battery over a whole day.
2. **`trackAgents` off** — skips the agent pass outright (no `ps`, no sysinfo, no
   session-file reads) and clears the tray badge.
3. **`agentIntervalSec` / `metricsIntervalSec`** — popover-open only, so they change
   what a glance costs, not what idling costs.
