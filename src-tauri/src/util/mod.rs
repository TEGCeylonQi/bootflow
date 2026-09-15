//! 通用工具层。
//!
//! 这里放的是**被多个扫描器共享**的能力，而不是某一种来源的专属逻辑。
//! 判断标准很简单：如果第二个人要用同一段代码，它就该在这里。
//!
//! | 模块 | 用途 | 谁在用 |
//! |---|---|---|
//! | `com` | COM 初始化 RAII 守卫 | .lnk 解析、计划任务 COM、图标提取 |
//! | `cmdline` | 命令行拆成「路径 + 参数」 | 启动文件夹、Run 键、服务命令行 |
//! | `approved` | 读 `StartupApproved` 缓存键判断启用状态 | 启动文件夹、Run 键 |
//! | `sign` | 版本信息、数字签名、系统组件判定 | 全部扫描器 |
//! | `icon` | 从 exe 提取图标（base64 PNG） | `get_icons` command |
//! | `link` | 解析 `.lnk` 快捷方式（走 `IShellLinkW`） | 启动文件夹、Run 键 |
//! | `http` | 一次性 HTTPS 读取（走系统 WinHTTP） | `check_update` command |
//! | `shell` | 用默认浏览器打开网址（走 `ShellExecuteW`） | `open_release_page` command |
//!
//! ⚠️ 这些模块**不改系统**：不写注册表、不写文件、不动任何配置。
//! 唯一的对外动作是 `http` 与 `shell`——检查更新时向 GitHub 读一次版本信息，
//! 以及（只在用户点击下载时）唤起浏览器。两者都不修改本机状态。

pub mod approved;
pub mod cmdline;
pub mod com;
pub mod http;
pub mod icon;
pub mod link;
pub mod shell;
pub mod sign;
