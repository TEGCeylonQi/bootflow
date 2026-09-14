//! 读取 Windows 的「启动项启用/停用」记录。
//!
//! 这是一个**容易被忽略但很关键**的数据源：用户在任务管理器里点过"禁用"，
//! 或者在「设置 → 应用 → 启动」里关掉某个启动项时，Windows **不会删除**
//! 注册表里的 Run 项，而是往 `StartupApproved` 里写一条缓存记录。
//!
//! 这带来两个后果：
//! 1. 只读 Run 键会**把已停用的项当成正常启动项**——用户看到 16 项，
//!    其中一半其实早就关了。这会直接误导"我的开机负担有多重"的判断。
//! 2. 这个机制本身就是 v1.5 实现「禁用」功能的正确做法：
//!    **写缓存键，不删实体**，用户随时能在任务管理器里改回来。
//!
//! ⚠️ 本模块**只读**。

use crate::model::SourceKind;

/// `StartupApproved` 的注册表前缀。
const BASE: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

/// 某个来源对应到哪个 `StartupApproved` 子键。
///
/// 返回 `(是否机器级, 子键名)`。`None` 表示该来源不在这个机制覆盖范围内
/// （服务与计划任务有各自的启用状态，不在这里）。
fn subkey_for(source: SourceKind) -> Option<(bool, &'static str)> {
    use SourceKind::*;
    match source {
        StartupFolderUser => Some((false, "StartupFolder")),
        StartupFolderMachine => Some((true, "StartupFolder")),
        // RunOnce 与 Run 共用同一个缓存键 —— 系统就是这么实现的
        RunUser | RunOnceUser => Some((false, "Run")),
        RunMachine | RunOnceMachine => Some((true, "Run")),
        // 32 位视图有独立的 Run32 键
        RunMachine32 | RunOnceMachine32 => Some((true, "Run32")),
        Service | ScheduledTask | SystemHook => None,
    }
}

/// 解析 `REG_BINARY` 值。
///
/// 格式是 12 字节：首字节表示状态，其余 11 字节是停用时刻的 FILETIME 片段
/// （Windows 用它来按"多久没用过"排序，我们不需要精确解析）。
///
/// - `0x02` → 启用
/// - `0x03` → 已停用
/// - 其余（含全零）→ 视为启用
///
/// 保守取"启用"是刻意的：把停用的误判为启用，用户最多是白看一眼；
/// 反过来把启用的误判为停用，用户会以为某个必需的程序被关了。
fn parse_approved(data: &[u8]) -> bool {
    // `0x03` 是唯一明确的「已停用」标记；其余取值（空值、全零、未知值）
    // 一律按启用处理——保守方向见上方说明。
    !matches!(data.first(), Some(0x03))
}

/// 查询某个启动项是否处于启用状态。
///
/// **查不到时返回 `true`**：`StartupApproved` 里没有记录，
/// 恰恰说明用户从没动过它，那它就是启用的。
pub fn is_enabled(source: SourceKind, name: &str) -> bool {
    lookup(source, name).unwrap_or(true)
}

/// 返回 `Some(true)` = 启用，`Some(false)` = 已停用，`None` = 无记录/读不到。
pub fn lookup(source: SourceKind, name: &str) -> Option<bool> {
    let (machine, sub) = subkey_for(source)?;

    let root = if machine {
        winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
    } else {
        winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
    };

    let key = root.open_subkey(format!(r"{BASE}\{sub}")).ok()?;

    // 用 `get_raw_value` 而不是 `get_value`：winreg 只为 `String` / `u32` 之类
    // 实现了 `FromRegValue`，`Vec<u8>` 不在其中。而 `StartupApproved` 下的值
    // 是 `REG_BINARY`，只能走原始字节这条路。
    let raw = key.get_raw_value(name).ok()?;

    Some(parse_approved(&raw.bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_0x03_means_disabled() {
        assert!(!parse_approved(&[0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    }

    #[test]
    fn byte_0x02_means_enabled() {
        assert!(parse_approved(&[0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    }

    #[test]
    fn empty_or_unknown_is_treated_as_enabled() {
        // 保守方向：宁可多显示一个，也不要把启用中的项说成已停用
        assert!(parse_approved(&[]));
        assert!(parse_approved(&[0x00; 12]));
        assert!(parse_approved(&[0xFF; 12]));
    }

    #[test]
    fn services_and_tasks_have_no_approved_record() {
        assert!(subkey_for(SourceKind::Service).is_none());
        assert!(subkey_for(SourceKind::ScheduledTask).is_none());
        assert_eq!(subkey_for(SourceKind::RunMachine32), Some((true, "Run32")));
    }
}
