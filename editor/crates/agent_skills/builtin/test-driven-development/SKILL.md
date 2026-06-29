---
name: test-driven-development
description: Use when implementing any feature or bugfix, before writing implementation code
---

# Test-Driven Development (TDD)

## Overview

Write the test first. Watch it fail. Write minimal code to pass.

**Core principle:** If you didn't watch the test fail, you don't know if it tests the right thing.

## The Iron Law

```
NO PRODUCTION CODE WITHOUT A FAILING TEST FIRST
```

Write code before the test? Delete it. Start over. No exceptions.

## Red-Green-Refactor

### RED — Write Failing Test

Write one minimal test showing what should happen:
- One behavior per test
- Clear name describing behavior
- Real code (no mocks unless unavoidable)

### Verify RED — Watch It Fail

**MANDATORY. Never skip.**
Run the test. Confirm: test fails (not errors), failure message is expected, fails because feature missing (not typos).

### GREEN — Minimal Code

Write simplest code to pass the test. Don't add features, refactor, or "improve" beyond the test.

### Verify GREEN — Watch It Pass

**MANDATORY.** Run the test. Confirm: test passes, other tests still pass, output pristine.

### REFACTOR — Clean Up

After green only: remove duplication, improve names, extract helpers. Keep tests green.

### Repeat

Next failing test for next feature.

## Good Tests

| Quality | Good | Bad |
|---------|------|-----|
| Minimal | One thing. "and" in name? Split it | `test('validates email and domain')` |
| Clear | Name describes behavior | `test('test1')` |
| Shows intent | Demonstrates desired API | Obscures what code should do |

## Why Order Matters

Tests written after code pass immediately. Passing immediately proves nothing. Test-first forces you to see the test fail, proving it actually tests something.

## Common Rationalizations

| Excuse | Reality |
|--------|---------|
| "Too simple to test" | Simple code breaks. Test takes 30 seconds. |
| "I'll test after" | Tests passing immediately prove nothing |
| "Already manually tested" | Manual is ad-hoc, not systematic |
| "Deleting X hours is wasteful" | Sunk cost fallacy |
| "TDD will slow me down" | TDD faster than debugging |

## Example: Bug Fix

**RED:**
```python
def test_rejects_empty_email():
    result = submit_form({"email": ""})
    assert result["error"] == "Email required"
```

**Verify RED:** Test fails — expected 'Email required', got nothing.

**GREEN:**
```python
def submit_form(data):
    if not data.get("email", "").strip():
        return {"error": "Email required"}
    # ...
```

**Verify GREEN:** Test passes.

## Verification Checklist

- [ ] Every new function has a test
- [ ] Watched each test fail before implementing
- [ ] Wrote minimal code to pass each test
- [ ] All tests pass, output pristine
- [ ] Tests use real code (not mocks)
- [ ] Edge cases covered

## Red Flags

**All of these mean: Delete code. Start over with TDD.**
- Code before test
- Test passes immediately
- "I already manually tested it"
- "Tests after achieve the same purpose"
- "This is different because..."

## Final Rule

```
Production code → test exists and failed first
Otherwise → not TDD
```
