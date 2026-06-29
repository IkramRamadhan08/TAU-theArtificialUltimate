---
name: systematic-debugging
description: Use when encountering any bug, test failure, or unexpected behavior, before proposing fixes
---

# Systematic Debugging

## Overview

Random fixes waste time and create new bugs. Quick patches mask underlying issues.

**Core principle:** ALWAYS find root cause before attempting fixes. Symptom fixes are failure.

## The Iron Law

```
NO FIXES WITHOUT ROOT CAUSE INVESTIGATION FIRST
```

## When to Use

Use for ANY technical issue: test failures, bugs, unexpected behavior, performance problems, build failures, integration issues.

**Use ESPECIALLY when:** under time pressure, "just one quick fix" seems obvious, you've already tried multiple fixes, you don't fully understand the issue.

## The Four Phases

Complete each phase before proceeding to the next.

### Phase 1: Root Cause Investigation

**BEFORE attempting ANY fix:**

1. **Read Error Messages Carefully** — stack traces, line numbers, file paths
2. **Reproduce Consistently** — exact steps, frequency
3. **Check Recent Changes** — git diff, recent commits
4. **Gather Evidence** — log data at component boundaries to find where it breaks
5. **Trace Data Flow** — where does bad value originate? Trace up until you find the source

### Phase 2: Pattern Analysis

1. **Find Working Examples** — similar working code in same codebase
2. **Compare Against References** — read reference implementation completely
3. **Identify Differences** — what's different between working and broken?
4. **Understand Dependencies** — what components, settings, environment does this need?

### Phase 3: Hypothesis and Testing

1. **Form Single Hypothesis** — "I think X is the root cause because Y"
2. **Test Minimally** — smallest possible change, one variable at a time
3. **Verify Before Continuing** — worked? Proceed. Didn't work? New hypothesis.

### Phase 4: Implementation

1. **Create Failing Test Case** — simplest possible reproduction
2. **Implement Single Fix** — ONE change at a time, no "while I'm here" improvements
3. **Verify Fix** — test passes, no other tests broken
4. **If Fix Doesn't Work:** STOP. If < 3 fixes: return to Phase 1. If ≥ 3: question the architecture.

## Supporting Techniques

### Root Cause Tracing
Trace backward through call stack to find original trigger. Start at the symptom (error, crash, wrong output). Work backward: "What called this?" Keep asking until you find where the bad value or state originated. Fix at source, not at symptom.

### Defense in Depth
After finding root cause, add validation at multiple layers: validate at input boundaries, add assertions at key transformation points, log unexpected states. Each layer catches failures the previous layer missed.

### Condition-Based Waiting
Replace arbitrary timeouts with condition polling:
```python
def wait_for_condition(check_fn, timeout=5.0, interval=0.1):
    deadline = time.time() + timeout
    while time.time() < deadline:
        result = check_fn()
        if result:
            return result
        time.sleep(interval)
    raise TimeoutError("Condition not met")
```

## Red Flags

If you catch yourself thinking: "Quick fix for now", "Just try changing X", "Skip the test", "I don't fully understand but this might work", "One more fix attempt" (3+ tries) — STOP. Return to Phase 1.

## Common Rationalizations

| Excuse | Reality |
|--------|---------|
| "Issue is simple, don't need process" | Simple issues have root causes too |
| "Emergency, no time for process" | Systematic is FASTER than thrashing |
| "Just try this first, then investigate" | First fix sets the pattern. Do it right |
| "I'll write test after confirming fix works" | Untested fixes don't stick |
| "Multiple fixes at once saves time" | Can't isolate what worked |

## Quick Reference

| Phase | Key Activities | Success Criteria |
|-------|---------------|------------------|
| **1. Root Cause** | Read errors, reproduce, check changes, gather evidence | Understand WHAT and WHY |
| **2. Pattern** | Find working examples, compare | Identify differences |
| **3. Hypothesis** | Form theory, test minimally | Confirmed or new hypothesis |
| **4. Implementation** | Create test, fix, verify | Bug resolved, tests pass |

**Related skill:** tau:test-driven-development — for creating failing test case (Phase 4, Step 1)
