//! 本程序自己的数据目录。
//!
//! 单独抽一个模块，是因为这些路径以前散在 `boot_marker` / `settings` 里各写一份
//! `join("BootFlow")`。目录名一旦重复出现，改动时就一定会漏掉一处——
//! 而漏掉的那处表现是"设置写进去了、读不出来"，静默且难查。

use std::path::PathBuf;

/// 数据目录名。**不要**在别处再字面量写一次。
pub const APP_DIR: &str = "BootFlow";

/// `%LOCALAPPDATA%\BootFlow`。取不到环境变量时返回 `None`。
pub fn app_data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join(APP_DIR))
}

/// 数据目录下的某个文件。
pub fn app_data_file(name: &str) -> Option<PathBuf> {
    Some(app_data_dir()?.join(name))
}

/// 确保数据目录存在，返回它。
pub fn ensure_app_data_dir() -> std::io::Result<PathBuf> {
    let dir = app_data_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "找不到 LOCALAPPDATA")
    })?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_dir_name_is_a_single_constant() {
        // 防止有人"顺手"在别处又拼一次目录名：这里锁住常量本身，
        // 而调用方全部走本模块，改一处即全局生效。
        assert_eq!(APP_DIR, "BootFlow");
    }
}
