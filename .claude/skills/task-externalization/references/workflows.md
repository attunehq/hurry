# Task Externalization Workflows

## Starting a Session or After `/compact` or `/new`

**Step 1: Check for existing tasks**
```bash
ls .scratch/tasks.md
```

**Step 2: If tasks.md exists**
1. Read `.scratch/tasks.md` to understand overall state
2. Identify current task from "Current Task" section
3. Read the current task file (e.g., `.scratch/tasks/01-refactor-storage.md`)

**Step 3: Summarize for user**
```
Continuing work on [task name].

Already completed:
- [X] Step 1
- [X] Step 2

Next step: [Y from Progress Notes]

Blocker: [any noted blockers]
```

**Step 4: Continue implementation**
- Pick up from where Progress Notes indicate
- Update Progress Notes as you work

## Starting a New Large Task

**Step 1: Create directory structure**
```bash
mkdir -p .scratch/tasks
```

**Step 2: Create overview file**
Create `.scratch/tasks.md`:
```markdown
# Task Overview

## Global Context
[Shared constraints/decisions]

## Current Task
In progress: `01-feature-name.md`

## Tasks
- [ ] 01-feature-name.md (IN PROGRESS)

## Cross-task Notes
[Any dependencies or notes]
```

**Step 3: Create task file**
Create `.scratch/tasks/01-feature-name.md`:
```markdown
# Task: [Descriptive Name]

## Goal
[What needs to be accomplished]

## Context
[Background, decisions, constraints]

## Files to Modify
- file1.rs: [what changes]
- file2.rs: [what changes]

## Implementation Plan
1. [Step 1]
2. [Step 2]
3. [Step 3]

## Progress Notes
- [ ] Step 1
- [ ] Step 2

## Testing
[How to verify]
```

**Step 4: Summarize and begin**
Tell user the plan, then start implementing.

## During Task Work

**Update Progress Notes regularly:**
```markdown
## Progress Notes
- [x] Defined Storage trait
- [x] Created disk implementation
- [ ] Currently: Updating API layer
- [ ] Todo: Update tests
- [ ] Blocker: Unsure about lifetime handling
```

**Document important decisions:**
```markdown
## Progress Notes
- [x] Defined Storage trait
  - Decision: Used async traits for API consistency
  - Alternative considered: sync traits, but would need wrapper
```

## Completing a Task

**Step 1: Mark task file complete**
Update all Progress Notes to `[x]`

**Step 2: Update overview**
In `.scratch/tasks.md`:
```markdown
## Tasks
- [x] 01-feature-name.md
- [ ] 02-next-task.md (IN PROGRESS)

## Current Task
In progress: `02-next-task.md`
```

**Step 3: Inform user**
```
✅ Completed: [task name]

Summary:
- [What was accomplished]
- [Files modified]
- [Tests passing]

Next task: [name of next task or "All tasks complete"]
```

## When Context Gets Low

**Step 1: Update current task file**
Ensure Progress Notes are current with:
- What's been completed
- What's in progress
- What remains
- Any blockers or decisions

**Step 2: Save findings**
Document any important discoveries in Progress Notes or Context

**Step 3: Suggest compaction**
```
Context window getting full. Current progress saved to .scratch/tasks/01-feature-name.md.

Recommend running `/compact` or `/new` to continue with fresh context.
```

**Step 4: Auto-recovery after compaction**
When session resumes, automatically:
1. Check for `.scratch/tasks.md`
2. Read and summarize current state
3. Continue where left off

## Managing Multiple Related Tasks

**Overview tracks all tasks:**
```markdown
## Tasks
- [x] 01-refactor-storage.md
- [ ] 02-add-caching.md (IN PROGRESS)
- [ ] 03-update-api.md
- [ ] 04-write-docs.md

## Cross-task Notes
- Caching layer depends on storage refactor (complete)
- API changes will be breaking: need migration guide
- Docs should wait until API is stable
```

**Update Current Task as you complete each:**
```markdown
## Current Task
In progress: `02-add-caching.md`
```

This allows you to work through a series of related tasks while maintaining context continuity.
