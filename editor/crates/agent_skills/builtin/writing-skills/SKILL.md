---
name: writing-skills
description: Use when creating new skills, editing existing skills, or verifying skills work before deployment
---

# Writing Skills

## Overview

**Writing skills IS Test-Driven Development applied to process documentation.**

Skills live in `~/.agents/skills/<name>/SKILL.md` (global) or `<project>/.agents/skills/<name>/SKILL.md` (project-local).

**Core principle:** If you didn't watch an agent fail without the skill, you don't know if the skill teaches the right thing.

**REQUIRED BACKGROUND:** You MUST understand tau:test-driven-development before using this skill.

## What is a Skill?

A **skill** is a reusable set of instructions that an agent can load on demand. Each skill lives in its own directory and is defined by a `SKILL.md` file with YAML frontmatter.

## TDD Mapping for Skills

| TDD Concept | Skill Creation |
|-------------|----------------|
| Test case | Pressure scenario with subagent |
| Production code | Skill document (SKILL.md) |
| Test fails (RED) | Agent violates rule without skill |
| Test passes (GREEN) | Agent complies with skill present |
| Refactor | Close loopholes while maintaining compliance |

## SKILL.md Format

```markdown
---
name: my-skill-name
description: Use when [specific triggering conditions and symptoms]
---

# Skill Name

Instructions for the agent go here.
```

### Required Frontmatter

- **name** (required): 1-64 chars, lowercase alphanumeric with hyphens. Must match directory name. Regex: `^[a-z0-9]+(-[a-z0-9]+)*$`
- **description** (required): 1-1024 chars. Start with "Use when..." Only describe triggering conditions, NOT the skill's process/workflow.

### Where Skills Live

| Scope | Path |
|-------|------|
| Global | `~/.agents/skills/<name>/SKILL.md` |
| Project | `<project>/.agents/skills/<name>/SKILL.md` |

## Skill Discovery Optimization (SDO)

**Critical: Description = When to Use, NOT What the Skill Does**

```yaml
# ❌ BAD: Summarizes workflow
description: Use when executing plans - dispatches subagent per task with review

# ✅ GOOD: Just triggering conditions
description: Use when executing implementation plans with independent tasks
```

**Keyword Coverage:** Use words an agent would search for — error messages, symptoms, synonyms, tools.

**Descriptive Naming:** Use active voice, verb-first: `creating-skills` not `skill-creation`.

## The Iron Law (Same as TDD)

```
NO SKILL WITHOUT A FAILING TEST FIRST
```

Write skill before testing? Delete it. Start over. Edit skill without testing? Same violation.

## Bulletproofing Skills

- Close every loophole explicitly — forbid specific workarounds
- Address "spirit vs letter" arguments with foundational principle
- Build rationalization table from baseline testing
- Create red flags list for self-checking

## Skill Creation Checklist

**RED:** Create pressure scenarios, run WITHOUT skill, document baseline behavior
**GREEN:** Write minimal skill, address specific baseline failures, verify WITH skill
**REFACTOR:** Identify new rationalizations, add explicit counters, re-test

## Anti-Patterns

- **Narrative examples** — too specific, not reusable
- **Multi-language dilution** — one excellent example beats many mediocre ones
- **Code in flowcharts** — can't copy-paste
- **Generic labels** — use semantic meaning
