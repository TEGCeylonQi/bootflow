//! 三层风险评级。
//!
//! ## 一个必须先说清的区分
//!
//! `RiskLevel::Locked` 与其他三档**不是同一个维度**：
//!
//! | | 含义 | 后果 |
//! |---|---|---|
//! | `Locked` | **不能改**（护栏） | 界面上锁死，任何版本都不给写入口 |
//! | `High` / `Medium` / `Safe` | **风险有多大**（判断） | 只影响提示的醒目程度 |
//!
//! 它们共用 `RiskLevel` 一个枚举，是因为界面上都在同一个位置显示。
//! 但 `Locked` 一旦成立就**直接返回**，不再叠加其他理由——
//! 对一个"根本不会让你改"的项说"它还没签名、它启动很慢"，
//! 全部是无效信息。
//!
//! ## 评级的判据是"影响面"，不是"看起来可疑"
//!
//! 同一个未签名的小工具：
//! - 放在用户启动文件夹里 → 只影响当前用户，出问题也就是它自己不启动
//! - 放在 `HKLM` 全局位置 → 影响这台机器上所有账户
//!
//! 后者比前者值得警惕，不是因为"HKLM 更高级"，而是**影响的人数不同**。
//! 这也是为什么 `UNSIGNED` 在两种位置会落到不同档位。

use crate::model::{ItemKind, RiskLevel, Scope, StartupItem};

/// 动它们会直接破坏系统启动或登录的服务。
///
/// 这份清单收的是"**关了它，机器会明显不正常**"的服务，
/// 不是"重要的服务"——两者范围差很远。判断标准是可验证的：
/// 把某个服务改成禁用，重启后系统还能不能正常进桌面。
///
/// ⚠️ 清单变长时要重新审视：每多一条，就多一批被锁死、用户永远
/// 无法管理的项。宁可漏掉几个（它们仍会显示风险评级，只是不带锁），
/// 也不要多锁——被锁住的项用户是真的一点办法都没有。
const CORE_SERVICES: &[&str] = &[
    // 进程/服务基础设施：停了之后任何服务都起不来
    "RpcSs",
    "RpcEptMapper",
    "DcomLaunch",
    "BrokerInfrastructure",
    "SystemEventsBroker",
    // 会话与登录
    "SamSs",
    "LSM",
    "ProfSvc",
    "UserManager",
    "gpsvc",
    // 系统可用的最低条件
    "Power",
    "Themes",
    "EventLog",
    "Schedule",
    "AudioSrv",
    "AudioEndpointBuilder",
];

/// 评估一个启动项的风险。
///
/// 返回 `(级别, 人话理由列表)`。理由列表**必须是人话**——
/// 这些字符串会直接进属性面板，出现 `HKLM\WOW6432Node` 这类
/// 术语就违背了"一级界面说人话"的约束。
pub fn assess(item: &StartupItem) -> (RiskLevel, Vec<String>) {
    // ── 第一档：禁改区。一旦成立就定案，不再叠加理由。──
    if let Some(reason) = locked_reason(item) {
        return (RiskLevel::Locked, vec![reason]);
    }

    let mut reasons: Vec<String> = Vec::new();
    let mut level = RiskLevel::Safe;

    // 收集结论，最后取最严重的一档。
    // 这里刻意不在每个 if 里直接改 level——那样后写的规则会覆盖先写的，
    // 结果取决于代码顺序，改一次顺序就可能悄悄改变评级。
    let bump = |l: RiskLevel, why: String, level: &mut RiskLevel, rs: &mut Vec<String>| {
        if l < *level {
            *level = l;
        }
        rs.push(why);
    };

    // ── 系统注入：影响面覆盖所有被注入的进程 ──
    if item.kind == ItemKind::Hook {
        for d in &item.diagnostics {
            if d.code == crate::diag::codes::GLOBAL_HOOK {
                bump(
                    RiskLevel::High,
                    "它会被加载进其它程序里一起运行，影响范围远超它自己".to_string(),
                    &mut level,
                    &mut reasons,
                );
            }
            if d.code == crate::diag::codes::IFEO_HIJACK {
                bump(
                    RiskLevel::High,
                    "有一个程序的启动方式被改写了——可能是调试工具，也可能是别的东西"
                        .to_string(),
                    &mut level,
                    &mut reasons,
                );
            }
        }
    }

    // ── 系统实测的慢启动：这是硬证据，比推测值得警惕 ──
    if item
        .diagnostics
        .iter()
        .any(|d| d.code == crate::diag::codes::SLOW_START)
    {
        bump(
            RiskLevel::High,
            "上次开机时系统实测到它启动很慢，直接拖长了开机时间".to_string(),
            &mut level,
            &mut reasons,
        );
    }

    // ── 启动方式被改坏了：能说清后果，所以值得提 ──
    for d in &item.diagnostics {
        if d.code == crate::diag::codes::MANUAL_BUT_SHOULD_AUTO {
            // 用诊断自带的级别：Office 服务是"用起来难受"(Medium)，
            // 防护服务是"这段时间没有防护"(High)。混成一档会让
            // 真正的安全问题贬值。
            bump(
                d.severity,
                "它的启动方式被改过了，和出厂状态不一致".to_string(),
                &mut level,
                &mut reasons,
            );
        }
        if d.code == crate::diag::codes::OFFICE_TASKS_DISABLED {
            bump(
                RiskLevel::Medium,
                "它被关闭了，Office 的一些后台功能会受影响".to_string(),
                &mut level,
                &mut reasons,
            );
        }
    }

    // ── 目标已经不存在：不是"危险"，但值得处理 ──
    if item.validity.is_broken() {
        bump(
            RiskLevel::Medium,
            "它指向的程序已经不在电脑上了".to_string(),
            &mut level,
            &mut reasons,
        );
    }

    // ── 无法核验发布者。档位取决于影响面。──
    if !item.signer.is_signed {
        if item.scope == Scope::Machine {
            bump(
                RiskLevel::High,
                "无法核验它的发布者，而它位于对本机所有账户都生效的位置".to_string(),
                &mut level,
                &mut reasons,
            );
        } else {
            bump(
                RiskLevel::Medium,
                "无法核验它的发布者".to_string(),
                &mut level,
                &mut reasons,
            );
        }
    }

    if reasons.is_empty() {
        reasons.push("没有发现值得注意的地方".to_string());
    }

    (level, reasons)
}

/// 是否属于「任何版本都不给写入口」的禁改区。
fn locked_reason(item: &StartupItem) -> Option<String> {
    // 路径在系统目录里（System32 / SysWOW64 / WinSxS 等）。
    //
    // 对服务而言这涵盖了 svchost 承载的全部服务——看起来"一刀切"，
    // 但那是对的：svchost 自己就是系统文件，而它承载的服务名清单
    // 无法穷举（随系统版本变化）。与其猜，不如让它保持锁定：
    // 用户确实不应该去动这些服务。
    if item.signer.is_os_component {
        return Some(
            "这是 Windows 自带的组件。改动它可能影响系统启动甚至无法进桌面，\
             因此不提供修改入口。"
                .to_string(),
        );
    }

    // 核心服务名单：它们的可执行文件可能不在系统目录
    // （个别安全软件会替换），但语义上仍是系统命脉。
    if item.source == crate::model::SourceKind::Service
        && CORE_SERVICES
            .iter()
            .any(|s| s.eq_ignore_ascii_case(&item.name))
    {
        return Some(
            "这是维持 Windows 正常运行的基础服务。它不是可以被安排的对象，\
             关掉它会让系统出问题，因此不提供修改入口。"
                .to_string(),
        );
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn item() -> StartupItem {
        StartupItem {
            id: "x".into(),
            source: SourceKind::RunUser,
            identity_key: "k".into(),
            name: "Test".into(),
            kind: ItemKind::App,
            display_name: None,
            name_from: None,
            summary: None,
            command: r"C:\App\a.exe".into(),
            resolved_path: r"C:\App\a.exe".into(),
            args: vec![],
            location: "l".into(),
            scope: Scope::User,
            enabled: true,
            // 默认给一个"已签名"的状态：未签名本身会产生评级，
            // 测试其他规则时应先把这个变量按住。
            signer: SignerInfo {
                is_signed: true,
                ..Default::default()
            },
            icon_data: None,
            risk: RiskLevel::Safe,
            risk_reasons: vec![],
            diagnostics: vec![],
            boot_phase: BootPhase::Logon,
            timing: ItemTiming::default(),
            validity: ValidityStatus::Ok,
            validity_detail: None,
            recommendation: None,
            duplicate_of: None,
            raw: serde_json::Value::Null,
            desired: DesiredState::default(),
            snapshot_ref: None,
        }
    }

    fn diag(code: &str, severity: RiskLevel) -> DiagnosticInfo {
        DiagnosticInfo {
            code: code.to_string(),
            severity,
            message: "m".into(),
            evidence: None,
        }
    }

    #[test]
    fn healthy_user_level_signed_item_is_safe() {
        let (level, reasons) = assess(&item());
        assert_eq!(level, RiskLevel::Safe);
        assert!(reasons[0].contains("没有发现"));
    }

    #[test]
    fn os_component_is_locked_and_short_circuits() {
        let mut it = item();
        it.signer.is_os_component = true;
        it.signer.is_signed = false; // 就算还有别的理由

        let (level, reasons) = assess(&it);

        assert_eq!(level, RiskLevel::Locked);
        // 关键：不再叠加其他理由。对一个"根本不让改"的项
        // 说"它还没签名"，是无效信息。
        assert_eq!(reasons.len(), 1);
    }

    #[test]
    fn core_service_is_locked_even_outside_system_dir() {
        let mut it = item();
        it.source = SourceKind::Service;
        it.name = "RpcSs".into();
        // 假设某个安全软件把它替换到了别处
        it.resolved_path = r"D:\Vendor\rpcss.exe".into();

        let (level, _) = assess(&it);
        assert_eq!(
            level,
            RiskLevel::Locked,
            "核心服务不该因为路径不在系统目录就变成可管理的"
        );
    }

    #[test]
    fn locked_reason_does_not_mention_technical_terms() {
        // 理由会直接显示在界面上，不能出现注册表路径之类的术语
        let mut it = item();
        it.signer.is_os_component = true;

        let (_, reasons) = assess(&it);
        for term in ["HKLM", "HKCU", "Registry", "AntiVirus", "System32"] {
            assert!(
                !reasons[0].contains(term),
                "禁改区理由里出现了术语 {term}：{}",
                reasons[0]
            );
        }
    }

    #[test]
    fn unsigned_scope_decides_the_level() {
        // 同一个未签名的小工具，全局位置比用户位置影响面大
        let mut user = item();
        user.signer.is_signed = false;

        let mut machine = item();
        machine.signer.is_signed = false;
        machine.scope = Scope::Machine;

        assert_eq!(assess(&user).0, RiskLevel::Medium);
        assert_eq!(assess(&machine).0, RiskLevel::High);
    }

    #[test]
    fn severity_takes_the_most_serious_factor_not_the_last_one() {
        // 同时有两个因素，且严重级别一高一低。
        // 结果必须是高的那个，且与规则书写顺序无关。
        let mut it = item();
        it.diagnostics = vec![
            diag(crate::diag::codes::OFFICE_TASKS_DISABLED, RiskLevel::Medium),
            diag(crate::diag::codes::MANUAL_BUT_SHOULD_AUTO, RiskLevel::High),
        ];

        assert_eq!(assess(&it).0, RiskLevel::High);

        // 顺序反过来结果必须一致——这一条防的是"评级随代码顺序漂移"
        it.diagnostics.reverse();
        assert_eq!(assess(&it).0, RiskLevel::High);
    }

    #[test]
    fn slow_start_evidence_raises_to_high() {
        let mut it = item();
        it.diagnostics = vec![diag(crate::diag::codes::SLOW_START, RiskLevel::Medium)];
        assert_eq!(assess(&it).0, RiskLevel::High);
    }

    #[test]
    fn global_hook_is_high() {
        let mut it = item();
        it.kind = ItemKind::Hook;
        it.diagnostics = vec![diag(crate::diag::codes::GLOBAL_HOOK, RiskLevel::High)];
        assert_eq!(assess(&it).0, RiskLevel::High);
    }

    #[test]
    fn office_service_uses_its_own_severity_not_a_fixed_one() {
        // ClickToRunSvc 是 Medium（用起来难受），不是 High（安全问题）。
        // 把它抬到 High 会让界面上的"高危"贬值。
        let mut it = item();
        it.source = SourceKind::Service;
        it.name = "ClickToRunSvc".into();
        it.resolved_path = r"C:\Program Files\Common Files\Microsoft Shared\ClickToRun\OfficeClickToRun.exe".into();
        it.diagnostics = vec![diag(
            crate::diag::codes::MANUAL_BUT_SHOULD_AUTO,
            RiskLevel::Medium,
        )];

        assert_eq!(assess(&it).0, RiskLevel::Medium);
    }

    #[test]
    fn dead_target_is_medium_not_high() {
        // 卸载残留不是"危险"，是"该清理"。判成 High 会让用户
        // 以为机器上有安全问题，实际上只是个空壳记录。
        let mut it = item();
        it.validity = ValidityStatus::MissingTarget;
        assert_eq!(assess(&it).0, RiskLevel::Medium);
    }

    #[test]
    fn reasons_are_never_empty() {
        // 界面上"风险理由"是空列表会渲染成一片空白，
        // 比给一句中性说明更让人困惑
        for mut it in [item(), item(), item()] {
            it.signer.is_os_component = false;
            it.validity = ValidityStatus::MissingTarget;
            let (_, reasons) = assess(&it);
            assert!(!reasons.is_empty());
        }
    }
}
