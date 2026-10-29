# Git Repository Installation Methods

Research for EasyAI's install engine, with a recommendation that replaces the assumption that
“install from source” is the default for a Git repository.

## 1. Question

For a project whose upstream lives in a Git repository, what are all the realistic ways a user or
a tool can install it, and which ones should EasyAI support, in what order?

The important conclusion up front: **“the project is on GitHub” does not imply “build from
source”.** A repository is a source of metadata; the actual distribution channel can be a release
archive, an installer, a package registry, a container image, a portable bundle, a package
manager, or source code — often several at once. EasyAI should model a ranked set of methods per
application, not a single `install` block.

## 2. Taxonomy

| # | Channel | Typical artifacts | Privilege | Isolation | Update detection | EasyAI priority |
|---|---|---|---|---|---|---|
| 1 | Prebuilt release artifacts | `.tar.gz`, `.zip`, `.7z`, `.tar.zst`, `.dmg`, `.msi`, `.exe`, `.deb`, `.rpm`, checksums | User or system | Directory-level | Release tag | **P0** |
| 2 | Official install script | `install.sh`, `install.ps1`, `curl \| sh` | Often root | Varies / poor | Post-install version probe | **P1** |
| 3 | OS package manager | apt, dnf, pacman, zypper, apk; official repos, PPA, Copr, AUR | Root | Package-managed | Distro metadata | P1 |
| 4 | User package manager | Homebrew formula/cask, winget, scoop, Chocolatey, MacPorts | User | Package-managed | Manager metadata | **P0** |
| 5 | Language registry | PyPI, npm, crates.io, Go modules, RubyGems, NuGet, Composer, conda-forge | User | venv / prefix / global | Registry API | **P0** (pip/uv) |
| 6 | VCS install | `pip install git+…`, `uv tool install git+…`, `cargo install --git`, `go install module@version`, `npm install github:…` | User | Tool-managed | Pinned ref / registry | P2 (advanced) |
| 7 | Container / OCI | Docker Hub, GHCR, Quay images; Compose; Helm/Kustomize | User or root | Strong | Image tag/digest | **P0** |
| 8 | Desktop / portable bundle | `.dmg`, `.msi`, `.exe`, Windows portable `.7z` with embedded Python, macOS `.app` | User | Directory-level | Embedded version / release | **P0** |
| 9 | Universal Linux formats | AppImage, Flatpak, Snap | User | Strong (Flatpak/Snap) | Format metadata | P1 |
| 10 | Nix | Flake, `nix profile install github:…`, NixOS module | User | Strong | Flake lock | P2 |
| 11 | Source build | `git clone` + CMake/Make/Meson/Cargo/Go/npm/Python build | User or root | Depends on prefix | Git tag / commit | P1 (fallback) |
| 12 | CI artifacts | GitHub Actions artifacts | User | Temporary | Artifact ID | P3 (not a channel) |
| 13 | Plugins/extensions | ComfyUI custom nodes, Open WebUI functions/tools | User | App-level | Git ref | Adjacent feature |
| 14 | Hosted/cloud/devcontainer | Codespaces, devcontainers, hosted APIs | N/A | N/A | N/A | Out of scope |

### 2.1 Prebuilt release artifacts

The upstream publishes platform-specific binaries or packages on the Git host's release page
(GitHub Releases, GitLab Releases) or a vendor CDN. Assets often encode OS, architecture, GPU
variant, and version in the filename. This is usually the best default: no build toolchain, fast,
deterministic, and easy to uninstall by removing a version directory.

- **Detect:** `GET /repos/{owner}/{repo}/releases/latest`; inspect asset names and extensions;
  `sha256sum.txt`, `.sig`, `.sbom` assets; release notes.
- **Version:** release tag; verify with the binary's `--version`.
- **Uninstall:** remove the version directory; keep declared data paths.
- **Caveats:** Asset naming varies; GPU variants (CUDA/ROCm/Metal) must be selected; some releases
  ship archives for all platforms in one file; checksums may be missing.

### 2.2 Official install scripts

Many upstreams publish `install.sh` or `install.ps1` that detect the platform and install a system
service. It is convenient but opaque: it often requires root, writes outside the user's home,
creates services, and has no reliable uninstall.

- **Detect:** `install.sh`, `install.ps1`, README quick-start.
- **Version:** run the installed command after the script; the script usually installs latest.
- **Uninstall:** often none; needs a bill of materials or a user-space rewrite.
- **Fit:** acceptable only with a preview of the exact URL and optional checksum; prefer a
  user-space release binary when one exists.

### 2.3 OS package managers

Distribution packages (`apt`, `dnf`, `pacman`, `zypper`, `apk`) and third-party repositories
(PPA, Copr, AUR) provide native update and uninstall, but usually require root, lag upstream
releases, and may be built without the GPU features the user expects.

- **Detect:** Repology, distro package APIs, `debian/`, `rpm/`, `PKGBUILD`, PPA/Copr metadata.
- **Version:** package manager query; compare with upstream release.
- **Uninstall:** package manager.
- **Fit:** good for Linux servers and system dependencies; avoid overriding a distribution package
  with a user-space build of the same app, and vice versa.

### 2.4 User package managers

Homebrew (formula for CLI, cask for GUI apps), winget, scoop, Chocolatey, and MacPorts install
without a system-wide package database. They are the natural default on macOS and Windows when the
upstream maintains a manifest.

- **Detect:** Homebrew API/formula, winget-pkgs manifests, scoop buckets, Chocolatey packages,
  MacPorts ports.
- **Version:** manager metadata; native `outdated`/`upgrade`.
- **Uninstall:** manager.
- **Caveats:** community taps/buckets are supply-chain risk; GUI casks may not expose a service
  command; some formulas build from source while casks ship binaries.

### 2.5 Language registries

For Python, Node, Rust, Go, Ruby, .NET, and PHP projects, the repository is often a source for a
package published to a registry. Installing the published artifact is usually better than
building: wheels and prebuilt binaries avoid a compiler and are reproducible.

- **Detect:** `pyproject.toml`/`setup.py` + PyPI; `package.json` + npm; `Cargo.toml` + crates.io;
  `go.mod`; `Gemfile`; `*.csproj`; `composer.json`; conda-forge recipe.
- **Version:** registry API; local package metadata.
- **Uninstall:** package manager / remove the isolated environment.
- **GPU caveat:** Python AI packages often need a specific index (for example PyTorch CUDA or ROCm
  wheels). The resolver must choose the index from the GPU probe, not just “latest”.
- **Fit:** `pip`/`uv` into an isolated environment, `pipx`/`uv tool` for CLIs, `npm`/`pnpm` for
  Node CLIs, `cargo`/`go` for compiled CLIs. Avoid polluting the system Python.

### 2.6 VCS installs (direct from the repository)

Package managers can install directly from a Git ref. This is still a source build, but the tool
handles fetching, building, and recording the package.

- `pip install "pkg @ git+https://github.com/owner/repo.git@v1.2.3"`
- `uv tool install git+https://github.com/owner/repo@v1.2.3`
- `cargo install --git https://github.com/owner/repo --tag v1.2.3 --locked`
- `go install example.com/module@v1.2.3` (module path, not a bare Git URL)
- `npm install github:owner/repo#semver:^1.2.3`

- **Why it matters:** it covers projects that publish no release artifacts and no registry package.
- **Rules:** always pin a tag or commit; never install from a moving branch; treat it as an
  advanced method with a visible warning about build time and toolchain requirements.

### 2.7 Containers and OCI images

Official images are common for server-side AI software (Open WebUI, vLLM, LocalAI, Ollama,
llama.cpp). They give strong isolation, reproducible dependencies, and trivial uninstall (remove
container, image, volume). GPU access requires the NVIDIA container toolkit or ROCm setup, and
desktop GUI apps are awkward.

- **Detect:** `Dockerfile`, `compose.yaml`, `.github/workflows` publishing to GHCR/Docker Hub,
  README pull commands; tags such as `:cuda`, `:rocm`, `:cpu`, `:ollama`.
- **Version:** image tag or digest; prefer pinning a digest.
- **Uninstall:** stop/remove container, image, and declared volumes; keep model/data volumes when
  requested.
- **Fit:** high priority when Docker/Podman exists and the app is a service; not a substitute for
  desktop apps.

### 2.8 Desktop and portable bundles

Some projects ship a desktop app or a portable bundle with an embedded runtime. For AI software
this is often the most reliable Windows path because the PyTorch/CUDA/Python matrix is pre-solved.

Examples: ComfyUI Desktop, ComfyUI Windows Portable (NVIDIA/AMD/Intel variants), one-click
packages for text-generation-webui, Ollama's macOS/Windows apps.

- **Detect:** release assets named `*portable*`, `*windows*`, `*.dmg`, `*.msi`, `*.exe`; a separate
  desktop repository; embedded `python_embeded`/`venv` directories.
- **Version:** bundle manifest or embedded binary `--version`.
- **Uninstall:** remove the bundle directory; keep the models/data directory.
- **Fit:** high priority on Windows; large downloads; variant selection by GPU is mandatory.

### 2.9 Universal Linux formats

- **AppImage:** a single executable, no installation, no sandbox; good for desktop tools.
- **Flatpak:** sandboxed, runtime-based, portal-mediated file access; can complicate access to
  model directories and GPUs.
- **Snap:** sandboxed, automatic updates; confinement can block model/data paths.
- **Nix/Guix:** reproducible, per-user profiles, flakes can install directly from GitHub; steep
  learning curve.

These are valid methods but should not outrank release binaries or containers for server
workloads.

### 2.10 Source build

Clone a tag, install build dependencies, compile, and place the result in a prefix. This is the
most expensive method: toolchain versions, CUDA/ROCm/Metal/Vulkan variants, long build times, and
a large set of build artifacts to clean up.

- **Detect:** `CMakeLists.txt`, `Makefile`, `meson.build`, `build.rs`, build docs, `.github/workflows`.
- **Version:** Git tag/commit; verify the built binary.
- **Uninstall:** remove the prefix/version directory; build caches may remain elsewhere.
- **Fit:** a fallback, not the default. It is justified when:
  - no release, container, package, or registry artifact exists;
  - the user needs a non-default backend (for example a specific CUDA/ROCm/Vulkan build);
  - the upstream explicitly documents source as the primary install path (ComfyUI, A1111).

### 2.11 CI artifacts and developer environments

GitHub Actions artifacts are not a distribution channel: they expire, require authentication, and
are not versioned for users. Devcontainers/Codespaces are hosted development environments, not
local installations. Both are out of scope for EasyAI.

### 2.12 Plugins and extensions

ComfyUI custom nodes, Open WebUI functions/tools, and similar extensions are installed *into* an
application. They should be a separate lifecycle feature: a managed directory, Git ref pinning,
dependency installation, and compatibility checks — not an application install method.

## 3. Discovery signals from a repository

| Signal | Likely method |
|---|---|
| Release assets with OS/arch names and checksums | Release binary / package |
| `.deb`, `.rpm`, `.pkg`, `.msi`, `.dmg`, `.AppImage` assets | OS package / desktop / universal |
| `install.sh`, `install.ps1` | Install script |
| `Dockerfile`, `compose.yaml`, GHCR/Docker Hub workflow | Container |
| `pyproject.toml` + PyPI package | pip / uv / pipx |
| `package.json` with `bin` | npm / pnpm |
| `Cargo.toml` with `[[bin]]` | cargo / cargo-binstall |
| `go.mod` | `go install` |
| `Gemfile`, `*.gemspec` | gem |
| `composer.json` | composer |
| `*.csproj`, NuGet package | dotnet tool / NuGet |
| `flake.nix` | Nix |
| `snap/snapcraft.yaml` | Snap |
| Flatpak manifest | Flatpak |
| `debian/`, `rpm/`, `PKGBUILD`, PPA/Copr | Distro package |
| `Makefile`, `CMakeLists.txt`, `meson.build` | Source build |
| `*portable*`, `*windows*` release assets | Portable bundle |
| Homebrew formula/cask, winget manifest, scoop bucket | User package manager |

Additional metadata sources: GitHub Releases API, GitHub Packages, PyPI JSON, npm registry,
crates.io API, proxy.golang.org, RubyGems, NuGet, conda-forge, Repology, Homebrew API, winget-pkgs,
AUR RPC, Flathub, Snapcraft, Docker Hub/GHCR/Quay tag APIs.

The README's “Install” section and the official website download page are useful **hints**, but
should not be treated as authoritative on their own. A later LLM-assisted step may summarize them;
the deterministic signals decide.

## 4. Scoring model

Hard filters (a method is rejected if any fails):

1. OS and architecture.
2. GPU vendor and runtime (CUDA version, ROCm, Metal, Vulkan, CPU fallback).
3. Required runtime/toolchain (Docker, Podman, Python, Node, compiler).
4. Elevation policy (EasyAI defaults to no root).
5. Disk and network budget.
6. User preference (isolated vs system, GUI vs CLI).

Weighted score:

| Factor | Why |
|---|---|
| Upstream-supported | Official methods are less likely to break |
| Isolation / clean uninstall | Avoid contaminating the machine |
| Version determinism | Tags/digests/wheels beat moving branches |
| Update path | Native manager > re-download > rebuild |
| Download/build cost | Source builds are slow and fragile |
| Service manageability | Can EasyAI start/stop/health-check it? |
| Provenance/security | Official + checksum/signature > community script |

Default preference tiers (context still matters):

```
T0  Official desktop/portable/installer with lifecycle support
T1  Official release binary or official container image
T2  Official package manager / language wheel / brew / winget
T3  Install script / community package / distro package
T4  Source build / VCS install (advanced, explicit opt-in)
```

Examples where the tier changes:

- **ComfyUI on Windows:** portable/desktop is T0; container and source are lower.
- **ComfyUI on Linux:** container or uv/venv may beat the manual source clone, but custom nodes and
  models make a writable app directory attractive.
- **Ollama:** release binary or official installer is T0; source is not a supported path for most
  users.
- **llama.cpp:** brew/winget/prebuilt binaries beat source because the backend matrix (CUDA, ROCm,
  Metal, Vulkan, SYCL, CANN) is hard to reproduce.
- **vLLM:** a matching wheel is usually better than source; containers are best for servers.
- **A1111 / some ComfyUI nodes:** source or portable is the only realistic path.

## 5. AI application matrix (examples)

| Application | Upstream methods observed | Recommended order for EasyAI | Source build role |
|---|---|---|---|
| Ollama | install.sh/ps1, DMG/EXE, manual tar.zst, Docker, Homebrew, winget | release binary/installer → container → brew/winget | Not recommended |
| Open WebUI | Docker/GHCR (`:main`, `:cuda`, `:ollama`), pip, uv/uvx, Kubernetes, source | container → pip/uv venv → source | Development only |
| ComfyUI | Desktop app, Windows portable (NVIDIA/AMD/Intel), comfy-cli, manual git+venv, community Docker | portable/desktop (Win) → container → uv/venv source | Primary for Linux/macOS; pin tag |
| llama.cpp | install script, brew/nix/winget, Docker, prebuilt binaries, source | prebuilt/brew/winget → container → source | Only for custom backends |
| vLLM | pip wheels (CUDA/ROCm), Docker, source | matching wheel → container → source | Unreleased features only |
| A1111 | git clone + venv, community portable, Docker | portable/uv venv → container → source | Primary |
| LocalAI | Docker, binary release, source | container → release → source | Fallback |
| text-generation-webui | one-click installer, portable, Docker, source | portable/container → source | Fallback |

The table is illustrative, not a catalog. The point is that “source” is one option among many and
is frequently not the best default.

## 6. Recipe schema changes

Replace the single `install` block with an ordered/scored `methods` list. A method is only offered
if its `when` constraints match the probe.

```yaml
id: comfyui
name: ComfyUI
methods:
  - id: windows-portable-nvidia
    kind: portable-bundle
    when: { os: windows, gpu: nvidia, ram_mb: 8192, disk_mb: 20480 }
    source:
      type: github-release-asset
      repo: comfyanonymous/ComfyUI
      asset: ComfyUI_windows_portable_nvidia.7z
    version: { type: github-release, repo: comfyanonymous/ComfyUI }
    service:
      kind: managed-process
      command: ["{app_dir}/python_embeded/python.exe", "-s", "ComfyUI/main.py", "--port", "8188"]
      health: { type: http, url: "http://127.0.0.1:8188/system_stats" }
    uninstall: { remove_paths: ["{app_dir}"], keep_paths: ["{app_dir}/models"] }
    prefer: 90

  - id: linux-uv-venv
    kind: python-venv
    when: { os: linux, python: ">=3.10", gpu: [cuda, rocm, cpu] }
    steps:
      - { kind: git_clone, url: "https://github.com/comfyanonymous/ComfyUI.git", ref: "v0.3.40", dest: "{app_dir}" }
      - { kind: uv_venv, python: "3.11" }
      - { kind: pip_install, packages: [torch, torchvision, torchaudio], index: "{torch_index}" }
      - { kind: pip_install, requirements: "requirements.txt" }
    prefer: 70

  - id: container
    kind: container
    when: { runtime: [docker, podman], gpu: [cuda, rocm, cpu] }
    image: "ghcr.io/example/comfyui:{version}"
    volumes: ["{data_dir}/models:/app/models"]
    ports: ["8188:8188"]
    prefer: 60

  - id: source-build
    kind: source-build
    when: { toolchain: [git, python], advanced: true }
    steps: [...]
    prefer: 20
    advanced: true
```

Common fields per method: `id`, `kind`, `when`, `requires`, `prefer`, `version`, `service`,
`health`, `uninstall`, `keep_paths`, `size`, `checksums`, `provenance`, `elevation`, `advanced`.

Method-specific descriptors:

- `release`: host/repo, asset map, archive format, checksum asset.
- `package-manager`: manager, package/tap/bucket, version pin.
- `language`: ecosystem, package, version, index/extra-index, environment strategy.
- `container`: image, tag/digest, runtime, volumes, ports, env, GPU flag, restart policy.
- `script`: URL, checksum, elevation, post-install probe.
- `portable`: archive, embedded runtime, launcher.
- `vcs`: URL, ref (tag/commit), build tool.
- `source`: repo, ref, build system, build deps, build commands, install prefix.
- `nix`/`appimage`/`flatpak`/`snap`: format-specific references.

The resolver returns an `InstallPlan` with the chosen method, interpolated steps, rationale, and
rejected alternatives. The UI should show the method and why it was chosen.

## 7. Lifecycle implications

- **Detect existing installs across methods** before installing: PATH binaries, package manager
  queries, container names/images, version directories. Never create a second unmanaged copy.
- **Record the method** with the installed app; upgrades must stay within the method unless the
  user explicitly switches.
- **Method switching** requires uninstall + data migration + reinstall; keep data paths declared.
- **Uninstall**:
  - package manager: native uninstall;
  - container: stop/remove container, image, declared volumes;
  - release/portable/source: remove the version/prefix directory;
  - script: use a bill of materials, or prefer rewriting the script as a user-space install.
- **Upgrade**:
  - release/portable: download new version, swap directory, keep previous for rollback;
  - package manager: native upgrade;
  - container: pull new tag/digest, recreate;
  - pip/uv: recreate the environment from the new version;
  - source: fetch tag, rebuild, keep previous build;
  - VCS: pin a new ref, rebuild.
- **Bill of materials** is mandatory for methods without a native uninstaller: record every file,
  service, environment change, and registry entry created.

## 8. Security and trust

- Install scripts and source builds execute arbitrary code. Show the URL, checksum, commands, and
  provenance before running.
- Prefer release checksums/signatures; pin container digests; pin Git tags/commits; never install
  from a moving branch by default.
- Distinguish official vs community package managers/taps/buckets/repos; default to official.
- Container images: verify registry and signature where available; mount only needed data.
- Source builds: build scripts are code; a pinned tag is reproducible but not necessarily safe.
- Record the chosen method's provenance in the app record and expose it in the detail panel.

## 9. Recommendation

1. **Model methods, not a method.** A recipe has multiple ranked install methods; source build is
   T4/fallback, not the default.
2. **Implement in this order:**
   - already done: release binary;
   - next: portable bundle, container, brew/winget, pip/uv;
   - then: install script, system package manager, AppImage/Flatpak/Snap;
   - then: source build and VCS installs behind an “advanced” flag.
3. **Resolver rules:** hard-filter by platform/GPU/runtime/elevation; score by upstream support,
   isolation, update path, cost, and service manageability; show rejected methods and reasons.
4. **Never infer “source is best” from “it is a Git repo”.** Use deterministic signals from
   releases, manifests, workflows, and registries; use README/LLM only as hints.
5. **For apps with only source**, make the prerequisites, build time, disk usage, and uninstall
   behavior explicit before starting; keep the previous build for rollback.
6. **For AI software specifically**, the highest-value additions are containers and portable
   bundles because they solve the Python/PyTorch/CUDA matrix that makes source installs painful.

## 10. Sources

- Ollama README (install script, installers, Docker, Homebrew):
  <https://github.com/ollama/ollama>
- ComfyUI README (desktop, Windows portable, comfy-cli, manual install):
  <https://github.com/comfyanonymous/ComfyUI>
- Open WebUI README (pip, Docker/GHCR, Kubernetes, source):
  <https://github.com/open-webui/open-webui>
- llama.cpp README (install script, brew/nix/winget, Docker, prebuilt, source):
  <https://github.com/ggml-org/llama.cpp>
- uv tools (installing tools from Git and URLs):
  <https://docs.astral.sh/uv/guides/tools/>
- pip VCS support (git+https, refs, tags, commits):
  <https://pip.pypa.io/en/stable/topics/vcs-support/>
- npm install from Git:
  <https://docs.npmjs.com/cli/install>
- Cargo install:
  <https://doc.rust-lang.org/cargo/commands/cargo-install.html>
- Go install:
  <https://pkg.go.dev/cmd/go>
- Homebrew documentation:
  <https://docs.brew.sh/>
- winget documentation:
  <https://learn.microsoft.com/windows/package-manager/winget/>
- Nix profile install:
  <https://nix.dev/manual/nix/latest/command-ref/new-cli/nix3-profile-install>
- GitHub Releases:
  <https://docs.github.com/en/repositories/releasing-projects-on-github>
- GitHub Packages:
  <https://docs.github.com/en/packages>
- Repology (distro package availability):
  <https://repology.org/>
