//! 统一错误类型。
//!
//! 设计要点：Tauri command 返回的 `Err` 会被序列化后交给前端，
//! 因此 `AppError` 必须实现 `Serialize`。这里序列化成"一句人话"，
//! 因为前端 `invoke` 的 catch 分支拿到的就是它。
//!
//! 另一个要点：**单项失败不整体失败**。扫描过程中某个 HKLM 子键 ACL 受限，
//! 不应该让整次扫描挂掉——那种情况进 `ScanResult.errors`（String 列表），
//! 只有"整件事都做不了"才返回 `Err`。

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("访问注册表失败：{0}")]
    Registry(String),

    #[error("调用 Windows 接口失败：{0}")]
    Win32(String),

    #[error("COM 组件调用失败：{0}")]
    Com(String),

    #[error("读取文件失败：{0}")]
    Io(#[from] std::io::Error),

    #[error("数据格式错误：{0}")]
    Serde(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    // 注意：这里必须写全 `std::result::Result`。
    // 本模块下方定义了单参数的 `Result<T>` 别名，会遮蔽标准库的双参数 `Result`，
    // 直接用 `Result<S::Ok, S::Error>` 会被解析成给别名塞两个参数而报错。
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

/// 便捷宏：把 `windows` crate 的 `Result<()>` 转成 `AppError::Win32`，
/// 并附上调用点上下文。`windows` crate 的 HRESULT 消息对用户毫无意义，
/// 所以上下文必须由我们补。
#[macro_export]
macro_rules! win_ctx {
    ($expr:expr, $ctx:literal) => {
        $expr.map_err(|e| $crate::error::AppError::Win32(format!("{}：{}", $ctx, e)))
    };
}
