---
name: task-externalization
description: System for externalizing large task context to .scratch/tasks/ files to maintain continuity across context compactions and session resets. Use this skill when starting large/complex tasks, after /compact or /new commands, or when context window is getting full during multi-file work.
---

# Task Externalization System

Maintain continuity for large tasks across context compactions and session resets by externalizing task state to `.scratch/tasks/`.

## When to Use

Use task externalization when:
- Starting large/complex tasks (5+ files, architectural changes)
- After `/compact` or `/new` commands
- Context window is getting full during multi-file work
- Task spans multiple sessions

## Two-File Structure

### 1. Overview File: `.scratch/tasks.md`

Tracks overall state across all tasks:
- Global Context: Shared decisions/constraints
- Current Task: Which task file is active
- Task List: All tasks with status (pending, in progress, completed)
- Cross-task Notes: Dependencies, blockers

### 2. Task Files: `.scratch/tasks/{name}.md`

Each task file contains:
- Goal: What needs to be accomplished
- Context: Background, decisions, constraints
- Files to Modify: List of files to change
- Implementation Plan: Step-by-step approach
- Progress Notes: What's done, what remains, blockers
- Testing: How to verify

**See** `references/file-structure.md` for complete examples

## Quick Workflow

**Starting session or after `/compact`/`/new`:**
1. Check if `.scratch/tasks.md` exists
2. If yes: Read it + current task file
3. Summarize and continue

**Starting new large task:**
1. Create `.scratch/tasks/` directory
2. Create/update overview file
3. Create task file with plan
4. Update overview to mark as current
5. Begin implementation

**During work:**
- Update Progress Notes as steps complete
- Note blockers and decisions

**Completing task:**
1. Mark complete in task file
2. Update overview: `[x]`
3. Update current task to next pending

**See** `references/workflows.md` for detailed workflows

## Important Notes

- Keep progress notes current for accurate recovery
- Don't delete task files until entire effort is complete
- `.scratch/` is already in gitignore
- Task files are read/updated throughout work
