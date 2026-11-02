# Usage Guidance: Accuracy and Coverage Strategy

Status: proposal for review
Related documents: [design.md](design.md), [install-methods.md](install-methods.md), [harness-options.md](harness-options.md)

## 1. Goal

The USE section is how a user learns what to do after an application starts. “Usage exists” is not
enough; it must be **accurate** (the URL, command, and first-run step actually work for the
installed version and method) and **comprehensive** (every app category exposes the entry points
its users need).

This document defines the model, the evidence and verification process, the coverage workflow, and
the concrete changes EasyAI needs.
.

## 2. The USE model

A usage declaration covers these dimensions:

| Dimension | Content | Example |
|---|---|---|
| Web UI | URL, first-run path, whether login is required | Open WebUI `http://127.0.0.1:3000` |
| API | Base URL, OpenAI-compatible URL, OpenAPI document | Ollama `http://127.0.0.1:11434/v1` |
| CLI | Copyable commands, working directory, prerequisites | `ollama pull llama3.2` |
| First run | Pull a model, create an admin, open a workspace | Open WebUI initial setup |
| Models / data | Model directory, data directory, import method | ComfyUI `models/` |
| Documentation | Quick start and reference links | `https://docs.ollama.com` |
| Pairings | Recommended companion apps | Ollama + Open WebUI |
| Lifecycle details | Port, health endpoint, log location, stop method | `8188`, `/system_stats` |
| Differences | Per method, version, and platform | Docker `3000:8080` vs pip `8080` |

## 3. Usage must be method-scoped

The same application often exposes different entry points depending on how it was installed. A
Docker deployment may map port `3000:8080` and have no host-level CLI; a pip install may serve
`8080` and provide an `open-webui` command. A single app-level usage block cannot represent both
accurately.

Usage should therefore live on the method, with the recipe providing defaults:

```yaml
usage:                              # shared defaults
  api:
    base_url: "http://127.0.0.1:8080"

methods:
  - id: docker
    usage:
      ui_url: "http://127.0.0.1:3000"   # host side of 3000:8080
      cli: []                            # CLI exists only inside the container
  - id: pip
    usage:
      ui_url: "http://127.0.0.1:8080"
      cli:
        - label: "Start"
          command: "open-webui serve"
```

Rules:

- A method inherits the recipe-level usage and overrides what differs.
- Usage is recorded with the installed method, so later edits to another method do not change the
  guidance for an existing install.
- CLI entries should declare whether they run on the host, inside a container, or need a shell.
- URLs that depend on a port mapping must use the host-side address.

## 4. Accuracy

### 4.1 Evidence sources

Every usage item should be traceable to a source:

- the official documentation's Quick start / Usage / API sections;
- `Dockerfile` `EXPOSE` statements and `compose.yaml` port mappings;
- OpenAPI documents or framework route definitions;
- `--help` output from the released binary;
- environment-variable and configuration references;
- official curl/Python/JavaScript examples;
- release notes for behavior changes.

Usage without evidence is a draft, not a verified entry.

### 4.2 Runtime verification

Before a recipe is marked verified, install and start it in a clean environment and check:

- the web UI returns 2xx and the expected page title;
- the health endpoint returns 2xx;
- API endpoints such as `/openapi.json` or `/v1/models` respond;
- declared CLI commands exist and their `--help` runs;
- the expected port is listening and container port mappings match the host URL;
- documentation links are reachable (HEAD/GET, with retries).

Record the result:

```json
"verification": {
  "verified_at": "2026-11-02",
  "verified_version": "0.40.12",
  "verified_method": "release-linux",
  "checks": ["ui_url", "health", "cli_help", "docs_url"]
}
```

A recipe may keep usage without verification, but the UI must distinguish “documented” from
“verified”, and the catalog should report stale entries.

### 4.3 Version and method binding

Usage is not a timeless property of a repository:

- ports, paths, and CLI flags change between versions;
- different install methods have different entry points;
- platforms differ (Windows executable, macOS app bundle, Linux binary).

Every verified usage must record the version and method it was verified against. A new release or a
new method resets the entry to “needs verification”; the old guidance may still be shown with a
stale marker.

## 5. Comprehensiveness

### 5.1 Category templates

Different application categories need different usage coverage:

| Category | Required usage | Commonly missed |
|---|---|---|
| LLM runtime (Ollama, llama.cpp) | API, OpenAI-compatible URL, CLI, model pull, docs | What to do when no model is installed |
| Web UI (Open WebUI) | UI URL, first login/admin, API, data volume | Docker port mapping |
| Image generation (ComfyUI, A1111) | UI URL, model directory, workflows, extensions | Where models and custom nodes go |
| CLI tool | Commands, arguments, model path | Prerequisites, working directory |
| Desktop app | Launch path, data directory, auto-update | Entry point when there is no CLI/API |

Each category has a required-field checklist enforced by recipe validation. For example:

- web UI: `ui_url` is required;
- LLM runtime: at least one of `api` or `cli` is required;
- image generation: model/data directory notes are required;
- CLI tool: at least one CLI example is required.

### 5.2 Stable IDs for usage items

Each usage item should have a stable id (for example `api-base`, `openai-base`, `ui`, `cli-pull`,
`cli-run`). Stable ids allow:

- per-item verification and staleness;
- user feedback on a specific command or link;
- translations and documentation links;
- permission rules (for example, “open only these URLs”).

## 6. Delivery pipeline

```
repository / docs
  → deterministic extraction (ports, Dockerfile, README headings, --help)
  → draft usage with evidence links and confidence
  → schema validation
  → live install/start verification
  → human review
  → publish recipe with verification metadata
  → periodic re-verification / user feedback
```

### 6.1 Deterministic extraction

Extract candidates from:

- `Dockerfile` (`EXPOSE`), compose files (published ports), Helm values;
- OpenAPI/JSON route definitions;
- `--help`, `--version`, and man pages from the released artifact;
- README sections whose headings contain Install, Usage, Quick start, Docker, API;
- package metadata that describes binaries and default ports;
- official documentation sitemaps and quick-start pages.

### 6.2 Draft generation

The extractor produces a draft with:

- candidate values;
- the evidence URL or file path for each value;
- confidence (for example “found in Dockerfile” vs “mentioned in a blog”);
- unknown fields marked explicitly.

An optional LLM step may summarize documentation into a draft, but it must return evidence and
confidence, and its output is never treated as verified.

### 6.3 Verification runner

The verifier runs on Linux, macOS, and Windows where the method supports it:

- install into a disposable prefix or container;
- start the application and wait for the health check;
- run HTTP probes with response-time limits;
- run declared CLI commands with `--help` and timeouts;
- capture the observed ports and compare them with the declared URLs;
- store results and logs as verification evidence.

Failures block the “verified” state, not the recipe itself; the recipe can still be published as a
draft.

### 6.4 Human review

A reviewer checks:

- the category checklist is complete;
- every URL and command has evidence;
- the first-run path is covered (model pull, admin creation, data directory);
- method-specific differences are represented;
- the wording is short and actionable;
- the verification record matches the current version and method.

### 6.5 Re-verification

Trigger re-verification when:

- a new upstream release is detected;
- a method is added or changed;
- a link check fails;
- a user reports a broken usage item;
- a scheduled interval expires (for example 90 days).

## 7. Runtime fallback discovery

For applications without curated usage, EasyAI can discover likely entry points after start:

- list listening TCP ports of the application process;
- probe `/`, `/api`, `/health`, `/openapi.json`, `/v1/models`;
- read the page title and favicon;
- inspect the process command line and `--help`;
- inspect container port mappings and labels.

Present these as “detected entry points” with confidence and let the user confirm them. Confirmed
entries feed back into the recipe authoring flow. This grows coverage from real use instead of
requiring every app to be curated up front.

## 8. Coverage metrics and CI gates

Track a usage coverage dashboard:

- percentage of recipes with a usage section;
- percentage with web UI, API, and CLI coverage by category;
- percentage verified, and percentage stale;
- number of failed links, commands, and port checks;
- user-reported issues per recipe version.

CI gates:

- schema validation for usage (URLs, non-empty commands, stable ids);
- offline unit tests for URL allowlists and rendering;
- online link checks with retries (allowed to be skipped in offline CI);
- end-to-end install/start probes for a representative application per category;
- no empty placeholders or `TODO` values in a verified recipe.

## 9. LLM boundaries

The LLM is useful for:

- extracting entry points and examples from long documentation;
- compressing documentation into first-run steps;
- drafting curl/Python examples;
- explaining a failed command or log.

The LLM is not permitted to:

- decide a port or command and publish it without verification;
- write unverified usage into a verified recipe;
- execute arbitrary shell commands.

The rule is simple: **the LLM produces drafts with evidence; the verifier and the reviewer produce
accuracy.**

## 10. Recommendations for EasyAI

1. **Move `usage` to the method level**, with recipe-level defaults. Docker and pip differences
   must be expressible.
2. **Add verification metadata** to each usage item or to the recipe version: `verified_at`,
   `verified_version`, `verified_method`, `checks`.
3. **Add a `usage-check` tool** that validates URLs, commands, and ports. Run it in CI and from a
   “Verify usage” action in the UI.
4. **Add runtime discovery** for uncurated apps: detected ports and HTTP endpoints, with user
   confirmation and write-back.
5. **Create recipe templates** for LLM runtimes, web UIs, image generation, CLI tools, and desktop
   apps.
6. **Add a “Report an issue” action** on usage items that records the recipe version and method.
7. **Expose draft vs verified vs stale** in the UI, without adding noise to the normal path.

## 11. Roadmap

| Phase | Content | Outcome |
|---|---|---|
| P0 (done) | Basic usage contract, Use panel, Open/Copy, Ollama guidance | Users see what to do after start |
| P1 | Method-level usage, stable ids, verification metadata, `usage-check` | Accuracy for multi-method apps |
| P2 | Verification runner, CI gates, coverage dashboard | Coverage becomes measurable |
| P3 | Runtime discovery, LLM draft extraction, user feedback loop | Coverage grows continuously |

## 12. Acceptance criteria

- Every bundled application has method-scoped usage with stable ids.
- Every verified usage item records version, method, timestamp, and checks.
- The UI distinguishes draft, verified, and stale entries.
- `usage-check` fails on broken URLs, missing commands, and port mismatches.
- A new application can be added with a category template and pass the checklist without ad-hoc
  decisions.
- User feedback on a usage item is attached to the recipe version and method.

Concise UI copy applies here as elsewhere: the panel shows entry points and actions, not
implementation details.
