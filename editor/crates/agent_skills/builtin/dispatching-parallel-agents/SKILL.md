---
name: dispatching-parallel-agents
description: Use when facing 2+ independent tasks that can be worked on without shared state or sequential dependencies
---

# Dispatching Parallel Agents

## Overview

You delegate tasks to specialized agents with isolated context. By precisely crafting their instructions and context, you ensure they stay focused and succeed. They should never inherit your session's context or history — you construct exactly what they need.

When you have multiple unrelated failures (different test files, different subsystems, different bugs), investigating them sequentially wastes time. Each investigation is independent and can happen in parallel.

**Core principle:** Dispatch one agent per independent problem domain. Let them work concurrently.

## When to Use

**Use when:**
- 3+ test files failing with different root causes
- Multiple subsystems broken independently
- Each problem can be understood without context from others
- No shared state between investigations

**Don't use when:**
- Failures are related (fix one might fix others)
- Need to understand full system state
- Agents would interfere with each other

## The Pattern

### 1. Identify Independent Domains

Group failures by what's broken.

### 2. Create Focused Agent Tasks

Each agent gets:
- **Specific scope:** One test file or subsystem
- **Clear goal:** Make these tests pass
- **Constraints:** Don't change other code
- **Expected output:** Summary of what you found and fixed

### 3. Dispatch in Parallel

Issue all subagent dispatches in the same response — they run in parallel:

```
Task (general-purpose):
  description: "Fix agent-tool-abort.test.ts failures"
  prompt: "..."
Task (general-purpose):
  description: "Fix batch-completion-behavior.test.ts failures"
  prompt: "..."
```

### 4. Review and Integrate

When agents return:
- Read each summary
- Verify fixes don't conflict
- Run full test suite
- Integrate all changes

## Common Mistakes

**Too broad:** "Fix all the tests" — agent gets lost
**No context:** "Fix the race condition" — agent doesn't know where
**No constraints:** Agent might refactor everything
**Vague output:** "Fix it" — you don't know what changed

## When NOT to Use

**Related failures:** Fixing one might fix others
**Need full context:** Understanding requires seeing entire system
**Exploratory debugging:** You don't know what's broken yet
**Shared state:** Agents would interfere (editing same files)

## Key Benefits

1. **Parallelization** — Multiple investigations happen simultaneously
2. **Focus** — Each agent has narrow scope
3. **Independence** — Agents don't interfere with each other
4. **Speed** — N problems solved in time of 1

## Verification

After agents return:
1. **Review each summary** — Understand what changed
2. **Check for conflicts** — Did agents edit same code?
3. **Run full suite** — Verify all fixes work together
