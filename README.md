# BootFlow

**Windows 开机自启的统一清单与可逆开关。** 把散落在启动文件夹、注册表 Run / RunOnce、计划任务、服务和系统注入里的自启动项收拢到一处，看清是谁拖慢了开机，并把不需要的关掉——预演在前、快照在后、随时可回滚。

原生桌面程序（Rust + 系统 WebView2 渲染界面，单个 exe）。**不请求提权、不弹 UAC。**

**v0.2.1** · Windows 11 已实测 / Windows 10 1809+ 支持

## 为什么需要它

Windows 的自启动机制散落在至少八个位置：注册表 Run / RunOnce 的三个视图、两个启动文件夹、计划任务、服务、系统注入（AppInit_DLLs / IFEO）。任务管理器只呈现其中一部分；Autoruns 这类工具能看全却只诊断不改，且以注册表路径组织界面——用户看不出「这些项分别意味着什么、哪些能关」。

BootFlow 把它们归一成**一份按用途分类的清单**（应用 / 后台服务 / 计划任务 / 系统注入 / 系统组件），配风险分级、处置建议，以及可逆的执行。

## 能力

| 能力 | 说明 |
| --- | --- |
| 全来源扫描 | 启动文件夹（用户 / 全局）、Run · RunOnce（HKCU / HKLM / WOW6432Node）、登录与开机触发的计划任务、开机服务（含延迟启动）、系统注入 |
| 有效性检测 | 指向已卸载程序的残留、重复入口、当前不可达的网络位置 |
| 风险分级与建议 | 禁改区 / 高危 / 注意 / 安全，每项附判定依据与一句话处置理由 |
| 开机开销 | 各阶段实测耗时（系统事件日志）、每项在开机后第几秒出现（内核记录）、每项资源占用（WDI，与任务管理器「启动影响」同源同阈值） |
| 可逆写入 | 变更篮 → 应用前预演（改动前/后、后果、风险）→ 自动快照 → 一键回滚；快照可导出为**不依赖本程序**的 `.reg` / `.ps1` |
| 报告导出 | JSON / CSV / Markdown |

## 三条刻意的克制

- **权限**——manifest 是 `asInvoker`，以启动它的用户令牌运行，永不提权。代价是「分段耗时」与「启动影响」两个通道读不到（ACL 只给管理员）：界面给出原因和「以管理员身份重开」入口，不用空数据充数。核心功能不依赖提权。
- **精度**——「什么时候出现」「占了多少资源」「花了多久」是三个不同的量，来自三条独立通路，界面分开陈述，**从不合成一个数字**。CPU 时间跨核累加，因此从不被当作耗时展示。
- **可逆性**——任何写操作前自动快照；服务启动类型只允许「自动 ↔ 手动」，不提供「禁用」（单向门）；禁改区（服务栈 / Winlogon / explorer / 安全中心 / 驱动）不提供写入口。

## 安装

从 [Releases](https://github.com/TEGCeylonQi/bootflow/releases) 下载：

- `BootFlow_x.x.x_x64-setup.exe` — 用户级安装，不需要管理员
- `bootflow-portable-x64.zip` — 解压即用，不写注册表

需要 [WebView2 运行时](https://developer.microsoft.com/microsoft-edge/webview2/)；Windows 11 与较新的 Windows 10 已内置。

## 构建与验证

前置：Rust stable、Node.js 20+、Visual Studio Build Tools（含「使用 C++ 的桌面开发」）。

```bash
npm install
npm run tauri dev      # 开发模式
npm run tauri build    # 产出安装包与免安装 exe
npm run verify         # 与 CI 相同的全部闸门
```

发布包必须走 `npm run tauri build`：裸 `cargo build --release` 只编 Rust 后端，前端资源不会内嵌进二进制，运行时界面打不开。

- **前端** React 19 · TypeScript · Vite · Tailwind · ECharts
- **后端** Rust · `windows-rs`，直调 COM 与 Win32（不走 `schtasks` / `sc` 命令行）
- **关键模块** `scanners/`（五类来源读取）· `diag/`（开机日志、耗时归因、诊断）· `pipeline.rs`（身份归一 → 有效性 → 去重 → 耗时 → 风险 → 建议）· `writers/`（可逆写入与回滚）

## 已知限制

- Windows 11 已实测；Windows 10 1809+ 支持但未实测；Server / ARM64 未验证
- 未覆盖：浏览器辅助对象（BHO）、UWP StartupTask、Winlogon 各键、驱动类自启
- **写能力尚未在真机上走完一次完整的「写入 → 回滚」闭环**（写入模块的测试是纯逻辑，不触真实系统）。界面与命令已接通，首次使用建议先在虚拟机里试
- 系统只在**完整引导且开机偏慢**时才写性能记录：没有分段耗时数据是常态，不等于开机不花时间；开了快速启动的机器尤其如此

## 许可

[MIT](LICENSE)
