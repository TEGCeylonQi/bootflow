# BootFlow

> 把 Windows 上散落在各处的开机自启机制统一收拢，看清它们，然后编排它们。

**当前状态：v1.0 只读体检版** — 本版本只读，不修改系统的任何配置。

---

## 为什么做这个

Windows 的启动项散落在至少八个地方，而且这些位置**在任何一台 Windows 上都是同一套结构**：
两个启动文件夹（当前用户的 `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup`
和所有用户的 `%ProgramData%\Microsoft\Windows\Start Menu\Programs\Startup`）、
三处注册表 Run 键（`HKCU` / `HKLM` / `HKLM\WOW6432Node`，分别对应用户级、全局、32 位程序）、
计划任务、服务、`Winlogon`、`AppInit_DLLs`、浏览器辅助对象、UWP 启动任务。

现有工具各管一段：

| 工具 | 强项 | 缺什么 |
| --- | --- | --- |
| Autoruns | 发现能力顶级 | **不做时序编排** |
| Process Lasso | 进程调优很强 | **不管启动时序** |
| 任务管理器 | 界面友好 | 只有启用/禁用 |

**没有人把「发现 → 诊断 → 按时序编排 → 可回滚」串起来。** BootFlow 想做这条链路。

## v1.0 能做什么

- **全来源扫描**：启动文件夹（用户级 + 全局）、Run / RunOnce（HKCU / HKLM / WOW6432Node）、含 Boot/Logon 触发器的计划任务、开机自启的服务（含延迟启动与触发器服务）、系统注入（`AppInit_DLLs` 与 `Image File Execution Options`）
- **无效启动项检测**：找出指向已卸载程序的残留记录、同一程序的重复入口、指向不可达位置的项
- **处置建议**：对每一项给出「建议清理 / 建议停用 / 建议确认」及一句话人话理由
- **编排草稿**：把想做的改动（停用 / 延迟 / 优先级 / 顺序）先收进底部**变更篮**，
  随时整体放弃或逐条撤销；确认后产出一份可保存的方案文件。**草稿阶段不改动系统**，
  执行能力要等存储层与回滚机制就绪后开放
- **跨来源去重**：同一个程序同时出现在 Run 键和启动文件夹时会被识别出来
- **开机耗时拆解**：读取系统性能日志，把开机时间拆到各阶段，并列出被标记为慢启动的服务
- **三层风险评级**：区分「禁改区 / 高危 / 注意 / 安全」，并给出判定依据
- **报告导出**：JSON / CSV / Markdown。Markdown 版是给人看的——含判定依据、诊断结论与
  每条建议的理由，可以直接发给帮你修电脑的人

### 界面

一级界面只说人话，技术细节折叠进二级。

```
┌────────────────────────────────────────────────────────┐
│ 体检│编排  启动项总数 · 待处理数 · 开机耗时 · OS · 权限   │
├──────────┬─────────────────────────────┬───────────────┤
│ 启动项    │  启动时序 / 耗时分析          │ 这是什么        │
│ 按类型分组 │  按开机阶段分档（可拖拽排序）  │ 需要你关注      │
│ 可勾选批量 │                              │ 编排设置        │
│          │                              │ ▸ 技术详情(折叠) │
├──────────┴─────────────────────────────┴───────────────┤
│ 变更篮：N 项待应用的变更 · 撤销 / 重做 / 放弃 / 保存方案  │
└────────────────────────────────────────────────────────┘
```

键盘可用：`↑↓` 移动、`空格` 勾选、`Ctrl+A` 全选、`/` 搜索、`1/2` 切视图、
`E` 切模式、`Esc` 逐层退出、`Ctrl+Z` 撤销编排。

名称会被统一识别：注册表里写的是 `localsend_app` 这类可执行文件名、或 `AcmeHelper`
这类厂商内部名，界面上显示的是从程序元数据读出来的产品名。**清单里不出现原始文件名、
注册表值名或任务路径**——那些只在「技术详情」里可查。
列表里的类型徽章回答「这是什么」（应用 / 服务 / 任务 / 注入 / 系统），而不是「它注册在哪个注册表键下」。

## 适用范围

下面描述的是**工具的行为**，不是某台机器的状态。凡是没验证过的，这里会直说没验证过。

**系统版本**

| 环境 | 状态 |
| --- | --- |
| Windows 11（21H2 及以上） | 已在实机验证 |
| Windows 10 1809+ | 支持，但**未在实机验证**——计划任务定义与事件日志的 schema 在不同版本间有差异，解析器按可选字段容错，具体结果未经确认 |
| Windows Server / LTSC | 未验证。所依赖的事件日志通道与计划任务机制相同，预期可用 |
| ARM64 | 未验证。Tauri 与 WebView2 均支持该架构，理论上可编译运行 |

**程序装在哪个目录都可以**

扫描读的是注册表值、任务定义与快捷方式里记录的**真实路径**，不对安装位置做任何假设：
`Program Files`、`Program Files (x86)`、用户目录（`%LOCALAPPDATA%` / `%APPDATA%`）、
非系统盘（如 `D:\Apps\`）一律同样处理。

**运行权限**

| 权限 | 能读到 |
| --- | --- |
| 普通用户 | 全部启动项：注册表 / 启动文件夹 / 计划任务 / 服务 / 系统注入 |
| 管理员 | 以上全部 **+** 开机阶段耗时与慢启动记录 |

程序始终以普通权限启动，**不会自动提权**。缺数据时界面会写明原因并给出显式提权入口，
不会静默显示成"没有耗时"。

**多用户**

扫描覆盖当前用户与全局（所有用户）两个范围，每项都标记了作用域，
两者同时存在同一程序时会被识别为跨作用域的重复入口。

**v1.0 尚未覆盖的位置**

明说比含糊好：本版本**不扫描**浏览器辅助对象（BHO）、UWP `StartupTask`、
`Winlogon` 各键、驱动类自启。它们同样属于"启动项"，只是不在这一版的扫描范围内。

## 安装

从 Releases 下载：

- `BootFlow_x.x.x_x64-setup.exe` — NSIS 安装包（用户级安装，不要求管理员权限）

> 免安装的便携版 `.zip` 尚未发布。在那之前可以直接用构建产物里的
> `src-tauri/target/release/bootflow.exe`——它本身就不需要安装。

**运行依赖**：需要 [WebView2 运行时](https://developer.microsoft.com/microsoft-edge/webview2/)
（Windows 11 与较新的 Windows 10 已内置；更早的系统需要单独安装，安装包会自动检测并提示）。

> 本应用以普通权限运行。个别系统位置可能因 ACL 限制读取失败，此时会在扫描结果里单独列出，不影响其余部分。

## 从源码构建

前置：

- [Rust](https://rustup.rs/)（stable）
- [Node.js](https://nodejs.org/) 20+
- Visual Studio Build Tools（含「使用 C++ 的桌面开发」工作负载）

```bash
npm install
npm run tauri dev      # 开发模式
npm run tauri build    # 产出安装包
```

构建产物：

- `src-tauri/target/release/bootflow.exe` — 免安装可执行文件，直接双击运行
- `src-tauri/target/release/bundle/nsis/BootFlow_<版本号>_x64-setup.exe` — NSIS 安装包

首次构建要编译全部依赖，耗时较长。

> **可选：依赖镜像**。如果 `cargo build` 长时间停在「Updating crates.io index」，
> 说明到你所在网络的 crates.io 较慢，配置镜像即可（这一步与 BootFlow 本身无关，
> 只影响依赖下载速度）：
> ```bash
> # Rust 工具链分发地址
> setx RUSTUP_DIST_SERVER "<镜像地址>"
> setx RUSTUP_UPDATE_ROOT "<镜像地址>/rustup"
> ```
> 再在 `~/.cargo/config.toml` 里按
> [Cargo 官方文档的 source replacement](https://doc.rust-lang.org/cargo/reference/source-replacement.html)
> 配置 crates.io 源替换。npm 侧同理，换成任意可用 registry 即可。

## 开发验证（不靠截图）

界面改动曾经靠「构建 → 装包 → 打开 → 截图 → 人眼比对」验证。这条链路很慢，
而且有个致命弱点：它只能判断「看起来有没有异样」。

`text-ink-faint` 那类**样式令牌失效**问题，在截图上就是"感觉有点怪"——
Tailwind 对未定义的令牌是**静默忽略**的，不报错也不警告，
颜色悄悄回退到继承值，本该最淡的说明文字反而变成最亮的。
这种情况靠看图基本发现不了，静态扫描却一抓一个准。

所以验证拆成三层，一层比一层慢，日常只用前两层：

```bash
npm run check      # ① 静态自检：样式令牌是否都有定义 + TS 与 Rust 的数据契约是否一致
npm run dev        # ② 浏览器里直接跑（mock 数据），改哪看哪，热更新
npm run ui:smoke   # ③ 真实浏览器跑关键交互并自动断言（需先装驱动）
```

第 ③ 层只需要一个几 MB 的驱动包，**不用下载浏览器内核**（自动回退到系统 Edge）：

```bash
npm i -D playwright-core
```

它把验证拆成两半：能断言的（元素在不在、点了有没有反应、有没有控制台报错）
一律自动断言；**只有真的失败才留截图**存证。

### 想看"读不到数据"时界面长什么样

开发服务器地址后加 `?timeline=denied`：

```
http://localhost:1420/?timeline=denied
```

会切到「权限不足、没读到开机性能日志」的那份 mock。

这不是臆想的边界情况——**任何一台 Windows 在非管理员权限下运行都是这个结果**
（见下方「已知限制」）。不把它做成随时可复现的状态，很容易写出"读不到就显示一张空图"，
而空图在用户眼里等于"你的开机不花时间"。

> 三层都通过，再去实机上人工跑一遍。不要用截图替代断言，
> 也不要在没跑 `npm run check` 的情况下就开始构建安装包。

## 已知限制

**开机耗时数据通常需要管理员权限才能读取。** 记录开机性能的事件日志通道
（`Microsoft-Windows-Diagnostics-Performance/Operational`）的访问控制列表里
**没有普通用户的条目**，普通权限下 `EvtQuery` 会直接返回「拒绝访问」。

这是 Windows 的默认设计，不是 bug。BootFlow 的处理方式：

- 启动项列表**不受影响**——它们从注册表、启动文件夹、计划任务和服务直接读，不需要这项权限
- 界面会明确写出「未读取」并说明原因，**不会显示成 0 秒或空图**
- 给出「以管理员身份重新打开」的入口，用来补齐这一层数据

其余可能读不到的情形（都是如实报告，不猜测）：

- 计划任务的个别定义文件 ACL 受限，会跳过并在结果里列出跳过了几个
- 目标位于已断开的网络共享或已拔出的可移动介质 —— 标为「位置当前访问不到」，
  插回来就恢复，不会被当成失效项

## 架构

**它是一个原生 Windows 桌面应用，不是网页。**
界面由系统自带的 WebView2 控件渲染（Windows 11 与较新的 Windows 10 已内置），所以你看到的 UI 是 HTML/CSS 画的；但整个程序是一个独立的 `.exe`，双击即运行——不需要打开浏览器，也没有后台服务或本地网页服务器。

```
┌───────────────────────────────────────┐
│  BootFlow.exe（单个原生可执行文件）     │
│ ┌─────────────────┬─────────────────┐ │
│ │ Rust 主体        │ WebView2 渲染层  │ │
│ │ 扫描 / COM /     │ React UI        │ │
│ │ Win32 / 分析     │ 图表 / 交互      │ │
│ └─────────────────┴─────────────────┘ │
│         ↑ Tauri IPC（进程内通信）       │
└───────────────────────────────────────┘
```

- **前端**：React 19 + TypeScript + Vite + Tailwind，图表用 ECharts
- **后端**：Rust + `windows-rs`，直接调用 COM 与 Win32（不用命令行封装 `schtasks` / `sc`——它们有编码与转义问题，也拿不到完整属性）
- **关键模块**：
  - `src-tauri/src/scanners/` — 各来源扫描器，只负责「把原始记录读出来」
    （`startup_folder` / `registry` / `scheduled_task` / `service` / `system_hooks`）
  - `src-tauri/src/pipeline.rs` — 七步后处理，**顺序即依赖**：
    身份归一 → 类型判定 → 有效性 → 去重 → 耗时注入 → 风险评级 → 建议。
    耗时必须在风险前（没有耗时就没有"启动偏慢"这条判据），风险必须在建议前
    （建议的措辞取决于风险等级）
  - `src-tauri/src/valid.rs` — 无效目标检测（文件是否还在、是否可执行、网络位置是否可达）
  - `src-tauri/src/advise.rs` — 处置建议规则引擎
  - `src-tauri/src/dedupe.rs` — 两级身份键（精确身份 / 程序身份）与稳定 ID 派生
  - `src-tauri/src/diag/` — 开机性能日志解析、三层风险判定、诊断码表与白名单
  - `src-tauri/src/util/sign.rs` — 签名验证。先查内嵌签名，失败再查 **catalog 目录签名**
    （系统文件大多用后者，只查内嵌会把 `ctfmon.exe` 这类系统文件全判成未签名）
  - `src-tauri/src/model.rs` — 前后端共用的数据模型（改动必须与 `src/types/model.ts` 同步）

## 关于数据精度（重要）

我们**不伪造精度**：

- 标 **实测** 的耗时来自系统事件日志（`Microsoft-Windows-Diagnostics-Performance`，EventID 103），是硬证据
- 标 **估算** 的为按启动相位推算，界面上会明确标注，且不计入实测统计
- **未知** 表示系统未记录，我们不会编一个数字出来

Windows 本身不为每个自启动项记录独立耗时。任何声称能精确给出「每个启动项耗费 X 秒」的工具，都在编数字。

## Roadmap

| 版本 | 内容 | 原则 |
| --- | --- | --- |
| **v1.0** | 只读体检：全来源发现 + 诊断 + 报告 | 零风险，先建立信任 |
| **v1.5** | 启用/禁用开关、变更前快照、Dry-run 预演、一键回滚 | 任何写操作都可逆 |
| **v2.0** | 可视化编排：泳道拖拽、依赖图、分级延迟 | 从「管理单个项」到「编排整体时序」 |
| **v2.5** | 调度引擎：就绪探针、DAG 执行、编译器 | 让编排真正生效 |
| **v3.0** | IO 感知调度、一键优化建议、场景预设 | 软件替用户做决策 |

### 永久护栏

- 🔴 **禁改区**（服务栈、Winlogon、explorer、安全中心、驱动类）任何版本都不提供写入口
- ⚠️ 进程优先级 `RealTime` **永久禁止设置**（会抢在音频驱动前获取 CPU，导致爆音与系统假死），上限为 `High`
- 📦 打包永久使用 unpackaged，不使用 MSIX（其注册表/文件系统虚拟化会破坏系统工具的行为）

## 参与

Issues 与 PR 都欢迎。提交前请跑一遍与 CI 相同的检查：

```bash
npm run check          # 类型检查 + 样式令牌校验 + 前后端数据契约比对
npm run ui:smoke       # 界面冒烟测试（需先 npm i -D playwright-core）

cd src-tauri
cargo clippy --all-targets -- -D warnings
cargo test
```

CI 在 `.github/workflows/ci.yml`：前端跑在 Linux，Rust 跑在 `windows-latest`
——这些测试要读真实的 Windows，所以不能放在 Linux runner 上。

## 许可

[MIT](LICENSE)

---

## English

BootFlow is a Windows startup-item **audit and orchestration** tool. The current v1.0 is a **read-only health check**: it scans startup folders, `Run`/`RunOnce` keys, scheduled tasks with boot/logon triggers, and auto-start services, then reports boot-phase timings and risk ratings. It never modifies your system.

The long-term goal is visual orchestration — sequence, delay, and dependency ordering for startup items, with every change reversible.

MIT licensed.
