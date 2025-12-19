#!/usr/bin/env python3
"""
CI Timeline Visualizer

Visualizes GitHub Actions workflow runs as ASCII art, showing:
- Parallel jobs on X axis
- Time on Y axis
- Queue wait time (dots) vs actual execution time (hashes)
- Side-by-side comparison of two runs

Usage:
    # View a single run
    ./ci-timeline.py <run_id> [--repo owner/repo]

    # View runs for a PR or commit
    ./ci-timeline.py --pr <pr_number> [--repo owner/repo]
    ./ci-timeline.py --commit <sha> [--repo owner/repo]

    # Compare two runs side-by-side (great for before/after comparisons)
    ./ci-timeline.py <baseline_run_id> --diff <comparison_run_id> --repo owner/repo

    # Show history of multiple runs
    ./ci-timeline.py --history <run1> <run2> <run3> --repo owner/repo

Example:
    # Compare a "cold cache" run vs "warm cache" run to see queue vs build time
    ./ci-timeline.py <run_id_1> --diff <run_id_2> --repo owner/repo

Legend:
    ... (dots)   = Job queued, waiting for runner
    ### (hashes) = Job running
    XXX          = Job failed
    --- (dashes) = Job cancelled
"""

import argparse
import json
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Optional, Union, List, Dict


@dataclass
class Job:
    name: str
    status: str
    conclusion: Optional[str]
    created_at: datetime
    started_at: Optional[datetime]
    completed_at: Optional[datetime]

    @property
    def queue_duration_seconds(self) -> float:
        if self.started_at:
            return (self.started_at - self.created_at).total_seconds()
        return 0

    @property
    def run_duration_seconds(self) -> float:
        if self.started_at and self.completed_at:
            return (self.completed_at - self.started_at).total_seconds()
        return 0

    @property
    def total_duration_seconds(self) -> float:
        if self.completed_at:
            return (self.completed_at - self.created_at).total_seconds()
        return 0


@dataclass
class WorkflowRun:
    id: int
    name: str
    status: str
    conclusion: Optional[str]
    created_at: datetime
    started_at: Optional[datetime]
    updated_at: datetime
    jobs: List[Job]


def run_gh(args: List[str]) -> Union[dict, list]:
    """Run gh CLI command and return parsed JSON."""
    cmd = ["gh"] + args + ["--json"]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"Error running gh: {result.stderr}", file=sys.stderr)
        sys.exit(1)
    # gh --json returns the fields we request, need to figure out the right invocation
    return json.loads(result.stdout) if result.stdout.strip() else {}


def run_gh_api(endpoint: str, repo: str) -> Union[dict, list]:
    """Run gh api command."""
    cmd = ["gh", "api", f"repos/{repo}/{endpoint}"]
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"Error running gh api: {result.stderr}", file=sys.stderr)
        sys.exit(1)
    return json.loads(result.stdout) if result.stdout.strip() else {}


def parse_datetime(s: Optional[str]) -> Optional[datetime]:
    """Parse ISO datetime string."""
    if not s:
        return None
    # Handle Z suffix
    if s.endswith('Z'):
        s = s[:-1] + '+00:00'
    return datetime.fromisoformat(s)


def get_run_details(run_id: int, repo: str) -> WorkflowRun:
    """Fetch workflow run and its jobs."""
    # Get run info
    run_data = run_gh_api(f"actions/runs/{run_id}", repo)

    # Get jobs for this run
    jobs_data = run_gh_api(f"actions/runs/{run_id}/jobs", repo)

    jobs = []
    for j in jobs_data.get("jobs", []):
        jobs.append(Job(
            name=j["name"],
            status=j["status"],
            conclusion=j.get("conclusion"),
            created_at=parse_datetime(j.get("created_at") or run_data.get("created_at")),
            started_at=parse_datetime(j.get("started_at")),
            completed_at=parse_datetime(j.get("completed_at")),
        ))

    return WorkflowRun(
        id=run_data["id"],
        name=run_data["name"],
        status=run_data["status"],
        conclusion=run_data.get("conclusion"),
        created_at=parse_datetime(run_data["created_at"]),
        started_at=parse_datetime(run_data.get("run_started_at")),
        updated_at=parse_datetime(run_data["updated_at"]),
        jobs=jobs,
    )


def get_runs_for_pr(pr_number: int, repo: str) -> List[int]:
    """Get all workflow run IDs for a PR."""
    # Get check runs for the PR's head SHA
    pr_data = run_gh_api(f"pulls/{pr_number}", repo)
    head_sha = pr_data["head"]["sha"]
    return get_runs_for_commit(head_sha, repo)


def get_runs_for_commit(sha: str, repo: str) -> List[int]:
    """Get all workflow run IDs for a commit."""
    runs_data = run_gh_api(f"actions/runs?head_sha={sha}", repo)
    return [r["id"] for r in runs_data.get("workflow_runs", [])]


def format_duration(seconds: float) -> str:
    """Format seconds as human-readable duration."""
    if seconds < 60:
        return f"{int(seconds)}s"
    elif seconds < 3600:
        mins = int(seconds // 60)
        secs = int(seconds % 60)
        return f"{mins}m{secs}s"
    else:
        hours = int(seconds // 3600)
        mins = int((seconds % 3600) // 60)
        return f"{hours}h{mins}m"


def render_timeline(run: WorkflowRun, width: int = 120, height: int = 40) -> str:
    """Render a workflow run as ASCII timeline."""
    if not run.jobs:
        return "No jobs found"

    # Filter to jobs that have timing info
    jobs = [j for j in run.jobs if j.started_at and j.completed_at]
    if not jobs:
        return "No completed jobs with timing info"

    # Find time bounds
    min_time = min(j.created_at for j in jobs)
    max_time = max(j.completed_at for j in jobs)
    total_seconds = (max_time - min_time).total_seconds()

    if total_seconds == 0:
        return "All jobs completed instantly"

    # Calculate column widths
    # Reserve space for time labels on left
    time_label_width = 10
    # Each job gets a column
    num_jobs = len(jobs)
    job_col_width = max(3, (width - time_label_width - num_jobs - 1) // num_jobs)

    # Truncate job names to fit
    def truncate(s: str, max_len: int) -> str:
        if len(s) <= max_len:
            return s
        return s[:max_len-2] + ".."

    # Sort jobs by start time
    jobs = sorted(jobs, key=lambda j: j.started_at)

    # Build the visualization
    lines = []

    # Header with workflow info
    lines.append(f"{'=' * width}")
    lines.append(f"Workflow: {run.name} (Run #{run.id})")
    lines.append(f"Status: {run.status} / {run.conclusion or 'in progress'}")
    lines.append(f"Total duration: {format_duration(total_seconds)}")
    lines.append(f"{'=' * width}")
    lines.append("")

    # Job name header
    header = " " * time_label_width + "|"
    for j in jobs:
        # Extract short name (last part after /)
        short_name = j.name.split("/")[-1].strip()
        # Further shorten common patterns
        short_name = short_name.replace("build (", "").replace(")", "")
        short_name = short_name.replace("ubuntu-22.04, ", "").replace("macos-14, ", "mac:")
        short_name = short_name.replace("windows-2022, ", "win:")
        short_name = short_name.replace("-unknown-linux-", "-linux-")
        short_name = short_name.replace("-apple-darwin", "-darwin")
        short_name = short_name.replace("-pc-windows-msvc", "-win")
        header += truncate(short_name, job_col_width - 1).center(job_col_width) + "|"
    lines.append(header)
    lines.append("-" * len(header))

    # Create time grid
    seconds_per_row = total_seconds / height

    for row in range(height):
        row_time = min_time.timestamp() + (row * seconds_per_row)
        row_end_time = row_time + seconds_per_row

        # Time label (show every 5 rows)
        if row % 5 == 0:
            elapsed = row * seconds_per_row
            time_label = format_duration(elapsed).rjust(time_label_width - 1) + " "
        else:
            time_label = " " * time_label_width

        line = time_label + "|"

        for j in jobs:
            created_ts = j.created_at.timestamp()
            started_ts = j.started_at.timestamp() if j.started_at else created_ts
            completed_ts = j.completed_at.timestamp() if j.completed_at else started_ts

            # Determine what to show in this cell
            cell = " " * (job_col_width - 1)

            # Check if this row intersects with the job's timeline
            if row_time < completed_ts and row_end_time > created_ts:
                if row_end_time <= started_ts:
                    # Queue time (waiting for runner)
                    cell = ("." * (job_col_width - 1))
                elif row_time >= started_ts:
                    # Running
                    if j.conclusion == "success":
                        cell = ("#" * (job_col_width - 1))
                    elif j.conclusion == "failure":
                        cell = ("X" * (job_col_width - 1))
                    elif j.conclusion == "cancelled":
                        cell = ("-" * (job_col_width - 1))
                    else:
                        cell = ("?" * (job_col_width - 1))
                else:
                    # Transition from queue to running
                    # Show partial
                    queue_portion = (started_ts - row_time) / seconds_per_row
                    run_portion = 1 - queue_portion
                    queue_chars = int(queue_portion * (job_col_width - 1))
                    run_chars = job_col_width - 1 - queue_chars
                    char = "#" if j.conclusion == "success" else "X" if j.conclusion == "failure" else "?"
                    cell = "." * queue_chars + char * run_chars

            line += cell + "|"

        lines.append(line)

    # Footer with end time
    footer_time = format_duration(total_seconds).rjust(time_label_width - 1) + " "
    lines.append("-" * len(header))
    lines.append(footer_time + "|" + " " * (len(header) - time_label_width - 2) + "|")

    # Legend
    lines.append("")
    lines.append("Legend:")
    lines.append("  ... = Queued (waiting for runner)")
    lines.append("  ### = Running (success)")
    lines.append("  XXX = Running (failed)")
    lines.append("  --- = Cancelled")
    lines.append("")

    # Job summary table
    lines.append("Job Details:")
    lines.append("-" * 80)
    lines.append(f"{'Job':<40} {'Queue':>8} {'Run':>8} {'Total':>8} {'Status':>10}")
    lines.append("-" * 80)

    for j in jobs:
        short_name = j.name.split("/")[-1].strip()[:38]
        queue = format_duration(j.queue_duration_seconds)
        run = format_duration(j.run_duration_seconds)
        total = format_duration(j.total_duration_seconds)
        status = j.conclusion or j.status
        lines.append(f"{short_name:<40} {queue:>8} {run:>8} {total:>8} {status:>10}")

    lines.append("-" * 80)

    # Summary stats
    total_queue = sum(j.queue_duration_seconds for j in jobs)
    total_run = sum(j.run_duration_seconds for j in jobs)
    max_queue = max(j.queue_duration_seconds for j in jobs)

    lines.append("")
    lines.append(f"Total queue time across all jobs: {format_duration(total_queue)}")
    lines.append(f"Total run time across all jobs: {format_duration(total_run)}")
    lines.append(f"Max queue wait (single job): {format_duration(max_queue)}")
    lines.append(f"Wall clock time: {format_duration(total_seconds)}")

    # Critical path analysis
    lines.append("")
    lines.append("Critical Path Analysis:")
    # Find the job that finished last
    last_job = max(jobs, key=lambda j: j.completed_at)
    lines.append(f"  Last job to complete: {last_job.name.split('/')[-1].strip()}")
    lines.append(f"    Queue wait: {format_duration(last_job.queue_duration_seconds)}")
    lines.append(f"    Run time: {format_duration(last_job.run_duration_seconds)}")

    return "\n".join(lines)


def render_history(runs: List[WorkflowRun], width: int = 120) -> str:
    """Render a historical view of multiple runs showing trends."""
    lines = []
    lines.append("=" * width)
    lines.append("RUN HISTORY (oldest to newest)")
    lines.append("=" * width)
    lines.append("")

    # Sort runs by time
    runs = sorted(runs, key=lambda r: r.created_at)

    # Header
    lines.append(f"{'Run ID':<12} | {'Wall Clock':>10} | {'Build Time':>10} | {'Queue Time':>10} | {'Max Queue':>10} | Timeline")
    lines.append("-" * width)

    max_wall = max((
        (max(j.completed_at for j in r.jobs if j.completed_at) - min(j.created_at for j in r.jobs if j.completed_at)).total_seconds()
        for r in runs if r.jobs
    ), default=1)

    for run in runs:
        if not run.jobs:
            continue
        jobs = [j for j in run.jobs if j.completed_at]
        if not jobs:
            continue

        min_t = min(j.created_at for j in jobs)
        max_t = max(j.completed_at for j in jobs)
        wall_time = (max_t - min_t).total_seconds()

        build_time = sum(j.run_duration_seconds for j in jobs)
        queue_time = sum(j.queue_duration_seconds for j in jobs)
        max_queue = max(j.queue_duration_seconds for j in jobs)

        # Mini timeline bar
        bar_width = 40
        build_chars = int((build_time / 8 / max_wall) * bar_width) if max_wall > 0 else 0  # /8 for 8 parallel jobs
        queue_chars = int((max_queue / max_wall) * bar_width) if max_wall > 0 else 0

        bar = "." * min(queue_chars, bar_width)
        remaining = bar_width - len(bar)
        bar += "#" * min(build_chars, remaining)
        bar = bar[:bar_width].ljust(bar_width)

        lines.append(f"{run.id:<12} | {format_duration(wall_time):>10} | {format_duration(build_time):>10} | {format_duration(queue_time):>10} | {format_duration(max_queue):>10} | [{bar}]")

    lines.append("-" * width)
    lines.append("")
    lines.append("Legend: [....######] = queue time (dots) + build time (hashes)")
    lines.append("        Build time is sum across all jobs; queue time is max single job wait")

    return "\n".join(lines)


def render_comparison(runs: List[WorkflowRun], width: int = 120) -> str:
    """Render a comparison of multiple workflow runs."""
    lines = []
    lines.append("=" * width)
    lines.append("WORKFLOW RUN COMPARISON")
    lines.append("=" * width)

    # Group by workflow name
    by_workflow: Dict[str, List[WorkflowRun]] = {}
    for run in runs:
        by_workflow.setdefault(run.name, []).append(run)

    for workflow_name, workflow_runs in by_workflow.items():
        lines.append("")
        lines.append(f"Workflow: {workflow_name}")
        lines.append("-" * 60)

        for run in workflow_runs:
            if not run.jobs:
                continue
            jobs = [j for j in run.jobs if j.completed_at]
            if not jobs:
                continue

            min_time = min(j.created_at for j in jobs)
            max_time = max(j.completed_at for j in jobs)
            total = (max_time - min_time).total_seconds()

            max_queue = max((j.queue_duration_seconds for j in jobs), default=0)

            lines.append(f"  Run #{run.id}: {format_duration(total)} total, {format_duration(max_queue)} max queue")

    return "\n".join(lines)


def render_side_by_side(run1: WorkflowRun, run2: WorkflowRun, width: int = 140) -> str:
    """Render two runs side by side for comparison."""
    lines = []
    lines.append("=" * width)
    lines.append("SIDE-BY-SIDE COMPARISON")
    lines.append("=" * width)
    lines.append("")

    # Match jobs by name (normalize names for comparison)
    def normalize_name(name: str) -> str:
        return name.split("/")[-1].strip()

    jobs1 = {normalize_name(j.name): j for j in run1.jobs if j.completed_at}
    jobs2 = {normalize_name(j.name): j for j in run2.jobs if j.completed_at}

    all_job_names = sorted(set(jobs1.keys()) | set(jobs2.keys()))

    lines.append(f"{'Job':<45} | {'Run 1':^25} | {'Run 2':^25} | {'Delta':^15}")
    lines.append(f"{'':45} | {'#' + str(run1.id):^25} | {'#' + str(run2.id):^25} |")
    lines.append("-" * width)

    total_queue_delta = 0
    total_run_delta = 0

    for name in all_job_names:
        j1 = jobs1.get(name)
        j2 = jobs2.get(name)

        short_name = name[:43]

        if j1 and j2:
            q1 = j1.queue_duration_seconds
            r1 = j1.run_duration_seconds
            q2 = j2.queue_duration_seconds
            r2 = j2.run_duration_seconds

            run1_str = f"Q:{format_duration(q1):>6} R:{format_duration(r1):>6}"
            run2_str = f"Q:{format_duration(q2):>6} R:{format_duration(r2):>6}"

            run_delta = r2 - r1
            total_run_delta += run_delta
            total_queue_delta += (q2 - q1)

            if abs(run_delta) < 60:
                delta_str = "~same"
            elif run_delta > 0:
                delta_str = f"+{format_duration(run_delta)}"
            else:
                delta_str = f"-{format_duration(-run_delta)}"

            lines.append(f"{short_name:<45} | {run1_str:^25} | {run2_str:^25} | {delta_str:^15}")
        elif j1:
            q1 = j1.queue_duration_seconds
            r1 = j1.run_duration_seconds
            run1_str = f"Q:{format_duration(q1):>6} R:{format_duration(r1):>6}"
            lines.append(f"{short_name:<45} | {run1_str:^25} | {'(missing)':^25} |")
        elif j2:
            q2 = j2.queue_duration_seconds
            r2 = j2.run_duration_seconds
            run2_str = f"Q:{format_duration(q2):>6} R:{format_duration(r2):>6}"
            lines.append(f"{short_name:<45} | {'(missing)':^25} | {run2_str:^25} |")

    lines.append("-" * width)

    # Overall timing
    def get_wall_time(run: WorkflowRun) -> float:
        jobs = [j for j in run.jobs if j.completed_at]
        if not jobs:
            return 0
        min_t = min(j.created_at for j in jobs)
        max_t = max(j.completed_at for j in jobs)
        return (max_t - min_t).total_seconds()

    wall1 = get_wall_time(run1)
    wall2 = get_wall_time(run2)
    wall_delta = wall2 - wall1

    lines.append("")
    lines.append(f"{'TOTALS':<45} | {'Run 1':^25} | {'Run 2':^25} | {'Delta':^15}")
    lines.append("-" * width)
    lines.append(f"{'Wall clock time':<45} | {format_duration(wall1):^25} | {format_duration(wall2):^25} | {'+' if wall_delta > 0 else ''}{format_duration(abs(wall_delta)):^14}")
    lines.append(f"{'Sum of run times':<45} | {format_duration(sum(j.run_duration_seconds for j in jobs1.values())):^25} | {format_duration(sum(j.run_duration_seconds for j in jobs2.values())):^25} | {'+' if total_run_delta > 0 else ''}{format_duration(abs(total_run_delta)):^14}")
    lines.append(f"{'Sum of queue times':<45} | {format_duration(sum(j.queue_duration_seconds for j in jobs1.values())):^25} | {format_duration(sum(j.queue_duration_seconds for j in jobs2.values())):^25} | {'+' if total_queue_delta > 0 else ''}{format_duration(abs(total_queue_delta)):^14}")

    # Key insight
    lines.append("")
    lines.append("=" * width)
    lines.append("KEY INSIGHT:")
    if total_run_delta < -60:
        lines.append(f"  Run 2 saved {format_duration(-total_run_delta)} in actual build time across all jobs.")
    elif total_run_delta > 60:
        lines.append(f"  Run 2 spent {format_duration(total_run_delta)} MORE in actual build time across all jobs.")
    else:
        lines.append(f"  Actual build times are roughly the same.")

    if total_queue_delta > 60:
        lines.append(f"  BUT Run 2 had {format_duration(total_queue_delta)} MORE queue wait time.")
    elif total_queue_delta < -60:
        lines.append(f"  AND Run 2 had {format_duration(-total_queue_delta)} LESS queue wait time.")

    if wall_delta > 0 and total_run_delta < 0:
        lines.append(f"")
        lines.append(f"  Despite faster builds, wall clock time increased due to runner queue delays!")
    elif wall_delta < 0 and total_run_delta < 0:
        lines.append(f"")
        lines.append(f"  Build time savings translated to faster wall clock time.")

    lines.append("=" * width)

    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description="Visualize GitHub Actions workflow runs")
    parser.add_argument("run_id", nargs="?", type=int, help="Workflow run ID")
    parser.add_argument("--pr", type=int, help="PR number to find runs for")
    parser.add_argument("--commit", type=str, help="Commit SHA to find runs for")
    parser.add_argument("--repo", type=str, help="Repository (owner/repo)")
    parser.add_argument("--width", type=int, default=140, help="Terminal width")
    parser.add_argument("--height", type=int, default=50, help="Timeline height in rows")
    parser.add_argument("--compare", action="store_true", help="Show comparison view for multiple runs")
    parser.add_argument("--diff", type=int, help="Compare with another run ID")
    parser.add_argument("--history", type=int, nargs="*", help="Show history view for multiple run IDs")

    args = parser.parse_args()

    # Determine repo
    repo = args.repo
    if not repo:
        # Try to get from current directory
        result = subprocess.run(
            ["gh", "repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"],
            capture_output=True, text=True
        )
        if result.returncode == 0 and result.stdout.strip():
            repo = result.stdout.strip()
        else:
            print("Could not determine repository. Use --repo owner/repo", file=sys.stderr)
            sys.exit(1)

    # Handle --history mode (can have its own run IDs)
    if args.history is not None:
        history_ids = args.history
        if not history_ids:
            print("No run IDs provided for --history", file=sys.stderr)
            sys.exit(1)
        history_runs = []
        for rid in history_ids:
            print(f"Fetching run #{rid}...", file=sys.stderr)
            history_runs.append(get_run_details(rid, repo))
        print(render_history(history_runs, args.width))
        sys.exit(0)

    # Get run IDs
    run_ids = []
    if args.run_id:
        run_ids = [args.run_id]
    elif args.pr:
        run_ids = get_runs_for_pr(args.pr, repo)
        if not run_ids:
            print(f"No workflow runs found for PR #{args.pr}", file=sys.stderr)
            sys.exit(1)
    elif args.commit:
        run_ids = get_runs_for_commit(args.commit, repo)
        if not run_ids:
            print(f"No workflow runs found for commit {args.commit}", file=sys.stderr)
            sys.exit(1)
    else:
        parser.print_help()
        sys.exit(1)

    # Fetch run details
    runs = []
    for run_id in run_ids:
        print(f"Fetching run #{run_id}...", file=sys.stderr)
        runs.append(get_run_details(run_id, repo))

    # Handle --diff mode
    if args.diff:
        print(f"Fetching comparison run #{args.diff}...", file=sys.stderr)
        diff_run = get_run_details(args.diff, repo)
        print(render_side_by_side(runs[0], diff_run, args.width))
        print()
        print("=" * args.width)
        print("TIMELINE: Run 1 (baseline)")
        print("=" * args.width)
        print(render_timeline(runs[0], args.width, args.height))
        print()
        print("=" * args.width)
        print("TIMELINE: Run 2 (comparison)")
        print("=" * args.width)
        print(render_timeline(diff_run, args.width, args.height))
    elif args.compare or len(runs) > 1:
        print(render_comparison(runs, args.width))
        print()
        for run in runs:
            print(render_timeline(run, args.width, args.height))
            print("\n" + "=" * args.width + "\n")
    else:
        print(render_timeline(runs[0], args.width, args.height))


if __name__ == "__main__":
    main()
