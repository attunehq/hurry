#!/usr/bin/env bash
# Example commands for ci/timeline.py
# These are meant to be copy-pasted, not run as a script

# =============================================================================
# LISTING RUNS (to find run IDs)
# =============================================================================

# List runs for a PR
./timeline.py --list --pr 123 --repo owner/repo

# List runs for a branch
./timeline.py --list --branch main --repo owner/repo

# List more runs (default is 10)
./timeline.py --list --branch main --limit 20 --repo owner/repo


# =============================================================================
# SINGLE RUN VISUALIZATION
# =============================================================================

# View a specific workflow run by ID
./timeline.py 20361281298 --repo owner/repo

# View most recent run on a branch
./timeline.py --branch main --repo owner/repo

# Wider output for large monitors
./timeline.py 20361281298 --repo owner/repo --width 160


# =============================================================================
# COMPARING TWO RUNS (BEFORE/AFTER)
# =============================================================================

# Compare baseline vs comparison run
# Great for: "Did my caching changes help?"
./timeline.py 20359275612 --diff 20361281298 --repo owner/repo

# The output shows:
# - Side-by-side job timings with deltas
# - Whether queue time or build time changed
# - A KEY INSIGHT explaining the results


# =============================================================================
# HISTORY VIEW (MULTIPLE RUNS)
# =============================================================================

# Track performance across multiple runs (by run IDs)
./timeline.py --history 20359275612 20360289566 20361281298 --repo owner/repo

# Track performance for recent runs on a branch (uses --limit, default 10)
./timeline.py --history --branch main --repo owner/repo

# Track performance for runs on a PR
./timeline.py --history --pr 123 --repo owner/repo

# Output shows sparklines indicating parallelism over time:
# Run ID       |    Wall |   Build |   Queue | Activity
# 20359275612  |  40m28s |    4h0m |     17s | ████████████████████████▇▇▆▅▄▄▄▄▄▄▄▄▃▂▂▂
# 20360289566  |   37m7s |   3h55m |     20s | ████████████████████████▇▇▆▅▄▄▄▄▄▄▄▄▃▂▂▂
# 20361281298  |  56m55s |   3h44m |  44m40s | ▆▆▆▆▆▆▆▆▆▆▆▆▆▆▆██▆▅▄▄▄▄▄▃▃▂▂▂▂▂▂▁▁▁▁▁▁▁▁


# =============================================================================
# FINDING RUNS
# =============================================================================

# Find runs for a PR
./timeline.py --pr 123 --repo owner/repo

# Find runs for a commit
./timeline.py --commit abc123def --repo owner/repo

# If you're in a git repo, --repo can be omitted
cd /path/to/repo
./timeline.py 20361281298


# =============================================================================
# REAL-WORLD SCENARIOS
# =============================================================================

# Scenario 1: "Why was this PR's CI so slow?"
./timeline.py --pr 456 --repo owner/repo

# Scenario 2: "Did enabling hurry caching help?"
# Run 1: before hurry (cold cache)
# Run 2: after hurry (warm cache)
./timeline.py <before_run_id> --diff <after_run_id> --repo owner/repo

# Scenario 3: "Show me the last 5 runs for this workflow"
# First, list recent runs:
gh run list --repo owner/repo --workflow "build.yml" --limit 5
# Then visualize them:
./timeline.py --history <id1> <id2> <id3> <id4> <id5> --repo owner/repo
