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
//!
//! ⚠️ 这些模块**只读**。整个 v1.0 不出现任何写注册表 / 写文件的代码路径。

pub mod approved;
pub mod cmdline;
pub mod com;
pub mod icon;
pub mod link;
pub mod sign;
