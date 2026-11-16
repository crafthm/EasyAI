# 预安装环境检测方案

Status: proposal for review（待讨论，未实现）
Related documents: [tiers-and-platform-detection.md](tiers-and-platform-detection.md), [install-methods.md](install-methods.md)

本文提出一套**通用**的「安装前环境检测」机制：在真正执行安装之前，判定目标机器是否满足该应用
的全部前置条件，并给出可执行的补救方式。

---

## 1. 现状与问题

当前唯一的「环境检查」在 `src/resolve.rs::hard_filter`，它只回答「机器是不是这个平台」，
且只看**存在性**，不看**版本**：

```rust
if !method.when.matches_runtimes(&profile.runtimes) {
    return Err(format!(
        "Requires runtime {}, available: {}.",
        method.when.runtime.join(", "),
        sorted(&profile.runtimes)
    ));
}
```

`HardwareProfile.runtimes` 的类型是 `HashSet<String>`（`src/probe.rs:16`），只保存名字。
由此产生四个结构性缺陷：

| 缺陷 | 表现 |
| --- | --- |
| 只判存在 | `node` 在 PATH 里就通过，不管它是 22.4 还是 24.11 |
| 无版本概念 | 无法表达「需要 Node >= 24.11.0」 |
| 无补救措施 | 缺失时只返回 `Err`，用户不知道下一步做什么 |
| 无分级 | `Err(String)` 是扁平字符串，UI 无法区分阻塞/建议/提示 |

实际后果有两种，都很差：

- **假通过**：版本不足却放行，`npm install` 中途报 `EBADENGINE`，或装上了但启动即崩。
- **假拒绝**：本可通过另一条方法装上的应用，被直接判死。

### 触发案例

`github.com/paperclipai/paperclip` 的 `cli/package.json` 声明 `engines.node: ">=24.11.0"`，
分发型为 npm 包 `paperclipai`（T2）。导入后会生成 npm 方法，但现有检查完全看不见
`engines` 字段，因此这条需求**从未被验证过**。

---

## 2. 设计目标

1. **通用**：不为 Node 写特例。任何工具链、库、服务、权限、磁盘、端口都用同一套机制表达。
2. **渐进**：recipe 不做任何声明也能工作，退化为与现状一致；声明则覆盖推断。
3. **可执行**：每条不满足都附带至少一个补救方式，并标注其风险等级。
4. **不破坏**：`hard_filter` 现有的 `Err(String)` 语义保留，新能力旁挂，不改既有调用方行为。

---

## 3. 核心抽象：把「环境条件」变成数据

关键动作是把需求从「散落在 match 分支里的 if」提升为**可枚举的声明式数据**。

### 3.1 需求与状态

```rust
pub enum Status {
    Satisfied,
    /// 可满足但当前未满足，附带补救方式。
    Missing(Vec<Remedy>),
    /// 无法判定（探测失败、平台不支持该探测手段）。
    Unknown(String),
}

pub enum Requirement {
    Toolchain { name: String, range: VersionRange, scope: Scope },
    Package   { manager: String, package: String, range: VersionRange },
    Disk      { min_mb: u64 },
    Memory    { min_mb: u64 },
    Gpu       { any: Vec<String> },
    Port      { port: u16, protocol: PortProtocol },
    PathExists { path: PathBuf, scope: Scope },
    Env       { name: String, matches: Option<String> },
    Command   { program: String, args: Vec<String>, expect_exit: i32 },
}
```

`Unknown` 是刻意设计的状态，不是 `Missing` 的变体。**探测失败不等于不满足**：`--version`
超时、输出格式无法解析时，应给出「无法确认，可能可以安装」，而不是拦下用户。这一点对
通用性至关重要，否则一个奇怪的工具输出就会让本可安装的应用被拒。

### 3.2 作用域

```rust
pub enum Scope { User, System, Any }
```

用于区分「用户级可自装」与「系统级需提权」。例如 `node` 位于
`~/.nvm/versions/node/v24.11.0/bin/node` 属 `User`，位于 `/usr/bin/node` 属 `System`。
它直接决定第 7 节中「能否自动解决」的判定。

### 3.3 补救方式

```rust
pub struct Remedy {
    pub kind: RemedyKind,
    pub label: String,
    /// 需要用户明确同意才执行（会改动机器全局状态）。
    pub needs_consent: bool,
    /// 可直接展示给用户复制的命令。
    pub command: Option<Vec<String>>,
}

pub enum RemedyKind {
    /// 只给出官方地址，本程序不动手。
    Manual,
    /// 本程序代为安装。
    AutoInstall,
    /// 切换到 recipe 中另一条方法。
    SwitchMethod { method_id: String },
    /// 调整偏好后重试（如允许提权）。
    ChangePreference,
}
```

---

## 4. 探测：从「存在性」升级为「带版本的工具清单」

### 4.1 数据结构

```rust
pub struct ToolInfo {
    pub name: String,
    pub path: PathBuf,
    pub version: Option<Version>,
    pub scope: Scope,
}
```

`HardwareProfile` 的 `HashSet<String>` 改为 `HashMap<String, ToolInfo>`。这个改动会波及
`matches_runtimes` / `matches_managers` / `matches_toolchains`，但语义不变，只需把
「是否包含 key」改为「key 是否存在」。

### 4.2 版本获取

不为每个工具写解析代码，统一约定：

```rust
fn read_version(program: &Path, args: &[&str]) -> Option<Version>
```

只尝试 `--version` 与 `-v` 两种调用，取输出首行、正则提取第一个版本号。覆盖绝大多数工具：

| 工具 | 调用 | 备注 |
| --- | --- | --- |
| `node` | `node -v` | 输出 `v24.11.0`，需剥离 `v` |
| `python3` | `python3 --version` | 输出到 stdout |
| `go` | `go version` | 形如 `go version go1.22.1 linux/amd64` |
| `cargo` | `cargo --version` | 形如 `cargo 1.76.0 (…)` |
| `java` | `java -version` | **输出到 stderr**，需特判 |

无法解析时返回 `None`，对应 `Status::Unknown`。

### 4.3 版本范围

复用已有实现，不新增 `semver` crate：`src/update.rs::compare_versions` 已正确处理
不等长版本（`1.2` 等于 `1.2.0`）与预发布（`1.0.0-beta` 早于 `1.0.0`）。新增的只有范围解析：

```rust
impl VersionRange {
    /// 支持 `>=24.11.0`、`^20.0.0`、`~1.2`、`20.x`、`*`、`>=18 <25`。
    pub fn parse(text: &str) -> Option<Self>;
    pub fn matches(&self, version: &Version) -> bool;
}
```

解析失败必须落为 `Unknown`，绝不能退化成「不满足」——否则 `engines` 里任何不常见写法
都会误伤。

---

## 5. 从方法推导需求

这是「通用」的核心。**不要求 recipe 声明任何东西**，按 `MethodKind` 推导出合理需求；
若 recipe 声明了，则以声明覆盖推导结果。

| `MethodKind` | 自动推导的需求 |
| --- | --- |
| `Release` / `Portable` | 磁盘空间、归档工具（tar/zip/7z 按扩展名） |
| `Container` | `docker` 或 `podman` 任一（含最低版本）、`docker info` 可连通、磁盘 |
| `PackageManager` | 对应 manager 存在；若 `elevation`，检查 `sudo -n` 可用 |
| `Language` | 对应 runtime（`python`/`node`/`cargo`/`go`）、manager、`engines` 范围 |
| `Script` | `sh`、`curl`、磁盘 |
| `Source` / `Vcs` | `git`、构建工具链、磁盘（含构建产物） |
| `Universal` | `flatpak` / `snap` / `nix` 命中其一 |

映射按 `MethodKind` 分派，各方法内部再读自己的 spec 分支。新增一种方法只需加一个分支，
不必改探测层。

**与 `MethodWhen` 的关系**：`MethodWhen` 保持现状（硬过滤），新增可选的
`requires: Vec<RequirementDecl>`。声明存在时覆盖推导，而不是叠加，避免同一条件
被报告两次。旧 recipe 无此字段时行为完全不变。

---

## 6. 在流程中的位置

检测放在**三处**，语义各不相同。

### 6.1 预览期（`preview_recipe`）

只做静态推断，**不触发任何命令**，用于在导入对话框里提示「需要 Node >= 24，本机 22.4」。
不真实探测，保持预览的秒级响应，也不产生副作用。

### 6.2 计划期（`plan_install`）—— 主要位置

真实探测，产出报告。`plan_for` 已持有 `db_path` / `app_data` / `install_root`，
且 `plan_install` 与 `install_app` 都经过它，天然保证「用户看到的计划」与「即将执行的计划」
是同一份。

```rust
pub struct RequirementReport {
    /// 人类可读，如 "Node >= 24.11.0"。
    pub requirement: String,
    /// "satisfied" | "missing" | "unknown"
    pub status: String,
    /// 实测详情，如 "current 22.4.0"。
    pub detail: String,
    pub remedies: Vec<Remedy>,
    /// true 时阻断安装。
    pub blocking: bool,
}
```

`InstallPlan` 新增两个字段：`requirements: Vec<RequirementReport>` 与 `blocked: bool`。
`blocked == true` 时 `install_app` 直接返回错误并附带补救清单，不进入 `run_job`。

### 6.3 安装期（`lifecycle::install`）—— 二次确认

执行前**重跑一次**探测。计划与执行之间存在时间窗，用户可能刚用 `nvm` 切了 Node 版本，
或停掉了 Docker。这一步耗时几十毫秒，属纵深防御，不是重复劳动。

---

## 7. 自动解决的边界

这是最需要克制的部分。原则：**能提示就提示，能本地安装才安装，绝不擅自改动全局环境。**

| 情形 | 处理 |
| --- | --- |
| 缺 `docker` | 仅提示 + 官方链接（`Manual`），不代装 |
| 缺 `node` 或版本过低 | 优先建议 recipe 的其它方法（`SwitchMethod`）；无替代则给出升级命令 |
| 缺 `git` 且方法为 `Source` | 若该 recipe 有 `Release` 方法，直接建议切换 |
| 磁盘不足 | 提示，允许用户先卸载旧应用 |
| 端口被占 | 报告占用者，**不杀进程** |
| 需提权但偏好关闭 | `ChangePreference`，引导打开开关 |
| 系统级工具缺失且配方自带安装脚本 | 标 `AutoInstall` + `needs_consent: true`，执行前弹确认 |

**明确不做**：

1. 不自动 `sudo` 安装系统包；
2. 不自动下载并覆盖用户已有的 Node / Python 等；
3. 不自动 `kill` 占用端口的进程。

这三件事属于「安装器越权」。一旦判断失误，用户很难自行恢复。真正安全的自动化只有一种：
**切换到 recipe 已经声明过的其它方法**——那是作者提供的分发型，不是本程序的猜测。

---

## 8. 落地顺序

四步，每步可独立验证。

1. **探测层**（`probe.rs`）：`HashSet<String>` → `HashMap<String, ToolInfo>`，新增
   `read_version`。约 150 行。现有 `matches_*` 改法简单，语义不变。
2. **需求层**（新增 `requirements.rs`）：`Requirement`、`VersionRange`、第 5 节映射表、
   `evaluate(profile, method) -> Vec<RequirementReport>`。纯函数，最易测。
3. **接入**（`resolve.rs`、`commands.rs`）：`InstallPlan` 加字段，`plan_for` 填充；
   `install_app` 检查 `blocked`；`lifecycle::install` 开头重探测。
4. **前端**（`PlanDialog` / `ImportDialog`）：分三类展示，每条 `missing` 下方给出补救按钮。

注意 `src/tools.rs` 中 `plan_install` 的 MCP 声明需同步新增字段，否则 MCP 侧拿不到报告。

---

## 9. 待讨论的问题

以下三点会显著影响实现，需要先定下来。

### 9.1 `blocked` 的判定标准

建议：**仅当 `Missing` 且没有任何 `Remedy` 时**才阻断。像 Node 版本不足这类「有补救方案」
的情况不阻断，而是警告并让用户确认。

理由：如果只要有一条 `Missing` 就阻断，会把「切换方法即可安装」的应用也堵死——这正是
现状的毛病之一。

### 9.2 是否引入 `semver` crate

倾向**不引入**。`compare_versions` 已能处理预发布与不等长版本，范围解析约 100 行即可覆盖
常见写法。取舍：

- 不引入：只支持 `>=`、`>`、`<=`、`<`、`^`、`~`、`x`、`*` 与空格连接的 AND，不支持
  `||`、`!=` 与严格的预发布比较细则。
- 引入：完整语义，但多一个依赖，且需把 `compare_versions` 的调用点也统一过去。

### 9.3 预览期是否做真实探测

默认**不做**，以保持预览秒级响应。若希望在导入时就回答「这台机器能不能跑」，则需在
`preview_recipe` 里也调用探测，代价是几十毫秒延迟，且预览会开始依赖本机状态
（同一 recipe 在不同机器上预览结果会不同）。需要权衡。

### 9.4 补充：与 T0–T4 的关系

本方案不改变档位判定，只在此之前增加一道「能否安装」的闸门。它影响的是同一 `MethodKind`
下的可行性，而不是档位归属。例如 npm 方法仍属 T2，但若 Node 版本不足且无替代方法，
则 T2 这条路不可走，需回退到 T3/T4。

---

## 10. 附：典型需求示例（npm · T2）

以 `paperclipai` 为例，生成的方法与环境需求大致如下：

```json
{
  "id": "npm",
  "kind": "language",
  "language": {
    "ecosystem": "npm",
    "package": "paperclipai",
    "manager": "npm",
    "executable": "{install_dir}/node_modules/.bin/paperclipai"
  }
}
```

推导出的需求：

| 需求 | 来源 | 判定 |
| --- | --- | --- |
| `node >= 24.11.0` | `engines.node` | 版本比较 |
| `npm`（或 `pnpm`）在 PATH | 方法推导 | 存在性 |
| 磁盘 ≥ 估算值 | `size` 或默认 | 数值比较 |

安装后版本需从 registry 或 `node_modules/<pkg>/package.json` 读取，而非 manifest 中的版本号
（本例 manifest 为 `0.3.1`，registry 实际已到 `2026.1106.0`，两套体系）。
