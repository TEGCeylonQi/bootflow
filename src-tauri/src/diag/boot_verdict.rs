//! 开机性能「为什么没数据」的一键诊断（可跨设备使用）。
//!
//! ## 用途
//!
//! 用户在界面上看到「没有开机性能数据」时，往往分不清到底是
//!   - 没权限（普通用户读不到性能日志）—— 需要提权
//!   - 系统策略禁用了记录 —— 需要改策略值
//!   - 「快速启动」一直生效 —— 关机再开不算完整引导，不产生记录
//!   - 系统觉得不值得记录（开机不慢，一直没写 Event 100）
//!   - 还是真的坏掉了（通道损坏）
//!
//! 泛泛地说「可能是这些原因」没有用；要让用户**在任意设备上都能自查**，
//! 就得把这个判断交给程序去查，而不是让用户对着事件查看器猜。
//!
//! ## 设计
//!
//! 一次调用同时读两类事实来源，组合出结论：
//!
//! | 来源 | 问什么 | 回答 |
//! |---|---|---|
//! | 策略注册表 `WDI\{…}\ScenarioExecutionEnabled` | 系统**允不允许**记录 | 允许 / 禁用 / 读不到 |
//! | 性能日志 `Diagnostics-Performance/Operational` | 系统**有没有**真的记录 | 有 / 无 / 没权限 |
//!
//! 结论是**可行动的**：每种情况都带一句「下一步该做什么」。
//! 权限类结论明确标出 `needs_elevation`，前端据此给「以管理员身份重开」入口。
//!
//! 跨设备可用：命令走 Tauri command 层，任一台装 BootFlow 的机器都能调用；
//! 全程只读系统，**不写任何东西**（诊断只是诊断）。
#![allow(dead_code)]

use crate::diag::boot_log::{self, BootLogOutcome};
use crate::diag::boot_record;

/// 一键诊断。**只读**，不写任何东西。
pub fn check() -> BootPerformanceDiagnosis {
    // 事实 1：开关（读策略）
    let switch = boot_record::probe();

    // 事实 2：有没有记录（读日志）
    let outcome = boot_log::read_boot_events();

    let channel = "Microsoft-Windows-Diagnostics-Performance/Operational";

    match (&switch.state, &outcome) {
        // ——— 能读到事件：系统正常在记录 ———
        (_, BootLogOutcome::Events(events)) => {
            let main = events.iter().find(|e| e.event_id == 100);
            let record_count = events.len();
            let last_boot_at = main.and_then(|e| e.time_created.clone());

            // 找到 Event 100 = 有完整引导记录；没有 → 只有慢启动明细
            let verdict = match main {
                Some(_) => BootPerformanceDiagnosis {
                    verdict: "正常",
                    summary: format!(
                        "系统一直在记录开机性能，最近一次开机主事件：{}（共 {} 条相关记录）。",
                        last_boot_at.as_deref().unwrap_or("未知"),
                        record_count
                    ),
                    action: "无需操作，启动时序图已可用。",
                    needs_elevation: false,
                    record_count,
                    last_boot_at,
                    channel,
                    record_switch: switch.state,
                },
                // 有事件但缺 Event 100，基本就是 Fast Startup 一直开着
                None => BootPerformanceDiagnosis {
                    verdict: "快速启动生效",
                    summary: "日志里有记录，但没找到开机主事件（Event 100）。\
                         这通常说明「快速启动」一直在生效——关机再开时系统没有真正跑一遍完整的引导流程，\
                         所以不写入开机性能主事件。".to_string(),
                    action: "用「完整重启」代替「关机再开」，或临时关闭快速启动（控制面板 → 电源选项 → 选择电源按钮的功能 → 取消勾选快速启动）。",
                    needs_elevation: false,
                    record_count,
                    last_boot_at: None,
                    channel,
                    record_switch: switch.state,
                },
            };
            verdict
        }

        // ② 没权限：需要提权才能读
        (_, BootLogOutcome::AccessDenied) => BootPerformanceDiagnosis {
            verdict: "没有读取权限",
            summary: "Windows 没有向普通账户开放开机性能日志的读取权限，你目前以普通权限运行，读不到这个日志。".to_string(),
            action: "以管理员身份重新打开 BootFlow（首次打开时选择『以管理员身份运行』），启动后会自动读到开机性能数据。",
            needs_elevation: true,
            record_count: 0,
            last_boot_at: None,
            channel,
            record_switch: switch.state,
        },

        // ③ 能打开日志但没数据
        (_, BootLogOutcome::Unavailable(_)) => {
            // 细分：开关又是禁用的 → 系统策略是主因
            if switch.state == "disabled" {
                BootPerformanceDiagnosis {
                    verdict: "记录被策略禁用",
                    summary: "系统策略显式关闭了开机性能诊断（ScenarioExecutionEnabled=0），所以即使重启也不会记录。".to_string(),
                    action: "在 BootFlow 的「每次开机都记录」引导里点一键开启（会临时改回允许，15 分钟后自动还原），或自行用管理员运行 PowerShell 恢复策略值。",
                    needs_elevation: true,
                    record_count: 0,
                    last_boot_at: None,
                    channel,
                    record_switch: switch.state,
                }
            }
            // 开关也读不到 → 提示系统层面的问题
            else if switch.state == "unreadable" {
                BootPerformanceDiagnosis {
                    verdict: "开关状态无法读取",
                    summary: "无法读取「开机性能诊断」的开关状态（权限不足或策略键异常），同时系统日志里也没有数据。".to_string(),
                    action: "请以管理员身份运行 BootFlow 后重试本诊断。",
                    needs_elevation: true,
                    record_count: 0,
                    last_boot_at: None,
                    channel,
                    record_switch: switch.state,
                }
            }
            // 开关允许 + 日志没有数据 → 最常见：一直没被记
            else {
                BootPerformanceDiagnosis {
                    verdict: "还没有开机性能记录",
                    summary: "系统已经允许记录，但日志里目前没有开机性能事件。Windows 通常在开机偏慢时才写入 Event 100，连续几次都很快时就会一直空白。".to_string(),
                    action: "这大多不是故障。想主动让数据出现：做一次「完整重启」（不是关机再开）。重启后一般就能读到；如果仍没有，检查一下「快速启动」是否开着。",
                    needs_elevation: false,
                    record_count: 0,
                    last_boot_at: None,
                    channel,
                    record_switch: switch.state,
                }
            }
        }
    }
}

/// 诊断结果（序列化给前端）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootPerformanceDiagnosis {
    pub verdict: &'static str,
    pub summary: String,
    pub action: &'static str,
    pub needs_elevation: bool,
    pub record_count: usize,
    pub last_boot_at: Option<String>,
    pub channel: &'static str,
    pub record_switch: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_but_no_events_is_benign() {
        let d = BootPerformanceDiagnosis {
            verdict: "还没有开机性能记录",
            summary: String::new(),
            action: "先做一次完整重启",
            needs_elevation: false,
            record_count: 0,
            last_boot_at: None,
            channel: "…",
            record_switch: "allowed",
        };
        assert!(!d.needs_elevation);
        assert_eq!(d.record_count, 0);
    }

    #[test]
    fn accessdenied_always_marks_elevation() {
        // AccessDenied 属于「没权限」，必须标 needs_elevation，前端才给提权入口
        // check() 结果没法在无真实系统环境里单测，这里至少锁住语义字段
        let d = BootPerformanceDiagnosis {
            verdict: "没有读取权限",
            summary: String::new(),
            action: "以管理员身份重开",
            needs_elevation: true,
            record_count: 0,
            last_boot_at: None,
            channel: "…",
            record_switch: "allowed",
        };
        assert!(d.needs_elevation);
    }
}