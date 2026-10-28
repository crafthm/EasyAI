# Harness Options for EasyAI

In this project, “harness” is understood as **a controlled tool set + context + an execution/verification loop**. The three options below are not three products; they are three ways to drive the same tool substrate, ordered by increasing automation and risk.

---

## Option A: Deterministic Harness (executor-driven)

**Who decides:** rules, recipes, and the state machine. No LLM participates in the loop.

```
tile clicked → state machine reads current state → resolver selects an install method by platform/arch/runtime
             → job executes steps → health check → persist state → update tile
```

**Shape in this project:**

- Tools are install, start, stop, uninstall, status, and read-log.
- Each application has a recipe declaring its install source, start command, health check, and what uninstall must preserve.
- On failure the system reports only which step failed, the exit code, and where the log is; it does not reason about the cause.
- Tiles only display state and trigger operations.

**What it solves:** installation, start/stop, and uninstall for standard cases; works offline; same input produces the same result; every step is unit-testable; least privilege.

**What it cannot solve:** failures it has not seen (CUDA version mismatch, Python dependency conflicts, port already in use), natural-language configuration intent, and orchestration across multiple applications.

**Cost and risk:** lowest development cost and highest testability; the price is recipe maintenance and leaving the user to read logs when a novel failure occurs.

**When to use:** now. It is the foundation for the other two options; every harness ultimately lands on these deterministic operations.

---

## Option B: Read-only Diagnostic Harness (observer-driven)

**Who decides:** the deterministic engine executes; the LLM only observes and explains; the human makes the final decision.

```
job fails → collect log + probe results + recipe + current state
         → LLM returns: most likely causes, evidence, suggested action
         → user clicks the suggested action in the UI → still executed by Option A
```

**Key constraint:** the tool set given to the model contains **read operations only** — read log, read application detail, read hardware probe, read recipe. It has no permission to call install/start/stop, so even if a log contains a prompt injection, the worst outcome is a bad suggestion, not a changed machine.

**Shape in this project:**

- The detail panel gains a Diagnose button, shown only after a failure.
- Output is structured: cause, evidence (log line or probe item), suggested action, confidence.
- Suggested actions map directly to existing buttons; no new execution path is introduced.
- Defaults to a local model (for example, an already installed Ollama) with optional cloud providers; the feature is hidden when nothing is configured.

**What it solves:** turns “reading the log”, the hardest part of the experience, into a readable explanation; substantially reduces support cost; accumulates real failure samples and prompts for Option C.

**What it cannot solve:** it cannot fix anything automatically, cannot change configuration, and may misattribute the cause. Evidence must therefore be shown, not just a conclusion.

**Cost and risk:** low cost, invoked only on failure. The main risks are misdiagnosis and sensitive data in logs, which require redaction and evidence display. Testability is moderate: build an offline evaluation set from seeded failures.

**When to use:** after Option A is stable. This is the highest return on investment and the safest entry point for adopting a harness.

---

## Option C: Guarded Agent Harness (goal-driven, confirmation-gated)

**Who decides:** the LLM plans and calls tools in a loop, but every machine-changing action passes through a permission gate.

```
user goal → model plans → calls read-only tools to gather context
         → proposes a mutating operation (install/start/config change)
         → UI shows an operation card with arguments or a config diff
         → user confirms → execute → observe → diagnose on failure → next step
```

**Shape in this project:**

- The tool set is identical to Options A/B; the model only gains the ability to propose changes. The backend still executes registered tools only; there is no arbitrary shell.
- Every mutating operation has a confirmation card. Auto-approval can be enabled per operation type (for example, start/stop only); install and uninstall always require confirmation by default.
- Configuration changes must be schema-constrained patches, shown as a diff before writing, with automatic rollback on failure.
- Step limits, token/time budgets, and a cancel control. Every operation records `origin=assistant` and shares the same log format as manual operations.
- Typical tasks: “get ComfyUI running on 8 GB of VRAM”, “move the model directory to the large disk”, “it will not open after installation — investigate”.

**What it solves:** ambiguous goals, multi-step recovery, natural-language configuration, and cross-application orchestration — exactly the areas rules cannot cover.

**What it cannot solve / new risks:**

- Non-determinism: the same goal may take different paths, making reproduction and testing hard.
- Prompt injection: READMEs, logs, and model output can all carry instructions; blast radius must be bounded by tool allowlists and confirmation gates.
- Error compounding: one misjudgement can steer later steps; every step must be observable and reversible.
- Cost and latency: multiple model calls per task; unusable offline.
- Evaluation difficulty: requires tool-call evaluations and fault injection rather than ordinary unit tests.

**When to use:** after Option B is validated. Start with configuration patches as the only mutating capability, then gradually open installation and upgrade orchestration.

> Option C has a dangerous variant: **fully autonomous harness** (all operations auto-approved). It amplifies every risk above and offers limited additional benefit; it should not be enabled by default in any release.

---

## Comparison

| Dimension | A — Deterministic | B — Read-only diagnosis | C — Guarded agent |
|---|---|---|---|
| Driver | State machine / rules | Human | LLM + human confirmation |
| LLM permissions | None | Read-only | Read + propose mutations |
| Reproducibility | High | High (execution side) | Low |
| Automation | Medium | Low | High |
| Security risk | Low | Low | Medium |
| Works offline | Yes | With a local model | With a local model |
| Testing approach | Unit/integration tests | Failure-sample evaluation | Tool-call evals + fault injection |
| Development cost | Low | Medium | High |
| Best for | Standard install/start/stop | Explaining failures | Ambiguous goals, configuration, multi-step recovery |

## Task-to-option mapping

| Task | Option to use |
|---|---|
| Choosing an install method | A (the LLM may explain the rationale at most) |
| Executing install/start/stop/uninstall | A (C may only propose) |
| Analyzing a failure | B |
| Natural-language configuration | C (B can only suggest) |
| Multi-step recovery / cross-app orchestration | C |
| Remote batch operations | C (later) |

## Recommended sequence

1. **This change = Option A**, while establishing the tool contract Option C will need (registry, `origin`/confirmation guard, audit) so no rework is required later.
2. **Next change = Option B**: provider configuration, failure diagnosis, evidence display, with a read-only tool set.
3. **Following change = Option C**: start with configuration patches for a single application (schema + diff + rollback); open installation and upgrade orchestration only after the confirmation flow is proven.

In short: **A is mandatory, B is the low-cost high-return entry point, and C is the target shape but cannot skip the first two steps.**