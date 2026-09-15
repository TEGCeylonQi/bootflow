//! 用户设置。目前只有一项：**「每次开机自记账」要不要开**。
//!
//! ## 为什么这一项必须是开关，而且默认关
//!
//! 自记账要在 `HKCU\...\Run` 或任务计划程序里**常驻一个自启条目**——
//! 那是往用户的系统里放东西。即使它完全无害（不弹窗、不联网、只写一个本地
//! JSON），"我装了个工具，它悄悄给我加了开机自启"这件事本身就是越界：
//! 用户没同意，也没地方关掉它。
//!
//! 所以这里取 **opt-in**：
//!
//! * 默认 `false`——不注册任何自启条目，一个都不加。
//! * 用户显式打开后才注册；关掉时同时清掉**两个**入口（任务 + Run 键），
//!   不留残骸。旧版本曾无条件注册过，`reconcile()` 负责把那些也清掉。
//! * 关掉之后，其余功能**不受影响**：单项的"开机后第几秒出现"来自进程采样，
//!   不需要自启条目、不需要任何权限。自记账只负责"总开机耗时"这一条曲线。
//!
//! ## 文件怎么会读不出来
//!
//! 读失败一律**退回默认值（关闭）**，绝不退回"开启"——
//! 一个损坏的配置文件不该被解释成用户同意过。

use crate::error::{AppError, Result};

/// 设置文件的 schema 版本。字段语义变化时递增，便于将来迁移。
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub schema_version: u32,
    /// 「每次开机自记账」是否开启。**默认关闭**，理由见模块头。
    pub boot_recording: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            boot_recording: false,
        }
    }
}

impl Settings {
    fn sanitized(mut self) -> Self {
        // 来自未来版本的文件（版本号更大）不猜字段含义，直接退回默认。
        // 宁可让用户重新勾一次，也不要按错误的理解去改他的系统。
        if self.schema_version > SETTINGS_SCHEMA_VERSION {
            return Self::default();
        }
        self.schema_version = SETTINGS_SCHEMA_VERSION;
        self
    }
}

fn settings_path() -> Option<std::path::PathBuf> {
    crate::paths::app_data_file(SETTINGS_FILE)
}

/// 读设置。文件不存在或损坏时返回**默认值（自记账关闭）**。
///
/// 不返回 `Result`：调用方（启动流程）在每个分支上都只需要一个可用的设置，
/// 没有"读不出来所以不能继续"的情况。
pub fn load() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Settings::default();
    };
    serde_json::from_str::<Settings>(&text)
        .map(Settings::sanitized)
        .unwrap_or_default()
}

/// 写设置。先建目录。
pub fn save(s: &Settings) -> Result<()> {
    let path = settings_path()
        .ok_or_else(|| AppError::Other("找不到 LOCALAPPDATA，无法保存设置".to_string()))?;
    crate::paths::ensure_app_data_dir()
        .map_err(|e| AppError::Other(format!("无法创建数据目录：{e}")))?;
    let text = serde_json::to_string_pretty(s)
        .map_err(|e| AppError::Other(format!("设置序列化失败：{e}")))?;
    std::fs::write(&path, text).map_err(|e| AppError::Other(format!("设置写入失败：{e}")))
}

/// 把**系统里的自启状态**对齐到设置值。
///
/// 关掉时必须主动去清，不能只是"以后不再注册"——旧版本无条件注册过，
/// 用户从旧版升级上来时那条自启还留在系统里，光改设置是清不掉的。
/// 这一步幂等：本来就没有条目时只是两次只读查询。
pub fn reconcile(s: &Settings) {
    if s.boot_recording {
        return;
    }
    if let Err(e) = crate::diag::boot_marker::remove_autostart() {
        log::warn!("清理自记账自启条目失败：{e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 改进程级 `LOCALAPPDATA` 的用例必须串行（全局环境变量）。
    fn with_isolated_dir<T>(name: &str, f: impl FnOnce() -> T) -> T {
        let _guard = crate::testenv::lock();
        let dir = std::env::temp_dir().join(format!("bootflow-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let prev = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &dir);
        let out = f();
        match prev {
            Some(v) => std::env::set_var("LOCALAPPDATA", v),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    #[test]
    fn default_is_off() {
        // 「不能强行要求」的代码化：默认值必须是"不注册"。
        assert!(!Settings::default().boot_recording);
    }

    #[test]
    fn missing_file_yields_off() {
        with_isolated_dir("missing", || {
            assert_eq!(load(), Settings::default());
            assert!(!load().boot_recording);
        });
    }

    #[test]
    fn corrupt_file_yields_off_not_on() {
        // 损坏的配置绝不能被解释成"用户同意过"。
        with_isolated_dir("corrupt", || {
            crate::paths::ensure_app_data_dir().unwrap();
            std::fs::write(
                crate::paths::app_data_file(SETTINGS_FILE).unwrap(),
                "{ 这不是 JSON",
            )
            .unwrap();
            assert!(!load().boot_recording);
        });
    }

    #[test]
    fn round_trips() {
        with_isolated_dir("roundtrip", || {
            let on = Settings {
                schema_version: SETTINGS_SCHEMA_VERSION,
                boot_recording: true,
            };
            save(&on).unwrap();
            assert_eq!(load(), on);
        });
    }

    #[test]
    fn future_schema_falls_back_to_default() {
        with_isolated_dir("future", || {
            crate::paths::ensure_app_data_dir().unwrap();
            std::fs::write(
                crate::paths::app_data_file(SETTINGS_FILE).unwrap(),
                r#"{"schemaVersion": 999, "bootRecording": true}"#,
            )
            .unwrap();
            assert_eq!(load(), Settings::default(), "来自未来的文件不猜语义");
        });
    }

    #[test]
    fn missing_field_uses_off() {
        // 早期只写 schemaVersion 的文件不该被当成打开。
        with_isolated_dir("partial", || {
            crate::paths::ensure_app_data_dir().unwrap();
            std::fs::write(
                crate::paths::app_data_file(SETTINGS_FILE).unwrap(),
                r#"{"schemaVersion": 1}"#,
            )
            .unwrap();
            assert!(!load().boot_recording);
        });
    }
}
