# EasyAI Design Proposal

Status: draft for review
Scope: the full product described below; milestone 1 implements a vertical slice of it.

## 1. Problem and goal

Installing, upgrading, running, and configuring AI open-source software is hard: every project
picks a different distribution channel (installer script, PyPI, GitHub release, Docker, source
build), GPU/Python/CUDA combinations differ per machine, and configuration is spread across
files, environment variables, and CLI flags.

EasyAI is a desktop application that turns "get this AI tool working on this machine" into a
guided, reversible operation, and extends the same operations to other machines on the LAN.

### Goals

1. **Best-fit installation.** Probe the machine, choose an install method, explain why, execute it
   with visible progress, and support clean uninstall.
2. **Upgrade.** Detect new versions, show release notes, upgrade with health check and rollback.
3. **Start/stop.** One-click lifecycle that survives closing the EasyAI window.
4. **LLM-assisted configuration.** Natural-language intent becomes a validated, previewable
   configuration change — never arbitrary shell.
5. **Fleet.** Discover x86/ARM machines on the LAN and perform the same operations remotely.

### Non-goals (v1)

- Training platforms, cloud GPU rental, model marketplaces.
- Arbitrary software installation; the catalog is curated.
- Running AI workloads inside EasyAI itself; EasyAI is a controller, not a runtime.

## 2. Key decisions

| # | Decision | Alternatives considered | Why |
|---|----------|------------------------|-----|
| D1 | Apps are described by **declarative recipes** in an external, signed catalog | Hardcode each app in Rust | New apps ship without an app release; resolver and UI stay generic |
| D2 | Install to **user space by default** (`~/.local/share/easyai/apps/<id>/<version>`), system-wide only when a recipe requires it | Always system-wide / always Docker | No root for the common case; clean uninstall; Docker is not always present |
| D3 | **OS-native service supervision** (systemd user unit, launchd agent, Windows scheduled task/service) with a detached-process fallback | Supervise children inside the app | Requirement 3: services must keep running after the window closes |
| D4 | First-run **remote management over SSH**, upgradeable to a persistent agent | Agent-only from day one | SSH exists on Linux/macOS and needs no new listening port; agent solves boards without SSH later |
| D5 | LLM may only produce **schema-constrained config patches** | Free-form shell/config generation | Safe, reviewable, rollback-able; no hallucinated commands |
| D6 | **Typed install steps** (`download`, `extract`, `run`, `pip_install`, `docker_run`, `write_file`, `service`) with a `run` escape hatch | Pure shell scripts / fully typed DSL | Typed steps enable progress, dry-run, and checksums; the escape hatch keeps recipes writable |
| D7 | Resolver is a **deterministic rule engine**; the LLM may explain or advise but never selects silently | LLM picks the method | Reproducible, testable, works offline |
| D8 | State in SQLite, logs and user files on disk | Everything in the DB | Logs are large and should be tail-able/attachable |
| D9 | **Each tile is one catalog application** and the lifecycle control surface; the grid is the home screen | Separate catalog page and installed-only grid | Matches the product model; installation starts where the application lives |
| D10 | **Layered harness**: deterministic recipe tools are the execution substrate; an optional LLM harness may only propose, diagnose, and patch configuration | Agent-driven execution core | Reproducibility, safety, offline operation; intelligence is most useful where rules are incomplete |
| D11 | **Install methods are ranked**: release binaries, containers, package managers, and portable bundles come before source builds; source is a fallback | One method per app / source-first | Most Git repos expose faster, more isolated, more reproducible channels; source builds are slow and fragile. See [install-methods.md](install-methods.md) |

## 3. Architecture

```
┌──────────────────────────── EasyAI desktop (Tauri) ────────────────────────────┐
│                                                                                │
│  React UI (src/ui)                    Tauri commands + events                   │
│  Home · Catalog · App detail   ◄────►  probe/catalog/install/job/config/...     │
│  Machines · Activity · Settings                                                │
│                                         │                                      │
│                                         ▼                                      │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌───────────────────┐   │
│  │   catalog    │  │    probe     │  │   resolve    │  │   job engine      │   │
│  │ recipes +    │  │ os/arch/gpu/ │  │ method       │  │ queue, logs,      │   │
│  │ signature    │  │ runtime/net  │  │ scoring      │  │ cancel, rollback  │   │
│  └──────────────┘  └──────────────┘  └──────────────┘  └─────────┬─────────┘   │
│                                                                  ▼             │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌───────────────────┐   │
│  │   update     │  │    config    │  │     llm      │  │   exec layer      │   │
│  │ version src, │  │ schema read/ │  │ patch gen +  │  │ shell, elevation, │   │
│  │ upgrade,     │  │ write, diff, │  │ validation   │  │ download, stream  │   │
│  │ rollback     │  │ backup       │  │              │  │                   │   │
│  └──────────────┘  └──────────────┘  └──────────────┘  └─────────┬─────────┘   │
│                                                                  ▼             │
│  ┌─────────────────────────── service layer ────────────────────────────────┐  │
│  │ systemd --user · launchd · Windows task/service · detached process       │  │
│  └───────────────────────────────────────────────────────────────────────────┘ │
│                                                                                │
│  ┌─────────────────────────── remote layer ─────────────────────────────────┐  │
│  │ discovery (mDNS / ARP+port scan / manual) → SSH transport → agent proto  │  │
│  └───────────────────────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────────┘
            │ SSH (v1)                          │ mTLS + pairing (v2)
            ▼                                   ▼
   remote Linux/macOS host              easyai-agent (headless, same engine)
```

The engine is a Rust library (`src/lib.rs`) used by both the desktop binary and a headless
`easyai-agent` binary, so probe/resolve/install/service logic is identical locally and remotely.

## 4. Core concepts

### 4.1 Recipe

A recipe is YAML/JSON validated by a JSON Schema and distributed through a signed catalog. It
declares *what* an app needs, not *how one specific machine* will install it. A recipe carries
several ranked install methods; the full taxonomy, discovery signals, and scoring model are in
[install-methods.md](install-methods.md).

```yaml
id: comfyui
name: ComfyUI
description: Node-based image generation
glyph: C
color: "linear-gradient(135deg, #f59e0b, #f97316)"
platforms: [linux-x86_64, macos-aarch64, windows-x86_64]
methods:
  - id: windows-portable-nvidia
    kind: portable
    when: { os: [windows], gpu: [cuda] }
    prefer: 90
    source: { repo: comfyanonymous/ComfyUI, asset: ComfyUI_windows_portable_nvidia.7z }
    version: { type: github-release, repo: comfyanonymous/ComfyUI }
    service:
      command: ["{app_dir}/python_embeded/python.exe", "-s", "ComfyUI/main.py", "--port", "8188"]
      health_url: "http://127.0.0.1:8188/system_stats"
    uninstall: { keep_paths: ["{app_dir}/models"] }

  - id: container
    kind: container
    when: { runtime: [docker, podman], gpu: [cuda, rocm, cpu] }
    prefer: 70
    image:
      image: ghcr.io/example/comfyui
      tag: "{version}"
      volumes: ["{app_dir}/models:/app/models"]
      ports: ["8188:8188"]
      gpu: auto
      restart: unless-stopped
    version: { type: image, image: ghcr.io/example/comfyui }
    service:
      command: []
      health_url: "http://127.0.0.1:8188/system_stats"
    uninstall: { native: true }

  - id: uv-venv
    kind: source
    advanced: true
    when: { os: [linux, macos], toolchain: [git, python], min_disk_mb: 20480 }
    prefer: 60
    source:
      git: https://github.com/comfyanonymous/ComfyUI
      ref: "{version}"
      build:
        commands:
          - [uv, venv, --python, "3.11", .venv]
          - [.venv/bin/pip, install, torch, torchvision, torchaudio, --index-url, "{torch_index}"]
          - [.venv/bin/pip, install, -r, requirements.txt]
    version: { type: github-release, repo: comfyanonymous/ComfyUI }
    service:
      command: ["{app_dir}/src/.venv/bin/python", "main.py", "--listen", "127.0.0.1", "--port", "8188"]
      health_url: "http://127.0.0.1:8188/system_stats"
    uninstall: { keep_paths: ["{app_dir}/models"] }
```

Rules:

- `{torch_index}` and similar variables are filled by the resolver from the GPU probe
  (CUDA version → PyTorch index URL, ROCm, MPS, CPU).
- `when` clauses are **hard constraints**; `prefer` is a score. Methods are tried in score order;
  source builds are the fallback, not the default.
- Every method must declare its version source, uninstall behavior, and prerequisites; nothing is
  inferred.
- Before installing, probe for existing copies across methods (PATH binary, package manager,
  container, version directory) and never create a second unmanaged copy.
- Methods that require elevation, run an unpinned script, or build from a moving branch are ranked
  lower and require an explicit advanced opt-in.
- Recipes are versioned; an installed app records the recipe version and the chosen method.

Catalog handling: a bundled snapshot for offline/first run, refreshed from a signed remote index
(ed25519/minisign) with a pinned public key. New catalog versions can add apps and fix recipes
without shipping a new app binary.

### 4.2 Probe and resolver

`probe::collect()` produces a `HardwareProfile`:

- OS, distro, kernel, arch (`uname -m`), libc (glibc/musl), package manager
- CPU cores, RAM, free disk, virtualization
- GPU: NVIDIA (driver, CUDA version, VRAM via `nvidia-smi`), AMD (ROCm), Intel, Apple
  (Metal/MPS), CPU fallback
- Runtimes: Docker/Podman (and whether GPU passthrough works), Python/uv, git, brew/winget
- Network: reachability/latency of GitHub, PyPI, Hugging Face, Docker Hub and configured mirrors

`resolve(recipe, profile, preferences) -> InstallPlan`:

```rust
struct InstallPlan {
    method_id: String,
    steps: Vec<Step>,          // fully interpolated, dry-runnable
    rationale: Vec<String>,    // e.g. "NVIDIA GPU with CUDA 12.4 → torch cu124 wheels"
    rejected: Vec<Rejection>,  // method + reason, shown in the UI
    download_bytes: Option<u64>,
    needs_elevation: bool,
}
```

Scoring = hard-filter on `when` (OS/arch, GPU/runtime, toolchain, elevation policy, disk) → score
by upstream support, isolation and clean uninstall, version determinism, update path, download or
build cost, and service manageability. Source builds score low unless no other method exists.
Unit-testable with fixture profiles; no network or shell calls in the resolver. See the preference
tiers in [install-methods.md](install-methods.md#4-scoring-model).

The UI always shows the chosen method, a one-line "why", and the rejected alternatives before
the user confirms.

### 4.3 Job engine

All state-changing work runs as a `Job`:

- States: `queued → running → succeeded | failed | cancelled`; one active job per machine,
  FIFO queue so installs on the same machine never interleave.
- Every step is logged with the exact command, env, exit code, and duration to
  `logs/<machine>/<job_id>.log`, and streamed to the UI via a Tauri event (`job://{id}/log`).
- Cancellation kills the whole process group; steps that create state declare an inverse step
  used for rollback.
- A failed job leaves the machine in a known state: the version directory is staged under
  `versions/<v>.tmp` and only activated (symlink/pointer switch) after the health check passes.

### 4.4 Lifecycle

Service kinds a recipe can declare:

1. **`systemd-user`** (Linux) — a generated unit under `~/.config/systemd/user/`, enabled with
   `systemctl --user`, linger enabled when the user wants boot start.
2. **`launchd`** (macOS) — a LaunchAgent plist.
3. **`windows-task`** (Windows) — a scheduled task at logon, or a service for system installs.
4. **`container`** — Docker/Podman with `--restart unless-stopped`.
5. **`managed-process`** — fallback: detached process with PID file and log file; reports
   "running outside a supervisor" in the UI.

`status` = supervisor state + health check + installed version. Start/stop/restart map to the
supervisor; logs are read from the journal/file and streamed to the UI.

#### 4.4.1 Tile-centric surface (first slice)

Each tile in the main grid represents exactly one application from the catalog, including one
that is not yet installed. The tile is the primary control surface:

- state text is always visible: `not_installed`, `installing`, `stopped`, `starting`, `running`,
  `stopping`, `failed`, `uninstalling`;
- the primary action follows the state — Install, Start, or Stop — and is disabled during
  transitional states;
- the tile body opens a detail panel with version, install method, install path, health, job
  history with log access, a configuration entry point, and Uninstall.

The first slice implements install, start, stop, status, and uninstall for one recipe (Ollama) on
Linux. Installation is user-space, the process is detached and survives closing the window, and
uninstall keeps user data declared by the recipe. State is reconciled from process liveness and
the health endpoint, not trusted from SQLite alone.

Lifecycle operations are exposed through a typed tool registry. Mutating tools record `origin`
(`ui` or `assistant`), and assistant-originated mutations require explicit confirmation. This is
the boundary that keeps a future assistant on the same deterministic path as the UI.

#### 4.4.2 Importing application recipes from a URL

The grid is not limited to the bundled catalog. The `+` action accepts a recipe URL, or a GitHub
repository that publishes `easyai.json` or `.easyai/recipe.json`. Only the recipe file is fetched;
the repository is never cloned. The app validates the recipe and shows a preview of the source,
release asset, start command, health check, and uninstall behavior before anything is written.

An imported recipe is stored in `<app-data>/catalog/imported/<id>.json` together with its source
URL and sha256, and behaves exactly like a bundled recipe afterwards. Import rules:

- `https` only (plain `http` is accepted only for loopback development servers);
- maximum 512 KB, with redirects limited;
- the sha256 shown in the preview is re-checked when the recipe is imported;
- imported recipes cannot override a bundled recipe, require elevation, or use an install kind
  the engine does not support;
- the current platform must be supported;
- recipes are never updated automatically, and can be removed from the catalog only while the
  application is not installed.

### 4.5 Versions and upgrades

`version_source` options: GitHub releases, PyPI, OCI registry tags, HTTP JSON endpoint, or a
version command (`ollama --version`). Poll on launch and every N hours; compare semantically
when possible.

Upgrade flow: stage new version alongside the old → health check on a temporary port → switch
`current` pointer → restart → verify → keep the previous version for one-click rollback. Config
files are backed up before any upgrade. Release notes are fetched from GitHub releases and shown.

### 4.6 Configuration and LLM assist

Configuration is **schema-first**: the recipe declares the file/env/flag surface and a JSON
Schema for it. EasyAI can always show a form and validate edits without any LLM.

LLM flow (opt-in, user supplies a provider key or local Ollama):

1. User states intent in natural language.
2. Prompt = config schema + current values (secrets redacted) + hardware profile + app version.
3. Model returns a JSON patch constrained to the schema.
4. EasyAI validates the patch, renders a diff with per-change explanations, and applies it only
   after confirmation — with backup, restart if required, health check, and automatic rollback
   on failure.
5. If the model proposes keys outside the schema, the patch is rejected (never executed).

Providers: any OpenAI-compatible endpoint, Anthropic, or local Ollama. Default: local Ollama if
detected, otherwise LLM features are off.

The same assistant is used read-only for diagnosis: a failed job's log plus the hardware profile
can be summarized into likely causes and suggested fixes, without executing anything.

**Harness decision.** EasyAI adopts a layered hybrid rather than an agent-driven execution core.
The deterministic recipe engine executes every machine change; the tool contract is the shared
interface; an optional agent harness sits on top for read-only diagnosis, schema-validated
configuration patches, and confirmation-gated proposals. Autonomous installation is never
enabled by default. The three candidate harness models and their trade-offs are documented in
[harness-options.md](harness-options.md).

### 4.7 Remote fleet

**Discovery**

- mDNS advertisement `_easyai._tcp` from the agent (v2).
- Subnet sweep on common ports (22, 5985, 3389) combined with the ARP table; SSH banner and
  `uname -srm` identify OS/arch. This works before any agent exists.
- Manual add by hostname/IP.

**Transport, phased**

- **v1 (SSH):** upload/boot a small `easyai-agent` binary over SSH (falling back to a generated
  bootstrap script when the binary cannot run, e.g. old glibc), then drive it with JSON-RPC over
  SSH stdio. The agent does probing, installs, service setup, and log streaming on the target.
- **v2 (agent):** persistent agent with mTLS and a one-time pairing code, discovery via mDNS,
  outbound connection to the controller (no inbound port), so boards without SSH can join.

**Scope of remote operations:** the same recipe engine; the UI shows a machine × app matrix.
Batch operations are grouped jobs with per-machine failure isolation.

**Security:** host-key pinning, credentials in the OS keychain (`keyring`), explicit consent per
operation, no password storage in SQLite, and an audit log of every remote command.

### 4.8 Network and mirror strategy

Installers frequently need GitHub, PyPI, Hugging Face, and container registries, which may be
slow or blocked. The probe measures reachability; the resolver fills mirror variables
(`{pip_index}`, `{hf_endpoint}`, `{github_proxy}`, `{docker_registry_mirror}`) from settings,
with sensible auto-selection and a global proxy override. Downloads support resume and
checksum verification. An offline mode uses only cached artifacts.

## 5. Data model (SQLite)

| Table | Purpose |
|-------|---------|
| `machines` | id, name, kind (`local`/`ssh`/`agent`), os, arch, address, agent_version, status, last_seen |
| `installed_apps` | PK(machine_id, recipe_id): app_version, recipe_version, method_id, install_prefix, service_kind, service_ref, state, timestamps |
| `jobs` | id, machine_id, recipe_id, kind, state, plan_json, error, log_path, timestamps |
| `recipes` | cached catalog entries: recipe_id, version, source, json, fetched_at |
| `machines_secrets` | key reference only; secret material lives in the OS keychain |
| `settings` | key/value (LLM provider, mirrors, proxy, catalog channel) |

## 6. Repository and module layout

```
src/
  main.rs                 # Tauri bootstrap (currently also holds app setup)
  lib.rs                  # engine entry point, shared by desktop and agent
  commands/               # Tauri commands, thin wrappers over the engine
  domain/                 # Recipe, HardwareProfile, InstallPlan, Job, Machine, InstalledApp
  catalog/                # bundled snapshot, remote index, signature verification
  probe/                  # os, cpu, memory, disk, gpu, runtimes, network
  resolve/                # constraint filtering + scoring
  exec/                   # command runner, elevation, download, streaming logs
  install/                # step interpreters (script, binary, package, pip, container, git)
  service/                # systemd, launchd, windows, container, detached process
  update/                 # version sources, upgrade, rollback
  config/                 # schema-driven read/write, diff, backup
  llm/                    # provider clients, prompt builders, patch validation
  remote/                 # discovery, ssh transport, agent protocol
  db/                     # schema, migrations, queries
  bin/easyai-agent.rs     # headless agent (feature-gated build)
src/ui/src/
  App.tsx                 # shell + routing
  api/                    # invoke wrappers, event subscriptions
  components/             # existing grid, toolbar, popovers, new views
  hooks/                  # useApps, useJobs, useMachines
  types/                  # mirrors of Rust serde types (consider ts-rs later)
docs/                     # design, recipe schema, catalog policy
```

## 7. UI shape

- **Home:** one tile per catalog application. The tile shows state and the primary action
  (Install/Start/Stop); the body opens the detail panel. The grid shows installed and
  not-yet-installed applications together.
- **Catalog:** implicit in the home grid for the first slice; later adds search, categories, and
  the install-plan preview.
- **App detail:** version, install method, install path, health, job history with logs,
  configuration entry point, Start/Stop, and uninstall (keeping user data).
- **Machines:** local machine plus discovered fleet; per-machine status and app matrix; add
  remote by pairing code or SSH.
- **Activity:** running and past jobs with live logs and cancel.
- **Settings:** LLM provider/key/model, catalog channel, mirrors/proxy, data locations.

## 8. Security model

- Recipes are signed; unsigned recipes require an explicit developer-mode toggle.
- The desktop app does not run as root. Elevation is requested per operation through a narrow
  helper (polkit/pkexec on Linux, admin prompt on macOS, UAC on Windows), and every elevated
  command is logged.
- Remote credentials live in the OS keychain; host keys are pinned on first use.
- The LLM never sees secrets and never produces executable commands; only schema-validated
  config patches.
- Downloads are checksum-verified when the recipe provides a digest.

## 9. Testing and verification

- **Resolver:** table-driven tests over fixture hardware profiles; no system calls.
- **Recipes:** schema validation + lint in CI; each recipe has a smoke test that runs install →
  health check → uninstall in a container for Linux, plus macOS/Windows runners where cheap.
- **Step generation:** snapshot tests for generated commands/units.
- **Job engine:** failure-injection tests (network drop, non-zero exit, cancel mid-download,
  health check timeout) asserting rollback leaves a clean state.
- **Remote:** SSH against disposable containers/VMs; agent protocol conformance tests.
- **Manual lab:** a machine per GPU vendor (NVIDIA, AMD, Apple, ARM board) for the paths CI
  cannot cover.

## 10. Roadmap and acceptance criteria

| Milestone | Content | Verified by |
|-----------|---------|-------------|
| **M0** (current prototype) | Repo metadata prototype, grid UI | Existing tests pass |
| **M1 Local core** | Hardware probe, recipe schema + catalog (Ollama, Open WebUI, ComfyUI, llama.cpp), resolver, job engine, install/uninstall, start/stop, logs | On a clean VM per OS: install → start → health check → stop → uninstall leaves no residue; resolver tests green |
| **M2 Update + config** | Version sources, upgrade with rollback, schema-driven config editor, logs UI | Upgrade v1→v2 and roll back; config edit is backed up and restored on failure |
| **M3 LLM assist** | Intent → patch, diff preview, diagnosis of failed jobs | Invalid/out-of-schema patches rejected; applied configs validated and reversible |
| **M4 Remote v1** | Discovery, SSH transport, agent bootstrap, app matrix | Install and start an app on a second LAN machine from the desktop app |
| **M5 Remote v2** | Persistent agent, mTLS pairing, ARM boards, batch operations | Board without SSH enrolled via pairing code; batch upgrade with per-machine results |

M1 is the smallest useful product and is the recommended next step. A concrete first slice:

1. Split `main.rs` into `lib.rs` + commands; add `probe` command → verify: fixture-based tests
   plus the UI renders the detected profile.
2. Define `Recipe` types + schema + loader → verify: catalog linter passes on the bundled set.
3. Implement resolver with 4 recipes → verify: fixture table tests, including rejected methods.
4. Implement job engine + `run`/`download`/`extract` steps → verify: cancel and failure tests.
5. Install Ollama end to end → verify: clean-VM checklist above.
6. Add service layer for the chosen recipes → verify: service survives app exit and restart.

## 11. Risks

- **Recipe maintenance** is the long-term cost; mitigate with a schema/linter, CI smoke tests,
  and a small curated catalog rather than broad coverage.
- **GPU/Python/CUDA matrix** is the hardest install surface; prefer containers where GPU
  passthrough works, and pin torch indexes explicitly.
- **Platform differences** in services and elevation; the fallback `managed-process` keeps
  behavior consistent but is less durable.
- **Remote execution is inherently dangerous**; signed recipes, pinned keys, explicit consent,
  and audit logs are mandatory, not optional.
- **Network restrictions** make mirror selection a first-class feature, not an afterthought.
- **LLM non-determinism**; schema constraints and diff review bound the blast radius.

## 12. Open questions

1. Which platform is the primary target for M1 — Windows, macOS, or Linux? This decides the
   first service backend and elevation path.
2. Is SSH-only remote management acceptable for the first release, or must ARM boards without
   SSH be supported immediately?
3. Which 3–5 apps seed the catalog? (Assumed: Ollama, Open WebUI, ComfyUI, llama.cpp.)
4. Should model downloads/storage be part of v1 configuration, or deferred?
5. Is the recipe catalog maintained in this repository or a separate signed repository?
6. LLM default: local Ollama, bring-your-own cloud key, or both?
7. Distribution model (bundle our own installers) vs. delegating to upstream installers
   affects mirrors, checksums, and license obligations.