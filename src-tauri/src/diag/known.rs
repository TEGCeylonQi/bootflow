//! 「本该是什么样」的参照表。
//!
//! 这是整个项目里**唯一一处声称"我们知道正确答案"**的地方，因此也是
//! 最需要克制的地方。每一条都必须有客观依据，能在一台干净系统上复核，
//! 而不是"我觉得它应该是自动的"。
//!
//! ## 收录标准（三条全中才收）
//!
//! 1. **出厂默认状态明确**：Windows 或厂商默认就是自动/启用，不是我们的偏好。
//! 2. **被改掉有具体后果**：能说清"会发生什么"，而不是"不推荐"。
//! 3. **是常见误改目标**：第三方"优化"工具确实会动它。否则收了也用不上。
//!
//! ## 为什么必须克制
//!
//! 这张表一旦列错一条，后果不是"多一句提示"，而是**我们的判断开始不可信**：
//! 用户会看到一条与事实不符的建议，然后合理地怀疑其他所有结论。
//! 宁可少列（漏掉的项会在服务/任务清单里正常显示，只是没有额外提示），
//! 也不能多列。
//!
//! ## 反面例子：为什么 `BITS` 和 `wuauserv` 没有被收录
//!
//! 本机实测这两个服务都是 `DEMAND_START`（手动）——看起来很像"被优化软件关了"。
//! 但**手动就是它们的出厂默认**：Windows 按需拉起它们（检查更新、后台传文件时）。
//! 收进来会让每一台正常电脑都收到一条假警报。这一条差别的分辨方法只有一个：
//! 去一台没被"优化"过的机器上看它默认是什么。

use crate::model::RiskLevel;

/// 一条已知的"默认自动"服务。
pub struct KnownAutoService {
    /// 服务名（`sc qc` 里的 SERVICE_NAME，不是显示名）
    pub name: &'static str,
    /// 出厂默认启动类型，用于向用户交代依据
    pub default_start: &'static str,
    /// 它负责什么（人话）
    pub purpose: &'static str,
    /// 被改成手动/禁用之后的**具体**后果
    pub consequence: &'static str,
    /// 这条诊断的严重级别
    pub severity: RiskLevel,
}

/// 被改成手动/禁用就值得指出来的服务。
///
/// 只在**当前状态不是自动**时才会产生诊断，所以正常机器上这张表是静默的。
/// 本机（一台被优化过的 Win11）实测只有 `ClickToRunSvc` 命中——
/// 也就是说这张表在生产环境里的信噪比很高。
pub const SHOULD_BE_AUTO_SERVICES: &[KnownAutoService] = &[
    KnownAutoService {
        name: "ClickToRunSvc",
        default_start: "自动（延迟启动）",
        purpose: "让 Microsoft Office 按需启动与更新",
        consequence: "开机是快了一点，但每次第一次打开 Office 都要等它爬起来，\
                      会明显卡顿；Office 的自动更新也会一并停掉",
        // 不是安全问题，是"用起来难受"。归 Medium 而不是 High——
        // 把它和"防护被关掉"列成同一档会让真正的安全问题贬值。
        severity: RiskLevel::Medium,
    },
    KnownAutoService {
        name: "WinDefend",
        default_start: "自动",
        purpose: "Microsoft Defender 防病毒的实时防护",
        consequence: "这台电脑在开机后的一段时间里没有实时病毒防护",
        severity: RiskLevel::High,
    },
    KnownAutoService {
        name: "wscsvc",
        default_start: "自动（延迟启动）",
        purpose: "安全中心——汇总并上报杀毒、防火墙、更新状态",
        consequence: "安全中心不再工作，Windows 设置里会显示「你的设备可能存在风险」",
        severity: RiskLevel::High,
    },
    KnownAutoService {
        name: "Schedule",
        default_start: "自动",
        purpose: "任务计划程序——所有计划任务（含系统维护、驱动更新）都由它调起",
        consequence: "所有计划任务都不会按时运行，包括磁盘检查与系统维护",
        severity: RiskLevel::High,
    },
    KnownAutoService {
        name: "EventLog",
        default_start: "自动",
        purpose: "Windows 事件日志",
        consequence: "系统无法记录事件，出问题时没有任何日志可查",
        severity: RiskLevel::High,
    },
];

/// 按服务名查（不区分大小写）。
pub fn lookup_auto_service(name: &str) -> Option<&'static KnownAutoService> {
    SHOULD_BE_AUTO_SERVICES
        .iter()
        .find(|k| k.name.eq_ignore_ascii_case(name))
}

/// 一条已知的"默认启用"计划任务。
pub struct KnownEnabledTask {
    /// 任务完整路径（任务计划程序里的持久化路径）
    pub path: &'static str,
    /// 这个任务干什么（人话）
    pub purpose: &'static str,
}

/// Microsoft Office 安装时创建的后台任务。
///
/// ⚠️ 这一组与前一张表**语义不同**，措辞也必须不同。
///
/// 上面那些服务被改掉是明确的配置损坏；而 Office 这批后台任务被关掉
/// **对很多用户来说是故意的**——它们正是"Office 开机自启太多"这类优化教程
/// 的头号目标，关掉确实能减少登录时的后台活动。
///
/// 所以我们不说"你该把它打开"，只说"它们被关了，这是代价与收益，
/// 你自己判断"。把它渲染成一个需要修复的问题，等于在纠正用户的个人选择。
///
/// 那为什么还要提？因为**用户往往不知道是自己关的**——
/// 某次装了个"系统优化大师"，或者点过一次"一键加速"，这 5 个任务就没了。
/// 等到某天发现 Office 不自动更新、反复提示版本过旧时，
/// 没人会把这两件事联系起来。指出这种隐性代价，才是这条诊断的价值。
pub const OFFICE_BACKGROUND_TASKS: &[KnownEnabledTask] = &[
    KnownEnabledTask {
        path: r"\Microsoft\Office\Office Automatic Updates 2.0",
        purpose: "检查并下载 Office 更新",
    },
    KnownEnabledTask {
        path: r"\Microsoft\Office\Office Feature Updates Logon",
        purpose: "登录后补齐 Office 功能更新",
    },
    KnownEnabledTask {
        path: r"\Microsoft\Office\Office Background Push Maintenance",
        purpose: "维护 Office 的后台推送通道",
    },
    KnownEnabledTask {
        path: r"\Microsoft\Office\Office Actions Server",
        purpose: "为 Office 提供后台动作服务",
    },
    KnownEnabledTask {
        path: r"\Microsoft\Office\Office Startup Maintenance",
        purpose: "启动时整理 Office 的缓存与组件",
    },
];

/// 按任务路径查（不区分大小写）。
pub fn lookup_office_task(path: &str) -> Option<&'static KnownEnabledTask> {
    OFFICE_BACKGROUND_TASKS
        .iter()
        .find(|k| k.path.eq_ignore_ascii_case(path))
}

/// Office 后台任务是否处在"预期之外"的状态：任务被禁用。
///
/// 触发条件是**任务级禁用**。触发器级的关闭不算——Office 的这几个任务
/// 有些本来就只有个别触发器，判断标准放太宽会误报。
pub fn is_office_task_disabled(task_path: &str, task_enabled: bool) -> bool {
    lookup_office_task(task_path).is_some() && !task_enabled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_service_lookup_is_case_insensitive() {
        assert!(lookup_auto_service("ClickToRunSvc").is_some());
        assert!(lookup_auto_service("clicktorunsvc").is_some());
        assert!(lookup_auto_service("CLICKTORUNSVC").is_some());
        assert!(lookup_auto_service("NotAService").is_none());
    }

    /// 这两条服务的手动启动是**出厂设计**，收进白名单会让所有正常机器误报。
    /// 本机实测它们就是 `DEMAND_START`——正是这条测试要防的场景。
    #[test]
    fn demand_start_by_design_services_are_not_listed() {
        for name in ["BITS", "wuauserv", "defragsvc", "sppsvc"] {
            assert!(
                lookup_auto_service(name).is_none(),
                "{name} 的手动启动是 Windows 出厂设计，不能当成「被改坏了」"
            );
        }
    }

    #[test]
    fn office_task_paths_are_matched_exactly() {
        assert!(lookup_office_task(r"\Microsoft\Office\Office Automatic Updates 2.0").is_some());
        assert!(lookup_office_task(r"\microsoft\office\office automatic updates 2.0").is_some());
        // 子路径与前缀不能误命中
        assert!(lookup_office_task(r"\Microsoft\Office\Office Automatic Updates").is_none());
        assert!(lookup_office_task(r"\Microsoft\Windows\Defrag\ScheduledDefrag").is_none());
    }

    #[test]
    fn office_disabled_detection_requires_task_level_disable() {
        let p = r"\Microsoft\Office\Office Feature Updates Logon";
        assert!(is_office_task_disabled(p, false));
        assert!(!is_office_task_disabled(p, true));
        // 不在表里的任务，无论状态如何都不产生这条诊断
        assert!(!is_office_task_disabled(r"\Some\Other\Task", false));
    }
}
