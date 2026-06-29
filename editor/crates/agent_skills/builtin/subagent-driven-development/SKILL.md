---
name: subagent-driven-development
description: Use when executing implementation plans with independent tasks in the current session
---

# Subagent-Driven Development

Execute plan by dispatching a fresh implementer subagent per task, a task review (spec compliance + code quality) after each, and a broad whole-branch review at the end.

**Core principle:** Fresh subagent per task + task review + broad final review = high quality, fast iteration.

**Announce at start:** "I'm using the subagent-driven-development skill to execute this plan."

**Continuous execution:** Do not pause to check in between tasks. Execute all tasks without stopping. The only reasons to stop are: BLOCKED status you cannot resolve, all tasks complete.

## When to Use

vs tau:executing-plans (parallel session):
- Same session, fresh subagent per task
- Review after each task, broad review at the end
- Faster iteration (no human-in-loop between tasks)

## The Process

1. **Read plan** — note context and global constraints, create todos
2. **For each task:**
   - Dispatch implementer subagent (template below)
   - If implementer asks questions, answer them
   - Implementer implements, tests, commits, self-reviews
   - Dispatch task reviewer subagent (template below)
   - If reviewer finds issues, dispatch fix subagent
   - Mark task complete
3. **After all tasks:** Dispatch final code reviewer
4. **Use tau:finishing-a-development-branch**

## Model Selection

Use the least powerful model that can handle each role:
- **Mechanical tasks** (isolated functions, clear specs): fast/cheap model
- **Integration tasks** (multi-file coordination): standard model
- **Architecture/design tasks**: most capable available model
- **Review tasks**: scale to diff's complexity

**Always specify the model explicitly when dispatching a subagent.**

## Implementer Prompt Template

```
Task (general-purpose):
  description: "Implement Task N: [task name]"
  model: [MODEL]
  prompt: |
    You are implementing Task N: [task name]

    ## Task Description

    Read your task brief first: [BRIEF_FILE]

    ## Context

    [Scene-setting: where this fits, dependencies, architectural context]

    ## Before You Begin

    If you have questions about requirements, approach, or anything unclear,
    ask them now. Raise any concerns before starting work.

    ## Your Job

    1. Implement exactly what the task specifies
    2. Write tests (following TDD if applicable)
    3. Verify implementation works
    4. Commit your work
    5. Self-review (completeness, quality, YAGNI, testing)
    6. Report back

    Work from: [directory]

    ## Report Format

    Write your full report to [REPORT_FILE]:
    - What you implemented
    - What you tested and test results
    - TDD Evidence (if required): RED + GREEN verification
    - Files changed
    - Self-review findings
    - Any concerns

    Then report back with:
    - Status: DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
    - Commits created
    - One-line test summary
    - Concerns, if any
```

## Task Reviewer Prompt Template

```
Task (general-purpose):
  description: "Review Task N (spec + quality)"
  model: [MODEL]
  prompt: |
    You are reviewing one task's implementation.

    ## What Was Requested

    Read the task brief: [BRIEF_FILE]

    Global constraints: [GLOBAL_CONSTRAINTS]

    ## Diff Under Review

    Base: [BASE_SHA]
    Head: [HEAD_SHA]

    ## Part 1: Spec Compliance

    - Missing: requirements skipped or missed
    - Extra: features not requested (YAGNI violations)
    - Misunderstood: right feature built wrong way

    ## Part 2: Code Quality

    - Clean separation of concerns?
    - Proper error handling?
    - Edge cases handled?
    - Tests verify real behavior, not mocks?

    ## Calibration

    Critical = must fix, Important = should fix, Minor = nice to have.
    Acknowledge strengths before listing issues.

    ## Output Format

    ### Spec Compliance
    ### Strengths
    ### Issues (Critical / Important / Minor)
    ### Assessment — [Approved | Needs fixes]
```

## Handling Implementer Status

- **DONE:** Generate review package, dispatch reviewer
- **DONE_WITH_CONCERNS:** Read concerns before proceeding
- **NEEDS_CONTEXT:** Provide missing context, re-dispatch
- **BLOCKED:** Assess blocker, re-dispatch with more capable model or break task

## Durable Progress

Track progress in todos. After compaction, trust git log and todos over recollection.

## Red Flags

**Never:** Start on main/master without consent, skip task review, proceed with unfixed issues, dispatch multiple implementers in parallel, skip scene-setting context, ignore subagent questions.

## Required Workflow Skills
- **tau:writing-plans** — Creates the plan
- **tau:requesting-code-review** — Code review template for final review
- **tau:finishing-a-development-branch** — Complete development after all tasks
- **tau:test-driven-development** — Subagents follow TDD for each task
