# CI Timeline Visualizer

Visualize GitHub Actions workflow runs as ASCII art to understand where time is spent.

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
# View a single workflow run
./timeline.py <run_id> --repo owner/repo

# Compare two runs (before/after)
./timeline.py <baseline_run> --diff <comparison_run> --repo owner/repo

# View history of multiple runs
./timeline.py --history <run1> <run2> <run3> --repo owner/repo
```

## Understanding the Output

### Timeline View

```
          | job-1       | job-2       | job-3       |
-------------------------------------------------------
       0s |#############|.............|.............|
          |#############|.............|.............|
          |#############|.........####|.........####|
      10m |#############|#############|#############|
          |             |#############|#############|
          |             |#############|#############|
      20m |             |             |#############|
```

| Symbol | Meaning |
|--------|---------|
| `...` | Queued - job is waiting for a runner |
| `###` | Running - job is executing (success) |
| `XXX` | Running - job failed |
| `---` | Job was cancelled |

### Key Metrics

- **Wall Clock Time**: Total time from workflow start to finish
- **Sum of Run Times**: Actual compute time across all jobs
- **Queue Time**: Time spent waiting for runners
- **Max Queue**: Longest single job waited for a runner

## Use Cases

### 1. Debugging Slow CI Runs

When a run seems slow, visualize it to see if the bottleneck is:
- A slow job (long `###` section)
- Runner availability (long `...` section at start)

```bash
./timeline.py 12345678 --repo myorg/myrepo
```

### 2. Comparing Before/After Changes

When evaluating CI improvements (like adding hurry caching):

```bash
./timeline.py <before_run> --diff <after_run> --repo myorg/myrepo
```

The output will show:
- Per-job timing changes
- Whether build time improved
- Whether queue time affected results

Example insight:
```
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

Output:
```
Run ID       | Wall Clock | Build Time | Queue Time |  Max Queue | Timeline
----------------------------------------------------------------------------
111          |     40m28s |       4h0m |        17s |         3s | [#####################    ]
222          |      37m7s |      3h55m |        20s |         5s | [####################     ]
333          |     56m55s |      3h44m |     44m40s |     22m16s | [.........###############]
```

## Options

| Option | Description |
|--------|-------------|
| `--repo OWNER/REPO` | GitHub repository (auto-detected if in a git repo) |
| `--diff RUN_ID` | Compare with another run |
| `--history RUN_ID...` | Show history view for multiple runs |
| `--pr PR_NUMBER` | Find runs for a pull request |
| `--commit SHA` | Find runs for a commit |
| `--width N` | Terminal width (default: 140) |
| `--height N` | Timeline height in rows (default: 50) |

## Tips

1. **Queue time is variable**: macOS runners often have longer queues than Linux
2. **Compare apples to apples**: When comparing runs, note the queue times
3. **Sum of run times is the true metric**: This shows actual compute time, unaffected by runner availability
4. **Look at the critical path**: The "Last job to complete" determines wall clock time
