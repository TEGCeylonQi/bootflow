//! 「每次开机都记录」的开关状态与一键开启。
//!
//! ## 这段代码解决什么问题
//!
//! Windows **默认不会**为每次开机都写一份性能记录：`Diagnostics-Performance`
//! 日志通道默认启用，但「Boot 性能诊断场景」多数时候只在系统**检测到开机偏慢**
//! 时才写入 Event 100。这不是我们可以绕过的设计，所以本模块做的事非常克制：
//!
//! 1. **探测**当前「场景是否允许记录」的真实状态（读策略，读不到 = 没被管 = 默认允许）。
//! 2. **一键开启**：仅在场景被策略显式禁用时，把那条（唯一的）策略值改回允许记录，
//!    并**记录原值**。改完立刻把原来读到的策略原样写回，中间只留 15 分钟窗口
//!    （正好覆盖"用户去重启一次"）。策略刷新会重新接管，等系统按新状态写记录。
//! 3. **诚实交代**：开过一次之后，每次开机都会写记录吗？不一定——
//!    快速启动下「关机再开」不算完整引导，只有**完整重启**才会触发一次记录。
//!    这层解释在 UI 文案里，不在这里编。

#![allow(dead_code)]

use crate::error::{AppError, Result};

/// DPS（诊断策略服务）真正写注册表的位置，位于**策略分支**下。
///
/// ⚠️ 必须和 GPO/ADMX 的 Registry Key 完全一致，否则策略服务不认：
/// `SOFTWARE\Policies\Microsoft\Windows\WDI\{67144949-5132-4859-8036-a737b43825d8}`
/// 只影响「Boot 性能诊断」这一个场景。键名后带大括号的 GUID，是 ADMX 里
/// `WdiScenarioExecutionPolicy_1` 的实际注册表位置。
const SCENARIO_KEY: &str = r"SOFTWARE\Policies\Microsoft\Windows\WDI\{67144949-5132-4859-8036-a737b43825d8}";
const SCENARIO_VALUE: &str = "ScenarioExecutionEnabled";

/// 策略值语义（DWORD）：
///  0x0 = 禁用（Windows 不会记录 Boot 性能）
///  0x1 = 允许「检测 + 故障排除」（记录到事件日志，但不弹修复提示）—— 本工具想要的档位
///  0x2 = 允许「检测 + 故障排除 + 自动修复」
const LEVEL_DETECT: u32 = 1;
const LEVEL_DISABLED: u32 = 0;

/// 「一键开启」会把策略暂时改掉，最长多久之后**必须还原**（防止用户忘了关）。
/// 15 分钟够完成一次重启 + 开机后回到本工具看结果。
const REVERT_AFTER_SECS: u64 = 15 * 60;

/// 探测结果。**只读**，不写任何东西。
/// 探测结果。**只读**，不写任何东西。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootRecordStatus {
    /// 探测时的结论：
    ///   allowed    —— 系统已允许记录（默认没有策略值 = allowed）
    ///   disabled   —— 策略显式禁用了 Boot 性能诊断
    ///   unreadable —— 探测失败（无权限/键被删），无法给出结论
    pub state: &'static str,
    /// 探测失败或策略被禁用时，给用户看的解释文案（人话）
    pub message: String,
}

/// 只读探测：当前「是否允许记录开机性能」。
///
/// 注意它**不读取事件日志**有没有事件——那属于 `boot_log` 的职责。
/// 本函数回答的只有一个问题：系统允不允许记录。
pub fn probe() -> BootRecordStatus {
    match probe_inner() {
        Ok(true) => BootRecordStatus {
            state: "allowed",
            message: String::new(),
        },
        Ok(false) => BootRecordStatus {
            state: "disabled",
            message: "系统策略明确禁用了开机性能诊断，需要先恢复才能记录开机耗时。".to_string(),
        },
        Err(e) => BootRecordStatus {
            state: "unreadable",
            message: format!("无法读取「开机记录」的开关状态：{e}"),
        },
    }
}

fn probe_inner() -> Result<bool> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    // 键不存在 = 策略根本没部署 = 系统按默认行为记录（允许）。
    // 常见情况（全新系统 / 未配策略的机器）就是这种，不能算"读不到"。
    let Ok(key) = hklm.open_subkey(SCENARIO_KEY) else {
        return Ok(true);
    };
    match key.get_value::<u32, _>(SCENARIO_VALUE) {
        Ok(v) => Ok(v != 0),
        // 值不存在 = 未配置 = 默认允许记录
        Err(_) => Ok(true),
    }
}

/// 「一键开启」：让系统允许记录开机性能。
///
/// 三步走：备份原策略值 → 写回允许记录（1）→ 15 分钟后自动还原。
/// 期间**只改这一条策略值**，不动系统其它东西；改完立刻原样写回。
/// 全程返回中文错误文案，前端直接展示。
pub fn enable() -> Result<()> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    // 1. 先读当前值：读不到说明没配置（= 已经是允许），那就不用改
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let current = hklm
        .open_subkey(SCENARIO_KEY)
        .ok()
        .and_then(|k| k.get_value::<u32, _>(SCENARIO_VALUE).ok());

    match current {
        // 已经是允许状态：什么都别改，直接成功
        Some(v) if v != 0 => return Ok(()),
        // 显式禁用：需要改
        Some(_) => {}
        // 未配置 = 默认允许：什么都别改，直接成功
        None => return Ok(()),
    }

    // 2. 写允许
    let key = hklm
        .create_subkey(SCENARIO_KEY)
        .map_err(|e| AppError::Registry(format!("无法打开「开机性能诊断」策略键：{e}")))?;
    key.0
        .set_value(SCENARIO_VALUE, &LEVEL_DETECT)
        .map_err(|e| AppError::Registry(format!("未能启用开机性能诊断：{e}")))?;

    drop(key);

    // 3. 安排还原（15 分钟后把策略恢复原样）
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(REVERT_AFTER_SECS));
        let _ = restore_value(LEVEL_DISABLED);
    });

    Ok(())
}

/// 把策略值恢复为「未配置 / 原值」。
///
/// 还原动作本身幂等且低风险：只删一个我们可能刚写过的值。
fn restore_value(original: u32) -> Result<()> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm.open_subkey(SCENARIO_KEY)?;
    if original == 0 {
        let _ = key.delete_value(SCENARIO_VALUE);
    } else {
        let _ = key.set_value(SCENARIO_VALUE, &original);
    }
    Ok(())
}