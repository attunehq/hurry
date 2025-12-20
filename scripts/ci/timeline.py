#!/usr/bin/env python3
"""
CI Timeline Visualizer

Visualizes GitHub Actions workflow runs, showing:
- Job timelines with queue vs run time
- Sparklines showing parallelism over time
- Unified diff view for comparing two runs

Usage:
    # List runs to find run IDs
    ./timeline.py --list --pr <pr_number> [--repo owner/repo]
    ./timeline.py --list --branch <branch_name> [--repo owner/repo]
    ./timeline.py --list [--repo owner/repo]  # lists recent runs on main

    # View a single run
    ./timeline.py <run_id> [--repo owner/repo]
    ./timeline.py --branch <branch_name> [--repo owner/repo]  # most recent run

    # View runs for a PR or commit
    ./timeline.py --pr <pr_number> [--repo owner/repo]
    ./timeline.py --commit <sha> [--repo owner/repo]

    # Compare two runs (great for before/after comparisons)
    ./timeline.py <baseline_run_id> --diff <comparison_run_id> --repo owner/repo

    # Show history of multiple runs
    ./timeline.py --history <run1> <run2> <run3> --repo owner/repo
    ./timeline.py --history --branch <branch_name> --repo owner/repo
    ./timeline.py --history --pr <pr_number> --repo owner/repo

Example:
    # Find run IDs for a PR, then compare two of them
    ./timeline.py --list --pr 123 --repo owner/repo
    ./timeline.py 111111 --diff 222222 --repo owner/repo

Legend:
    █ = Job running
    ░ = Job queued (waiting for runner)
    ▁▂▃▄▅▆▇█ = Sparkline showing parallel job activity
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


def get_runs_for_branch(branch: str, repo: str, limit: int = 10) -> List[dict]:
    """Get recent workflow runs for a branch."""
    runs_data = run_gh_api(f"actions/runs?branch={branch}&per_page={limit}", repo)
    return runs_data.get("workflow_runs", [])


def list_runs(runs_data: List[dict], repo: str) -> str:
    """Format a list of runs for display."""
    if not runs_data:
        return "No runs found"

    lines = []
    lines.append(f"{'Run ID':<12} {'Workflow':<40} {'Status':<12} {'Created':<20}")
    lines.append("-" * 90)

    for run in runs_data:
        run_id = run["id"]
        workflow = run["name"][:38]
        status = run.get("conclusion") or run["status"]
        created = run["created_at"][:19].replace("T", " ")
        lines.append(f"{run_id:<12} {workflow:<40} {status:<12} {created:<20}")

    lines.append("-" * 90)
    lines.append("")
    lines.append("Use these run IDs with:")
    lines.append(f"  ./timeline.py <run_id> --repo {repo}")
    lines.append(f"  ./timeline.py <run1> --diff <run2> --repo {repo}")
    lines.append(f"  ./timeline.py --history <run1> <run2> ... --repo {repo}")

    return "\n".join(lines)


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


# Unicode block characters for sparklines (lowest to highest)
SPARK_BLOCKS = " ▁▂▃▄▅▆▇█"


def render_sparkline(jobs: List[Job], width: int = 40) -> str:
    """Render job activity over time as a Unicode sparkline.

    Each character represents a time bucket. Height indicates how many
    jobs were running (not queued, not finished) during that bucket.
    """
    if not jobs:
        return " " * width

    completed_jobs = [j for j in jobs if j.started_at and j.completed_at]
    if not completed_jobs:
        return " " * width

    min_time = min(j.created_at for j in completed_jobs)
    max_time = max(j.completed_at for j in completed_jobs)
    total_seconds = (max_time - min_time).total_seconds()

    if total_seconds == 0:
        return "█" * width

    bucket_seconds = total_seconds / width
    max_parallelism = len(completed_jobs)

    result = []
    for i in range(width):
        bucket_start = min_time.timestamp() + (i * bucket_seconds)
        bucket_end = bucket_start + bucket_seconds

        # Count jobs running during this bucket
        running_count = 0
        for j in completed_jobs:
            started_ts = j.started_at.timestamp()
            completed_ts = j.completed_at.timestamp()
            # Job is running if it started before bucket ends and completed after bucket starts
            if started_ts < bucket_end and completed_ts > bucket_start:
                running_count += 1

        # Map count to block character
        if running_count == 0:
            result.append(SPARK_BLOCKS[0])
        else:
            # Scale to 1-8 range
            level = min(8, max(1, int((running_count / max_parallelism) * 8)))
            result.append(SPARK_BLOCKS[level])

    return "".join(result)


def render_job_bar(queue_secs: float, run_secs: float, max_secs: float, width: int = 16) -> str:
    """Render a job's timeline as a horizontal bar.

    Returns a string like: ░░░████████ (queue then run)
    """
    if max_secs == 0:
        return " " * width

    total = queue_secs + run_secs
    total_chars = int((total / max_secs) * width)
    queue_chars = int((queue_secs / max_secs) * width)
    run_chars = total_chars - queue_chars

    bar = "░" * queue_chars + "█" * run_chars
    return bar.ljust(width)


def normalize_job_name(name: str) -> str:
    """Extract a short, normalized job name for matching and display."""
    short = name.split("/")[-1].strip()
    # Remove common wrapper patterns
    short = short.replace("build (", "").replace(")", "")
    short = short.replace("ubuntu-22.04, ", "").replace("ubuntu-24.04, ", "")
    short = short.replace("macos-14, ", "").replace("macos-15, ", "")
    short = short.replace("windows-2022, ", "").replace("windows-2025, ", "")
    return short


def render_single_run(run: WorkflowRun, width: int = 120) -> str:
    """Render a compact view of a single workflow run."""
    if not run.jobs:
        return "No jobs found"

    jobs = [j for j in run.jobs if j.started_at and j.completed_at]
    if not jobs:
        return "No completed jobs with timing info"

    min_time = min(j.created_at for j in jobs)
    max_time = max(j.completed_at for j in jobs)
    total_seconds = (max_time - min_time).total_seconds()

    if total_seconds == 0:
        return "All jobs completed instantly"

    # Sort jobs by total duration (longest first)
    jobs = sorted(jobs, key=lambda j: j.total_duration_seconds, reverse=True)

    lines = []

    # Header
    lines.append("=" * width)
    lines.append(f"Workflow: {run.name} (Run #{run.id})")
    lines.append(f"Status: {run.status} / {run.conclusion or 'in progress'}")
    lines.append("=" * width)
    lines.append("")

    # Find max duration for scaling bars
    max_duration = max(j.total_duration_seconds for j in jobs)
    bar_width = 20
    name_width = 40

    # Job table with bars
    lines.append(f"{'Job':<{name_width}} {'Timeline':<{bar_width + 2}} {'Queue':>7} {'Run':>7} {'Total':>7}")
    lines.append("-" * width)

    for j in jobs:
        short_name = normalize_job_name(j.name)[:name_width - 2]
        bar = render_job_bar(j.queue_duration_seconds, j.run_duration_seconds, max_duration, bar_width)
        queue = format_duration(j.queue_duration_seconds)
        run_time = format_duration(j.run_duration_seconds)
        total = format_duration(j.total_duration_seconds)
        lines.append(f"{short_name:<{name_width}} {bar}  {queue:>7} {run_time:>7} {total:>7}")

    lines.append("-" * width)

    # Summary stats
    total_queue = sum(j.queue_duration_seconds for j in jobs)
    total_run = sum(j.run_duration_seconds for j in jobs)
    max_queue = max(j.queue_duration_seconds for j in jobs)

    lines.append("")
    lines.append(f"Wall clock: {format_duration(total_seconds):>10}    Sum of run times: {format_duration(total_run):>10}")
    lines.append(f"Max queue:  {format_duration(max_queue):>10}    Sum of queue times: {format_duration(total_queue):>10}")

    # Sparkline showing parallelism over time
    sparkline = render_sparkline(jobs, width=50)
    lines.append("")
    lines.append(f"Activity:   {sparkline}")
    lines.append(f"            {'0':^10}{format_duration(total_seconds/2):^30}{format_duration(total_seconds):>10}")

    # Critical path
    last_job = max(jobs, key=lambda j: j.completed_at)
    lines.append("")
    lines.append(f"Critical path: {normalize_job_name(last_job.name)} (queue {format_duration(last_job.queue_duration_seconds)}, run {format_duration(last_job.run_duration_seconds)})")

    # Legend
    lines.append("")
    lines.append("Legend: █ running  ░ queued")

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
    sparkline_width = 40
    lines.append(f"{'Run ID':<12} | {'Wall':>7} | {'Build':>7} | {'Queue':>7} | Activity")
    lines.append("-" * width)

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

        # Sparkline showing job activity over time
        sparkline = render_sparkline(jobs, width=sparkline_width)

        lines.append(f"{run.id:<12} | {format_duration(wall_time):>7} | {format_duration(build_time):>7} | {format_duration(queue_time):>7} | {sparkline}")

    lines.append("-" * width)
    lines.append("")
    lines.append("Activity: Height shows parallel job count over time (▁▂▃▄▅▆▇█)")
    lines.append("          Low blocks at start = queue delay; sustained height = good parallelism")

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


def render_unified_diff(run1: WorkflowRun, run2: WorkflowRun, width: int = 140) -> str:
    """Render a compact unified comparison of two runs."""
    lines = []
    lines.append("=" * width)
    lines.append(f"COMPARING: Run #{run1.id} vs #{run2.id}")
    lines.append("=" * width)
    lines.append("")

    # Match jobs by normalized name
    jobs1 = {normalize_job_name(j.name): j for j in run1.jobs if j.completed_at}
    jobs2 = {normalize_job_name(j.name): j for j in run2.jobs if j.completed_at}
    all_job_names = sorted(set(jobs1.keys()) | set(jobs2.keys()))

    # Find max duration across both runs for consistent bar scaling
    all_durations = []
    for name in all_job_names:
        if name in jobs1:
            all_durations.append(jobs1[name].total_duration_seconds)
        if name in jobs2:
            all_durations.append(jobs2[name].total_duration_seconds)
    max_duration = max(all_durations) if all_durations else 1

    bar_width = 16
    name_width = 28

    # Header
    lines.append(f"{'Job':<{name_width}} {'Run 1':<{bar_width + 8}} {'Run 2':<{bar_width + 8}} {'Delta':>10}")
    lines.append("-" * width)

    total_run_delta = 0
    total_queue_delta = 0

    for name in all_job_names:
        j1 = jobs1.get(name)
        j2 = jobs2.get(name)
        short_name = name[:name_width - 2]

        if j1 and j2:
            bar1 = render_job_bar(j1.queue_duration_seconds, j1.run_duration_seconds, max_duration, bar_width)
            bar2 = render_job_bar(j2.queue_duration_seconds, j2.run_duration_seconds, max_duration, bar_width)
            time1 = format_duration(j1.total_duration_seconds)
            time2 = format_duration(j2.total_duration_seconds)

            run_delta = j2.run_duration_seconds - j1.run_duration_seconds
            total_run_delta += run_delta
            total_queue_delta += (j2.queue_duration_seconds - j1.queue_duration_seconds)

            if abs(run_delta) < 60:
                delta_str = "~same"
            elif run_delta > 0:
                delta_str = f"+{format_duration(run_delta)}"
            else:
                delta_str = f"-{format_duration(-run_delta)}"

            lines.append(f"{short_name:<{name_width}} {bar1} {time1:>6}  {bar2} {time2:>6}  {delta_str:>10}")
        elif j1:
            bar1 = render_job_bar(j1.queue_duration_seconds, j1.run_duration_seconds, max_duration, bar_width)
            time1 = format_duration(j1.total_duration_seconds)
            lines.append(f"{short_name:<{name_width}} {bar1} {time1:>6}  {'(missing)':<{bar_width + 7}}")
        elif j2:
            bar2 = render_job_bar(j2.queue_duration_seconds, j2.run_duration_seconds, max_duration, bar_width)
            time2 = format_duration(j2.total_duration_seconds)
            lines.append(f"{short_name:<{name_width}} {'(missing)':<{bar_width + 7}}  {bar2} {time2:>6}")

    lines.append("-" * width)

    # Wall clock times
    def get_wall_time(run: WorkflowRun) -> float:
        jobs = [j for j in run.jobs if j.completed_at]
        if not jobs:
            return 0
        return (max(j.completed_at for j in jobs) - min(j.created_at for j in jobs)).total_seconds()

    wall1 = get_wall_time(run1)
    wall2 = get_wall_time(run2)
    wall_delta = wall2 - wall1

    sum_run1 = sum(j.run_duration_seconds for j in jobs1.values())
    sum_run2 = sum(j.run_duration_seconds for j in jobs2.values())
    sum_queue1 = sum(j.queue_duration_seconds for j in jobs1.values())
    sum_queue2 = sum(j.queue_duration_seconds for j in jobs2.values())

    lines.append("")
    lines.append(f"{'Wall clock:':<{name_width}} {format_duration(wall1):>{bar_width + 7}}  {format_duration(wall2):>{bar_width + 7}}  {'+' if wall_delta > 0 else ''}{format_duration(abs(wall_delta)):>10}")
    lines.append(f"{'Sum of run times:':<{name_width}} {format_duration(sum_run1):>{bar_width + 7}}  {format_duration(sum_run2):>{bar_width + 7}}  {'+' if total_run_delta > 0 else ''}{format_duration(abs(total_run_delta)):>10}")
    lines.append(f"{'Sum of queue times:':<{name_width}} {format_duration(sum_queue1):>{bar_width + 7}}  {format_duration(sum_queue2):>{bar_width + 7}}  {'+' if total_queue_delta > 0 else ''}{format_duration(abs(total_queue_delta)):>10}")

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
        lines.append("")
        lines.append(f"  Despite faster builds, wall clock time increased due to runner queue delays!")
    elif wall_delta < 0 and total_run_delta < 0:
        lines.append("")
        lines.append(f"  Build time savings translated to faster wall clock time.")

    lines.append("=" * width)
    lines.append("")
    lines.append("Legend: █ running  ░ queued")

    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description="Visualize GitHub Actions workflow runs")
    parser.add_argument("run_id", nargs="?", type=int, help="Workflow run ID")
    parser.add_argument("--pr", type=int, help="PR number to find runs for")
    parser.add_argument("--branch", type=str, help="Branch name to find runs for")
    parser.add_argument("--commit", type=str, help="Commit SHA to find runs for")
    parser.add_argument("--repo", type=str, help="Repository (owner/repo)")
    parser.add_argument("--width", type=int, default=120, help="Terminal width")
    parser.add_argument("--compare", action="store_true", help="Show comparison view for multiple runs")
    parser.add_argument("--diff", type=int, help="Compare with another run ID")
    parser.add_argument("--history", type=int, nargs="*", help="Show history view for multiple run IDs")
    parser.add_argument("--list", action="store_true", help="List runs instead of visualizing")
    parser.add_argument("--limit", type=int, default=10, help="Number of runs to list (default: 10)")

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

    # Handle --list mode (list runs for a PR or branch)
    if args.list:
        if args.branch:
            runs_data = get_runs_for_branch(args.branch, repo, args.limit)
            print(f"Recent runs for branch '{args.branch}':\n")
            print(list_runs(runs_data, repo))
        elif args.pr:
            pr_data = run_gh_api(f"pulls/{args.pr}", repo)
            head_sha = pr_data["head"]["sha"]
            runs_data = run_gh_api(f"actions/runs?head_sha={head_sha}", repo)
            print(f"Runs for PR #{args.pr} (head: {head_sha[:8]}):\n")
            print(list_runs(runs_data.get("workflow_runs", []), repo))
        else:
            # List recent runs for the default branch
            runs_data = get_runs_for_branch("main", repo, args.limit)
            if not runs_data:
                runs_data = get_runs_for_branch("master", repo, args.limit)
            print(f"Recent runs:\n")
            print(list_runs(runs_data, repo))
        sys.exit(0)

    # Handle --history mode
    if args.history is not None:
        history_ids = args.history

        # If no run IDs provided, try to get them from --branch or --pr
        if not history_ids:
            if args.branch:
                runs_data = get_runs_for_branch(args.branch, repo, args.limit)
                if not runs_data:
                    print(f"No workflow runs found for branch {args.branch}", file=sys.stderr)
                    sys.exit(1)
                history_ids = [r["id"] for r in runs_data]
            elif args.pr:
                pr_data = run_gh_api(f"pulls/{args.pr}", repo)
                head_sha = pr_data["head"]["sha"]
                runs_data = run_gh_api(f"actions/runs?head_sha={head_sha}", repo)
                workflow_runs = runs_data.get("workflow_runs", [])
                if not workflow_runs:
                    print(f"No workflow runs found for PR #{args.pr}", file=sys.stderr)
                    sys.exit(1)
                history_ids = [r["id"] for r in workflow_runs]
            else:
                print("No run IDs provided for --history. Use with --branch or --pr, or provide run IDs.", file=sys.stderr)
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
    elif args.branch:
        runs_data = get_runs_for_branch(args.branch, repo, 1)
        if not runs_data:
            print(f"No workflow runs found for branch {args.branch}", file=sys.stderr)
            sys.exit(1)
        run_ids = [runs_data[0]["id"]]
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
        print(render_unified_diff(runs[0], diff_run, args.width))
    elif args.compare or len(runs) > 1:
        print(render_comparison(runs, args.width))
        print()
        for run in runs:
            print(render_single_run(run, args.width))
            print()
    else:
        print(render_single_run(runs[0], args.width))


if __name__ == "__main__":
    main()
