# 安装档位（T0–T4）与平台支持判定

本文汇总 EasyAI 导入 GitHub 仓库时「如何判断一个仓库属于哪一档安装方式」以及「如何判断它支持
哪些操作系统」。平台维度只考虑 **Linux / Windows / macOS**，与 `probe::platform_id_for`
接受的值一致。

## 1. 档位与 `MethodKind` 的映射

档位定义在 `src/resolve.rs`：

```rust
pub const TIERS: [&str; 5] = ["T0", "T1", "T2", "T3", "T4"];

pub fn tier_for(method: &Method) -> &'static str {
    match method.kind {
        MethodKind::Portable => "T0",
        MethodKind::Release | MethodKind::Container => "T1",
        MethodKind::PackageManager => match manager {
            "brew" | "winget" => "T2",
            _ => "T3",
        },
        MethodKind::Language => "T2",
        MethodKind::Script | MethodKind::Universal => "T3",
        MethodKind::Source | MethodKind::Vcs => "T4",
    }
}
```

| Tier | 语义 | `MethodKind` |
|---|---|---|
| T0 | 官方桌面安装包 / 便携包 | `portable` |
| T1 | 官方 release 二进制 / 官方容器镜像 | `release`、`container` |
| T2 | 官方包管理器 / 语言包管理器 | `package-manager`（brew/winget）、`language` |
| T3 | 安装脚本 / 社区源 / 发行版原生包 | `script`、`universal`、`package-manager`（apt/dnf/pacman…） |
| T4 | 源码构建 / 直接 Git 安装 | `source`、`vcs` |

无法判定任何可用安装方式的仓库不加档位，导入为 `draft: true`，界面按「无安装方式」呈现。

## 2. 各档识别信号

### T0 官方桌面安装包 / 便携包

| 证据 | 具体 |
|---|---|
| release 资产 | `*.dmg`、`*.pkg`（macOS）；`*-Setup.exe`、`*.msi`、`*.msix`（Windows）；`*.AppImage`、`*.deb`、`*.rpm`（Linux）；`*portable*.zip` |
| 声明式清单 | `tauri.conf.json` 的 `bundle.targets`；`electron-builder.*` 的 `win/mac/linux.target`；`forge.config.js` 的 `makers` |
| 归档内容 | `Contents/MacOS/<exe>`（macOS）、`.exe` + 依赖 DLL（Windows）、`usr/bin/<exe>` + `.desktop`（Linux） |

### T1 官方 release 二进制 / 官方容器镜像

- 资产名含平台词：`linux`、`gnu`、`musl`；`windows`、`win32`、`win64`、`mingw`、`msvc`；
  `macos`、`mac`、`darwin`、`osx`、`apple`；
- 资产名或体积显示镜像包：`docker`、`image`、`oci`，或含层的大 `.tar`；
- release 资产为空时：`compose.yaml` / `docker-compose.yml` 中存在
  `services.*.image:`；
- README 中出现 `docker pull <registry>/<image>:<tag>`。

### T2 官方包管理器 / 语言包管理器

| 生态 | 证据 |
|---|---|
| Homebrew | `Formula/*.rb`、`Casks/*.rb`、README 的 `brew install` |
| winget | `manifests/*/`（YAML）、`winget install` |
| Scoop | `bucket/*.json`、`scoop install` |
| pip / uv | `pyproject.toml` 的 `[project.scripts]`、`pip install <pkg>` |
| npm | `package.json` 的 `bin`、`npm i -g` |
| cargo | `cargo install <crate>`、`[package] name` |
| go | `go install <module>@latest` |

### T3 安装脚本 / 社区源 / 发行版原生包

- 仓库根有 `install.sh`、`install.ps1`、`scripts/install.sh`；
- README 出现 `curl … | sh`、`iwr … | iex`；
- 第三方 tap、AUR `PKGBUILD`、`*.spec`、`snapcraft.yaml`、Flatpak manifest；
- `package-manager` 的 `manager` 为 `apt`/`dnf`/`pacman`/`zypper`/`snap`/`flatpak`/`choco`/`scoop`。

### T4 源码构建 / 直接 Git 安装

- 只有语言项目结构、无任何二进制或包发布：`Cargo.toml` + `[[bin]]`、`CMakeLists.txt` +
  `add_executable`、`package.json` + `build`、`pyproject.toml` + `[build-system]`；
- `source` / `vcs` 方法的 `source.git` 与 `build.commands`。

## 3. 平台判定规则

判定分两类信号：

1. **格式优先**：安装包格式本身已确定操作系统，不依赖文件名中的平台词。
2. **关键词兜底**：仅在格式无法确定平台时才读取名称中的 token。

### 3.1 由扩展名直接确定平台

| 扩展名 | 平台 | 示例 |
|---|---|---|
| `.deb` `.rpm` `.AppImage` `.snap` `.flatpak` `.rpm` | linux | `Foo_1.2_amd64.deb` |
| `.exe` `.msi` `.msix` | windows | `Foo-Setup.exe` |
| `.dmg` `.pkg` `.app` | macos | `Foo-1.2.dmg` |

### 3.2 名称 token

| 平台 | token |
|---|---|
| linux | `linux`、`gnu`、`musl`、`appimage`、`deb`、`rpm`、`flatpak`、`snap` |
| windows | `windows`、`win32`、`win64`、`win`、`mingw`、`msvc`、`exe`、`msi` |
| macos | `macos`、`mac`、`darwin`、`osx`、`apple`、`dmg`、`pkg`、`universal`（仅当无其他平台词） |

架构 token：

| token | 架构 |
|---|---|
| `universal`、`universal2` | 双架构：`x86_64` + `aarch64` |
| `aarch64`、`arm64`、`arm64e` | `aarch64` |
| `x86_64`、`amd64`、`x64` | `x86_64` |
| `i386`、`i686`、`x86` | `x86_64`（兼容） |
| `armv7`、`armhf`、`386` | 不在目标平台内，跳过 |

**关键规则：平台明确而架构缺失时不丢弃**，按 `x86_64` 兜底。

### 3.3 各档平台矩阵

| Tier | linux | windows | macos | 架构来源 |
|---|---|---|---|---|
| T0 | `.deb`/`.rpm`/`.AppImage`/`usr/bin` | `.exe`/`.msi`/`.msix` | `.dmg`/`.pkg`/`Contents/MacOS` | 资产名 + 归档内容 |
| T1 二进制 | `linux`/`gnu`/`musl` | `windows`/`win32`/`win64`/`mingw`/`msvc` | `macos`/`darwin`/`osx` | 资产名，缺省 `x86_64` |
| T1 容器 | 支持 | 支持 | 支持 | 不查 manifest，声明全架构 |
| T2 brew | — | — | 支持 | 无架构区分 |
| T2 winget/scoop | — | 支持 | — | 无架构区分 |
| T2 pip/npm/cargo/go | 支持 | 支持 | 支持 | 无架构区分 |
| T3 install.sh | 支持（脚本含 `Darwin` 时加 macos） | — | 视内容 | 无 |
| T3 install.ps1 | — | 支持 | — | 无 |
| T3 apt/dnf/pacman | 支持 | — | — | 通常多架构 |
| T3 choco/scoop | — | 支持 | — | 无 |
| T4 source | 支持 | 支持 | 支持 | 全架构，标记 advanced |
| 无安装方式（draft） | 不可装 | 不可装 | 不可装 | — |

## 4. 当前实现与缺口

`src/catalog.rs` 中与平台判定相关的两个函数：

- `archive_rank(name)` 只识别 `.tar.gz`、`.tar.zst`、`.zip`、`.7z`、`.tar`；
- `platforms_for_asset(name)` 要求名称同时含平台词与架构词，否则返回空。

因此 `.dmg`、`.exe`、`.deb`、`.AppImage` 等原生安装包既不会被识别为可安装资产，也无法
贡献平台信息；仅发布原生包的 macOS / Windows 应用会被判成「不支持本机」。

导入流程的现状：

| 场景 | 结果 |
|---|---|
| 有归档且有本机平台 | 正常生成 `release-<platform>` 方法 |
| 有归档但无本机平台 | 生成其他平台的方法，`supported = false` |
| 无归档 | `draft = true`，`supported = true`（不准确） |

## 5. 导入时的平台可用性提示

导入预览（`RecipePreview`）新增机器与可用性字段：

| 字段 | 含义 |
|---|---|
| `platform` | 当前机器平台 id，如 `linux-x86_64` |
| `supported` | 是否存在支持本机的可用安装方式 |
| `detectedPlatforms` | 从 release 资产推断出的全部平台 |
| `unusable` | `true` 时不可导入，例如仓库不是可运行软件 |
| `hint` | 面向用户的一句话说明，说明原因与建议 |

判定顺序：

1. 若 recipe 声明了方法，且存在 `when` 匹配本机平台的方法 → 可用；
2. 若声明了方法但不匹配本机 → 不可用，提示「该仓库只为 X 提供安装包」；
3. 若没有任何方法（draft）且 release 资产推断不出平台 → 不可用，提示
   「未找到可执行安装产物，可能不是可运行软件」；
4. 若 draft 但推断出其他平台 → 不可用，并列出该仓库支持的平台。

仅在 `unusable = false` 时允许点击「Add to catalog」，避免把插件的资源包或纯库作为应用加入
目录。
