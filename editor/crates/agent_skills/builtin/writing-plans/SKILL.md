---
name: writing-plans
description: Use when you have a spec or requirements for a multi-step task, before touching code
---

# Writing Plans

## Overview

Write comprehensive implementation plans. Assume the engineer has zero context for the codebase. Document everything: which files to touch, code, testing, how to test it. DRY. YAGNI. TDD. Frequent commits.

**Announce at start:** "I'm using the writing-plans skill to create the implementation plan."

**Save plans to:** `docs/tau/plans/YYYY-MM-DD-<feature-name>.md`

## Scope Check

If the spec covers multiple independent subsystems, suggest breaking into separate plans. Each plan should produce working, testable software on its own.

## Task Right-Sizing

A task is the smallest unit that carries its own test cycle and is worth a fresh reviewer's gate. Each task ends with an independently testable deliverable.

## Bite-Sized Task Granularity

Each step is one action (2-5 minutes):
- "Write the failing test" — step
- "Run it to make sure it fails" — step
- "Implement the minimal code to make the test pass" — step
- "Run the tests and make sure they pass" — step
- "Commit" — step

## Plan Document Header

```markdown
# [Feature Name] Implementation Plan

> REQUIRED SUB-SKILL: Use tau:subagent-driven-development or tau:executing-plans

**Goal:** [One sentence describing what this builds]
**Architecture:** [2-3 sentences about approach]
**Tech Stack:** [Key technologies/libraries]

## Global Constraints
---
```

## Task Structure

```markdown
### Task N: [Component Name]

**Files:**
- Create: `exact/path/to/file.py`
- Modify: `exact/path/to/existing.py`
- Test: `tests/exact/path/to/test.py`

**Interfaces:**
- Consumes: [what this task uses from earlier tasks]
- Produces: [what later tasks rely on]

- [ ] **Step 1: Write the failing test**
  ```python
  def test_specific_behavior():
      result = function(input)
      assert result == expected
  ```

- [ ] **Step 2: Run test to verify it fails**
  Expected: FAIL

- [ ] **Step 3: Write minimal implementation**
  ```python
  def function(input):
      return expected
  ```

- [ ] **Step 4: Run test to verify it passes**
  Expected: PASS

- [ ] **Step 5: Commit**
```

## No Placeholders

Every step must contain actual content. Plan failures:
- "TBD", "TODO", "implement later"
- "Add appropriate error handling" (without specifics)
- "Write tests for the above" (without actual test code)
- Steps that describe without showing code

## Self-Review

After writing the complete plan:
1. **Spec coverage:** Can you point to a task for each requirement?
2. **Placeholder scan:** Any "TBD" or incomplete sections?
3. **Type consistency:** Do signatures match across tasks?

Fix issues inline. No re-review needed.

## Execution Handoff

Offer execution choice: tau:subagent-driven-development (recommended) or tau:executing-plans.
