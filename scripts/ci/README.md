# CI Timeline Visualizer

Visualize GitHub Actions workflow runs to understand where time is spent.

This tool helps answer questions like:
- "Why did this CI run take so long?"
- "Did our caching improvements actually help?"
- "How much time are we losing to runner queue delays?"

## Installation

Requires Python 3.7+ and the GitHub CLI (`gh`) authenticated.

```bash
# Make executable
chmod +x timeline.py

# Verify gh is authenticated
gh auth status
```

## Quick Start

```bash
# List runs to find run IDs
./timeline.py --list --pr 123 --repo owner/repo
./timeline.py --list --branch main --repo owner/repo

# View a single workflow run
./timeline.py <run_id> --repo owner/repo
./timeline.py --branch main --repo owner/repo  # most recent run

# Compare two runs (before/after)
./timeline.py <baseline_run> --diff <comparison_run> --repo owner/repo

# View history of multiple runs
./timeline.py --history <run1> <run2> <run3> --repo owner/repo
./timeline.py --history --branch main --repo owner/repo
./timeline.py --history --pr 123 --repo owner/repo
```

## Understanding the Output

### Single Run View

```
Job                                      Timeline              Queue     Run   Total
------------------------------------------------------------------------------------------------------------------------
aarch64-apple-darwin                     ░░░░░░░░████████████    22m    22m     44m
x86_64-apple-darwin                      ░░░░░░░░██████████████  22m    34m     56m
aarch64-unknown-linux-gnu                ████████████             2s    24m     24m
x86_64-unknown-linux-gnu                 ███████████              2s    23m     23m
------------------------------------------------------------------------------------------------------------------------

Wall clock:      56m55s    Sum of run times:       3h44m
Max queue:       22m16s    Sum of queue times:    44m40s

Activity:   ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁████████████████████████████████████████████
            0                        28m27s                           56m55s

Legend: █ running  ░ queued
```

| Symbol | Meaning |
|--------|---------|
| `█` | Job running |
| `░` | Job queued (waiting for runner) |
| `▁▂▃▄▅▆▇█` | Sparkline showing parallel job activity over time |

### Key Metrics

- Wall Clock Time: Total time from workflow start to finish
- Sum of Run Times: Actual compute time across all jobs
- Queue Time: Time spent waiting for runners
- Activity Sparkline: Shows how many jobs ran in parallel over time

## Use Cases

### 1. Debugging Slow CI Runs

When a run seems slow, visualize it to see if the bottleneck is:
- A slow job (long `█` section)
- Runner availability (long `░` section, low sparkline at start)

```bash
./timeline.py 12345678 --repo myorg/myrepo
```

### 2. Comparing Before/After Changes

When evaluating CI improvements (like adding hurry caching):

```bash
./timeline.py <before_run> --diff <after_run> --repo myorg/myrepo
```

The unified diff view shows:
- Per-job timing bars scaled to the same max duration
- Queue vs run time for each job in both runs
- Delta column showing build time changes
- KEY INSIGHT section explaining the results

Example output:
```
Job                          Run 1            Run 2            Delta
------------------------------------------------------------------------------------------------------------------------
aarch64-apple-darwin         ████████████ 40m ░░░░░░░░████ 44m    -18m
x86_64-apple-darwin          ██████████ 34m   ░░░░░░░░██████ 56m  ~same
------------------------------------------------------------------------------------------------------------------------

KEY INSIGHT:
  Run 2 saved 16m19s in actual build time across all jobs.
  BUT Run 2 had 44m23s MORE queue wait time.

  Despite faster builds, wall clock time increased due to runner queue delays!
```

### 3. Tracking CI Performance Over Time

Monitor trends across multiple runs:

```bash
./timeline.py --history 111 222 333 444 --repo myorg/myrepo
```

Output shows sparklines for each run, making it easy to spot queue delays:
```
Run ID       |    Wall |   Build |   Queue | Activity
------------------------------------------------------------------------------------------------------------------------
20359275612  |  40m28s |    4h0m |     17s | ████████████████████████████████████████
20360289566  |   37m7s |   3h55m |     20s | ███████████████████████████████████▁▁▁▁▁
20361281298  |  56m55s |   3h44m |  44m40s | ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁█████████████████████
------------------------------------------------------------------------------------------------------------------------

Activity: Height shows parallel job count over time
          Low blocks at start = queue delay; sustained height = good parallelism
```

## Options

| Option | Description |
|--------|-------------|
| `--repo OWNER/REPO` | GitHub repository (auto-detected if in a git repo) |
| `--list` | List runs instead of visualizing (combine with `--pr` or `--branch`) |
| `--branch NAME` | Find runs for a branch |
| `--pr PR_NUMBER` | Find runs for a pull request |
| `--commit SHA` | Find runs for a commit |
| `--diff RUN_ID` | Compare with another run |
| `--history RUN_ID...` | Show history view for multiple runs |
| `--limit N` | Number of runs to list (default: 10) |
| `--width N` | Terminal width (default: 120) |

## Tips

1. Queue time is variable: macOS runners often have longer queues than Linux
2. Compare apples to apples: When comparing runs, note the queue times
3. Sum of run times is the true metric: This shows actual compute time, unaffected by runner availability
4. Watch the sparkline: Low blocks at the start indicate queue delays; sustained high blocks show good parallelism
