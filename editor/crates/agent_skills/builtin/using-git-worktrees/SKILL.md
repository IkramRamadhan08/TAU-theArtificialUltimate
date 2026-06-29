---
name: using-git-worktrees
description: Use when starting feature work that needs isolation from current workspace or before executing implementation plans - ensures an isolated workspace exists via git worktree
---

# Using Git Worktrees

## Overview

Ensure work happens in an isolated workspace.

**Core principle:** Detect existing isolation first. Never nest worktrees.

**Announce at start:** "I'm using the using-git-worktrees skill to set up an isolated workspace."

## Step 0: Detect Existing Isolation

```bash
GIT_DIR=$(cd "$(git rev-parse --git-dir)" 2>/dev/null && pwd -P)
GIT_COMMON=$(cd "$(git rev-parse --git-common-dir)" 2>/dev/null && pwd -P)
BRANCH=$(git branch --show-current)
```

**Submodule guard:** Verify you're not in a submodule:
```bash
git rev-parse --show-superproject-working-tree 2>/dev/null
```

**If `GIT_DIR != GIT_COMMON` (and not a submodule):** Already in a linked worktree. Skip to setup.

**If `GIT_DIR == GIT_COMMON`:** Normal repo. Ask for consent before creating a worktree.

## Step 1: Create Workspace

If user consents, create via git worktree:

```bash
# Use .worktrees/ at project root
git check-ignore -q .worktrees || (echo ".worktrees" >> .gitignore && git add .gitignore && git commit -m "chore: ignore worktrees directory")

git worktree add .worktrees/$BRANCH_NAME -b "$BRANCH_NAME"
cd .worktrees/$BRANCH_NAME
```

## Step 2: Project Setup

Auto-detect and run appropriate setup:
```bash
# Node.js
if [ -f package.json ]; then npm install; fi
# Rust
if [ -f Cargo.toml ]; then cargo build; fi
# Python
if [ -f requirements.txt ]; then pip install -r requirements.txt; fi
# Go
if [ -f go.mod ]; then go mod download; fi
```

## Step 3: Verify Clean Baseline

Run tests to ensure workspace starts clean. If tests fail, report and ask.

## Quick Reference

| Situation | Action |
|-----------|--------|
| Already in linked worktree | Skip creation |
| In a submodule | Treat as normal repo |
| `.worktrees/` exists | Use it (verify ignored) |
| Directory not ignored | Add to .gitignore + commit |
| Tests fail during baseline | Report + ask |

## Red Flags

**Never:** Create nested worktree, skip detection, skip ignore verification, proceed with failing tests without asking.
