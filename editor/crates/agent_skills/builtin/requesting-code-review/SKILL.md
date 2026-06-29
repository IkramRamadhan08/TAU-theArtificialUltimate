---
name: requesting-code-review
description: Use when completing tasks, implementing major features, or before merging to verify work meets requirements
---

# Requesting Code Review

Dispatch a code reviewer subagent to catch issues before they cascade. The reviewer gets precisely crafted context — never your session's history.

**Core principle:** Review early, review often.

## When to Request Review

**Mandatory:**
- After each task in tau:subagent-driven-development
- After completing major feature
- Before merge to main

**Optional but valuable:**
- When stuck (fresh perspective)
- Before refactoring (baseline check)
- After fixing complex bug

## How to Request

**1. Get git SHAs:**
```bash
BASE_SHA=$(git rev-parse HEAD~1)
HEAD_SHA=$(git rev-parse HEAD)
```

**2. Dispatch code reviewer subagent using the template below.**

**3. Act on feedback:**
- Fix Critical issues immediately
- Fix Important issues before proceeding
- Note Minor issues for later
- Push back if reviewer is wrong (with reasoning)

## Code Reviewer Prompt Template

```
Subagent (general-purpose):
  description: "Review code changes"
  model: [most capable available model]
  prompt: |
    You are a Senior Code Reviewer with expertise in software architecture,
    design patterns, and best practices.

    ## What Was Implemented

    [DESCRIPTION]

    ## Requirements / Plan

    [PLAN_OR_REQUIREMENTS]

    ## Git Range to Review

    Base: [BASE_SHA]
    Head: [HEAD_SHA]

    Review the diff between these commits.

    ## What to Check

    **Plan alignment:** Does the implementation match requirements?
    **Code quality:** Clean separation? Error handling? Edge cases?
    **Architecture:** Sound design decisions? Security concerns?
    **Testing:** Real behavior verified? Edge cases covered?
    **Production readiness:** Backward compatibility? Documentation?

    ## Output Format

    ### Strengths
    ### Issues (Critical / Important / Minor)
    ### Recommendations
    ### Assessment — Ready to merge? [Yes | No | With fixes]
```

## Integration with Workflows

**tau:subagent-driven-development:** Review after EACH task.
**tau:executing-plans:** Review after each task or at natural checkpoints.
**Ad-Hoc Development:** Review before merge.

## Red Flags

**Never:**
- Skip review because "it's simple"
- Ignore Critical issues
- Proceed with unfixed Important issues
