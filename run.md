The project currently has two debugging workflows: **frontend-only debugging** (usable right now) and **the full Tauri desktop window** (requires installing the Rust environment first).

## 1. Frontend debugging (available now)

The dev server is already running: [http://localhost:1420](http://localhost:1420) (process PID 757775). Open it directly in a browser; changes to code under [src/ui/src/](src/ui/src/) are hot-reloaded automatically via HMR.

To start it manually:

```bash
cd /home/lizh/workspace/startAI/src/ui
pnpm install        # first time, or when dependencies change
pnpm dev            # http://localhost:1420
```

Other commands:

```bash
pnpm build          # tsc type check + production build
pnpm lint           # oxlint
```

Debugging tools: browser DevTools (F12); the React DevTools extension to inspect components; in the Console, use `document.querySelector('.app-grid')` to check the column count and sizes. To stop the background server, use `pkill -f "vite/bin/vite.js"`.

## 2. Full Tauri desktop debugging (requires setting up the environment first)

Current check results: `cargo`/`rustc` and `webkit2gtk-4.1`, `gtk+-3.0` are all not installed, so the desktop window cannot run for now.

1) Install Rust:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

2) Install the Linux build dependencies (Debian/Ubuntu):

```bash
sudo apt update
sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

3) Run it (from the repository root):

```bash
cd /home/lizh/workspace/startAI
pnpm dlx @tauri-apps/cli@^2 dev
```

Based on `beforeDevCommand` in [tauri.conf.json](tauri.conf.json), the CLI automatically starts Vite in `src/ui` and opens the desktop window. The first build downloads and compiles a large number of crates, which takes several minutes.

If you don't want to use the Tauri CLI, you can also run the two parts separately:

```bash
# Terminal 1
cd src/ui && pnpm dev
# Terminal 2 (repository root)
cargo run
```

Debugging Rust: `RUST_LOG=debug cargo run`; inside the Tauri window, right-click → Inspect (DevTools are enabled by default in debug builds), where you can see the frontend console and network requests.

## 3. Release build

```bash
cd src/ui && pnpm build
cd ../..
cargo build --release --features custom-protocol
```

Note: `cargo run` does not start Vite automatically, so `pnpm dev` must already be running; if the window is blank, first confirm that [http://localhost:1420](http://localhost:1420) is reachable.

If you want, I can also install Rust and the system dependencies now and then actually run `tauri dev` once to verify the desktop window.
