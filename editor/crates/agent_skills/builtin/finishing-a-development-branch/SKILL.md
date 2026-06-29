---
name: finishing-a-development-branch
description: Use when implementation is complete, all tests pass, and you need to decide how to integrate the work - guides completion of development work by presenting structured options for merge, PR, or cleanup
---

# Finishing a Development Branch

## Overview

Guide completion of development work by presenting clear options and handling chosen workflow.

**Core principle:** Verify tests → Detect environment → Present options → Execute choice → Clean up.

**Announce at start:** "I'm using the finishing-a-development-branch skill to complete this work."

## The Process

### Step 1: Verify Tests

**Before presenting options, verify tests pass using the project's test command.**

**If tests fail:** Report failures. Cannot proceed until tests pass.

**If tests pass:** Continue to Step 2.

### Step 2: Detect Environment

Determine workspace state:
```bash
GIT_DIR=$(cd "$(git rev-parse --git-dir)" 2>/dev/null && pwd -P)
GIT_COMMON=$(cd "$(git rev-parse --git-common-dir)" 2>/dev/null && pwd -P)
```

### Step 3: Determine Base Branch

```bash
git merge-base HEAD main 2>/dev/null || git merge-base HEAD master 2>/dev/null
```

### Step 4: Present Options

```
Implementation complete. What would you like to do?

1. Merge back to <base-branch> locally
2. Push and create a Pull Request
3. Keep the branch as-is (I'll handle it later)
4. Discard this work

Which option?
```

### Step 5: Execute Choice

#### Option 1: Merge Locally
```bash
git checkout <base-branch>
git pull
git merge <feature-branch>
# Verify tests on merged result
# Only after merge succeeds: cleanup, then delete branch
git branch -d <feature-branch>
```

#### Option 2: Push and Create PR
```bash
git push -u origin <feature-branch>
```
Do NOT clean up worktree — user needs it alive to iterate on PR feedback.

#### Option 3: Keep As-Is
Report: "Keeping branch <name>."

#### Option 4: Discard
Confirm first — require typed "discard" confirmation.
Then: force-delete branch: `git branch -D <feature-branch>`

## Quick Reference

| Option | Merge | Push | Keep Worktree | Cleanup Branch |
|--------|-------|------|---------------|----------------|
| 1. Merge locally | yes | - | - | yes |
| 2. Create PR | - | yes | yes | - |
| 3. Keep as-is | - | - | yes | - |
| 4. Discard | - | - | - | yes (force) |

## Common Mistakes

**Skipping test verification** — Always verify tests before offering options
**Cleaning up worktree for Option 2** — Only cleanup for Options 1 and 4
**Deleting branch before removing worktree** — Merge first, remove worktree, then delete branch
**No confirmation for discard** — Require typed "discard" confirmation

## Red Flags

**Never:**
- Proceed with failing tests
- Merge without verifying tests on result
- Delete work without confirmation
- Force-push without explicit request
