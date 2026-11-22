# Import Warning Actions

Status: proposal (design only, no code changes yet)

Scope: turning the warnings shown when importing a GitHub repository into
"problem + executable action" instead of read-only text.

## 1. Goal

Keep every warning, but attach structured actions to it. The user should not
have to search GitHub, edit recipe files, or guess what to do next.

## 2. Data model

Replace `warnings: Vec<String>` in `RecipePreview` with structured warnings.
Each warning carries a stable code (for icons/colour in the UI), the existing
human-readable message, and zero or more actions:

```rust
struct PreviewWarning {
    code: WarningCode,          // stable enum used by the UI
    message: String,            // current user-facing text
    actions: Vec<WarningAction>,
}

enum WarningAction {
    RetryPreview,                  // run preview again
    RemoveImported { id: String }, // remove the old imported entry, then retry
    OpenApp { id: String },        // open the existing app's detail view
    OpenUrl { url: String },       // open Releases / npm / Node page in a browser
    EditRecipe { id: String },     // open or edit the recipe
    AddDraftAnyway,                // ignore the warning and import
    RegisterExistingApp,           // register an app installed manually
    DetectStartCommand,            // detect the start command after install
}
```

`RecipePreview` also needs enough detail about a conflict to choose the right
actions: whether the existing entry is built-in or imported, and its source URL.

## 3. Handling per warning

| Warning | Recommended actions | Notes |
|---|---|---|
| **ID conflict** | (1) `RemoveImported` + automatic retry, (2) `OpenApp` | If the existing entry is imported, allow one-click removal with confirmation. If it is built-in, hide removal and only offer "open existing app". |
| **Latest release could not be read** | (1) `RetryPreview`, (2) `OpenUrl` (repository Releases page) | When the cause is rate limiting, show the reset time and keep Retry disabled until it expires. |
| **No installable archive / data files only / not runnable** | (1) `OpenUrl` (Releases), (2) `EditRecipe`, (3) `AddDraftAnyway` | Let the user inspect the release assets, then add an install method manually or keep the draft. |
| **Native installers only (deb/rpm/AppImage/dmg/exe)** | (1) `OpenUrl` (installer download), (2) `RegisterExistingApp`, (3) `EditRecipe` | Do not run native installers automatically (elevation and system package management risk). Provide a "install manually, then register" loop. |
| **npm install inferred** | (1) `OpenUrl` (npm package page or the repository's package.json), (2) `EditRecipe` | Lets the user verify package name, `bin`, and version. |
| **npm version differs from registry** | `OpenUrl` (npm package page) only | Informational; no strong action needed. |
| **Start command not inferred** | (1) `EditRecipe` (set the start command), (2) `DetectStartCommand` after install | Best UX: after installation, scan `{install_dir}` for executables and let the user pick one. |
| **Node version too old** | (1) Detect the local `node -v` and show it, (2) `OpenUrl` (Node download / nvm docs) | Do not upgrade Node automatically. If the repository also has a release archive method, suggest switching to it. |
| **No install method for this platform / only other platforms** | (1) `OpenUrl` (Releases), (2) `EditRecipe` | Make clear this is a platform limitation, not a download failure. |

## 4. Required capabilities

**Already available and reusable:**

- `open_url` — open a page in the external browser.
- `remove_recipe` — delete an imported recipe.
- `get_app_detail` — navigate to an existing app.

**New work:**

1. **Recipe editing entry point** — open `catalog/imported/<id>.json` in an
   external editor or an in-app form; validate on save and recompute the
   checksum. This covers "no install method", "unsupported platform", and
   "missing start command" in one place.
2. **"Register an existing app" flow** — after a manual install, the user picks
   the executable and EasyAI creates an unmanaged entry with a start command.
   This covers native installers.
3. **Post-install start-command detection** — scan the install directory and
   present executable candidates.
4. **Warning action execution API** — front-end buttons call back-end commands
   with confirmation, error reporting, and automatic preview retry.
5. **Rate-limit metadata** — return the reset time from `github.rs` so the UI
   can show "retry in N minutes".

## 5. Rollout

**Phase 1 (low cost, highest value)**

- Structured warnings (code + message + actions).
- Conflict: remove imported copy and retry; open existing app.
- Release fetch failure: retry; open Releases page.
- npm inference: open npm page.
- Keep the current "add anyway" path.

**Phase 2 (removing dead ends)**

- Recipe editing entry point (manual install method / start command).
- Native installers: open download page + "register after manual install".
- Unsupported platform: open Releases page + edit recipe.

**Phase 3 (experience polish)**

- Post-install start-command detection.
- Local Node version detection and upgrade guidance.
- Rate-limit countdown and automatic retry.

## 6. Open questions

1. Should recipe editing use an **external editor** (simpler, lower risk) or an
   **in-app form** (better UX, more work)? External editor is suggested first.
2. Is the **"register a manually installed app"** (unmanaged entry) mode
   acceptable? It is the key to handling deb/dmg releases.
3. Should native installers remain non-automatic? Recommendation: yes, keep
   them manual because of elevation and system package-management risk.