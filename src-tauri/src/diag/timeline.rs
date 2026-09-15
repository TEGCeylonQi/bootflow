//! 开机时间轴的构建，以及把耗时数据对应回启动项。
//!
//! ## 相位是怎么划出来的
//!
//! 事件 100 给了一批**绝对时间锚点**（`BootPNPInitStartTimeMS` /
//! `SystemPNPInitStartTimeMS` / `SessionInitStartTimeMS` / `WinLogonStartTimeMS`），
//! 这比"把各个 Duration 字段累加"可靠得多——累加一旦哪个字段没写，
//! 后面所有相位就整体错位；而锚点缺失时只损失那一段，其它段不受影响。
//!
//! ```text
//!   0 ──────────────┬──────────────┬───────────┬────────────┬───→ BootTime
//!   Kernel          │ Driver       │ Devices   │ Smss       │ …
//!                   │              │           │            │
//!         BootPNPInit┘  SystemPNPInit┘  SessionInit┘  WinLogon┘
//! ```
//!
//! 锚点缺一个，就**不画那一段**——而不是用别的字段凑一个看起来连续的图。
//! 一段真实的空白，比一段编出来的连续更诚实。
//!
//! ## 为什么"慢启动"要有阈值
//!
//! Event 103 是**按 Windows 自己的严格标准**触发的，门槛很低：
//! 公开样本里 326ms / 405ms 这种量级的记录很常见。
//! 把它们全部渲染成"需要你关注"，用户会被几十条无害提示淹没，
//! 真正严重的那条反而看不见了。
//!
//! 所以数据分两层处理：
//! - `slow_services` **保留全部**记录（导出报告时不丢信息）
//! - 只有"多花的时间"超过 `MIN_REPORTED_DEGRADATION_MS` 的，
//!   才在启动项上挂 `SLOW_START` 诊断

use crate::diag::boot_log::{BootLogOutcome, RawEvent};
use crate::model::{
    BootPhase, BootTimeline, Confidence, DiagnosticInfo, ItemTiming, PhaseSpan, RiskLevel,
    SlowService, SourceKind, StartupItem,
};

/// 产生"值得看一眼"提示的门槛（毫秒）。
///
/// 取 500ms 的理由：这是在实机上"能感知到"的下限——
/// 300ms 的额外等待用户的注意力根本捕捉不到，提示出来只会稀释信号。
/// 数据本身不受影响，`slow_services` 里仍然完整保留。
const MIN_REPORTED_DEGRADATION_MS: u64 = 500;

/// 从读取结果构建时间轴。
pub fn build(outcome: &BootLogOutcome) -> BootTimeline {
    match outcome {
        // 权限问题要单独标出来：界面据此给出「以管理员身份重试」，
        // 而"日志没数据"给这个按钮是没用的。
        BootLogOutcome::AccessDenied => BootTimeline {
            unavailable_reason: Some(
                "读取开机耗时需要管理员权限。Windows 的性能日志没有向普通账户开放读取权限。"
                    .to_string(),
            ),
            needs_elevation: true,
            ..Default::default()
        },
        BootLogOutcome::Unavailable(why) => BootTimeline {
            unavailable_reason: Some(why.clone()),
            needs_elevation: false,
            ..Default::default()
        },
        BootLogOutcome::Events(events) => build_from_events(events),
    }
}

fn build_from_events(events: &[RawEvent]) -> BootTimeline {
    // 事件是倒序读出来的，第一条 100 就是最近一次开机
    let Some(main) = events.iter().find(|e| e.event_id == 100) else {
        return BootTimeline {
            unavailable_reason: Some(
                "日志里没有找到开机主事件，可能是「快速启动」一直在生效，\
                 系统没有走完整的引导流程"
                    .to_string(),
            ),
            ..Default::default()
        };
    };

    let main_path = main.get_u64("MainPathBootTime");
    let boot_time = main.get_u64("BootTime");
    let post_boot = main.get_u64("BootPostBootTime");

    let mut timeline = BootTimeline {
        total_boot_ms: boot_time,
        phases: phases_of(main),
        slow_services: collect_slow(events),
        boot_started_at: main.get("BootStartTime").map(str::to_string),
        unavailable_reason: None,
        needs_elevation: false,
    };

    // 桌面出现之后到"系统真正可用"这一段（PostBoot）单独成段。
    //
    // 它不被 `MainPathBootTime` 覆盖，是超出"主路径"的额外等待，
    // 而恰恰是登录后各种自启程序抢 IO 最凶的时候——
    // 对这类工具来说，这一段是最该被看见的。
    if let (Some(main_path), Some(boot_time)) = (main_path, boot_time) {
        if boot_time > main_path {
            timeline.phases.push(PhaseSpan {
                name: BootPhase::Logon,
                start_ms: main_path,
                end_ms: boot_time,
            });
        }
    }

    // ⚠️ 兜底：Event 100 存在、但上面无论如何都推不出任何一段时
    //（version 1 老模板连 BootTime/MainPathBootTime 都没有，或都是 0），
    // 面向用户的耗时页会误判成"系统没记录"——而诊断只看"有没有 Event 100"
    // 判正常，两者必然打架。这里用能找到的最大总长兜成一段，
    // 让耗时页至少画出时长，不让用户以为"系统觉得不值得记"。
    if timeline.phases.is_empty() {
        if let Some(total) = boot_time.filter(|&t| t > 0) {
            // 只有 PostBoot 会超出主路径；连主路径都没有时，总时长本身就是完整的一段
            timeline.phases.push(PhaseSpan {
                name: BootPhase::Logon,
                start_ms: 0,
                end_ms: total,
            });
            log::info!("Event 100 无可用相位，用总时长 {total}ms 兜底一段");
        } else if let Some(mp) = main_path.filter(|&v| v > 0) {
            timeline.phases.push(PhaseSpan {
                name: BootPhase::Logon,
                start_ms: 0,
                end_ms: mp,
            });
            log::info!("Event 100 无相位与总时长，退回 MainPathBootTime {mp}ms 兜底一段");
        }
    }

    // 内部一致性自检：相位不应越界到总时长之外
    if let Some(total) = boot_time {
        let overshoot = timeline
            .phases
            .iter()
            .filter(|p| p.end_ms > total)
            .count();
        if overshoot > 0 {
            log::warn!("有 {overshoot} 个相位的结束时间超过了开机总时长，已按总时长截断");
            for p in timeline.phases.iter_mut() {
                p.end_ms = p.end_ms.min(total);
            }
            timeline.phases.retain(|p| p.end_ms > p.start_ms);
        }
    }

    // PostBoot 存在与否影响"总时长"的解释，这里显式记一笔日志便于排查
    if post_boot.is_none() {
        log::debug!("事件 100 缺少 BootPostBootTime，未单独绘制登录后阶段");
    }

    timeline
}

/// 从事件 100 里切出各个相位。
fn phases_of(ev: &RawEvent) -> Vec<PhaseSpan> {
    let mut out = Vec::new();

    // 绝对锚点。缺任何一个，依赖它的那一段就不画。
    let boot_pnp = ev.get_u64("BootPNPInitStartTimeMS");
    let sys_pnp = ev.get_u64("SystemPNPInitStartTimeMS");
    let session = ev.get_u64("SessionInitStartTimeMS");
    let winlogon = ev.get_u64("WinLogonStartTimeMS");

    let mut push = |name: BootPhase, start: u64, end: u64| {
        // 零长度或倒序的段一律丢掉——那是字段缺失/错位，不是"耗时 0"
        if end > start {
            out.push(PhaseSpan {
                name,
                start_ms: start,
                end_ms: end,
            });
        }
    };

    // Kernel：从计时起点到引导期即插即用初始化开始
    if let Some(bp) = boot_pnp {
        push(BootPhase::Kernel, 0, bp);
    }
    // Driver：引导期驱动/设备枚举
    if let (Some(bp), Some(sp)) = (boot_pnp, sys_pnp) {
        push(BootPhase::Driver, bp, sp);
    }
    // Devices：系统期设备初始化
    if let (Some(sp), Some(se)) = (sys_pnp, session) {
        push(BootPhase::Devices, sp, se);
    }
    // Smss：会话管理器初始化（服务在这一段被拉起）
    if let (Some(se), Some(wl)) = (session, winlogon) {
        push(BootPhase::Smss, se, wl);
    }

    // Winlogon 之后是用户会话的部分。这里用"锚点 + 各活动耗时"逐段推进，
    // 而不是靠累加——累加会把缺失字段的影响扩散到后面每一段。
    if let Some(wl) = winlogon {
        let mut cursor = wl;

        // UserAuth：等用户完成登录（含自动登录的等待）
        if let Some(wait) = ev.get_u64("UserLogonWaitDuration") {
            push(BootPhase::UserAuth, cursor, cursor + wait);
            cursor += wait;
        }

        // UserInit：准备用户配置文件
        if let Some(profile) = ev.get_u64("BootUserProfileProcessingTime") {
            push(BootPhase::UserInit, cursor, cursor + profile);
            cursor += profile;
        }

        // Shell：启动桌面外壳。这是登录后最重的一段，
        // 也是绝大多数自启程序真正开始抢资源的地方。
        if let Some(shell) = ev.get_u64("BootExplorerInitTime") {
            push(BootPhase::Shell, cursor, cursor + shell);
        }
    }

    out
}

/// 汇总所有"慢启动"记录。
///
/// 同一项可能在多次开机里都被记录（日志保留了历史）。按名字去重，
/// **保留事件顺序里的第一条**——事件是倒序读的，所以第一条就是最近一次。
fn collect_slow(events: &[RawEvent]) -> Vec<SlowService> {
    let mut out: Vec<SlowService> = Vec::new();

    for ev in events {
        if !matches!(ev.event_id, 101..=103) {
            continue;
        }
        let Some(name) = ev.get("Name") else { continue };
        if name.trim().is_empty() {
            continue;
        }

        if out.iter().any(|s| s.name.eq_ignore_ascii_case(name)) {
            continue;
        }

        out.push(SlowService {
            name: name.to_string(),
            friendly_name: ev.get("FriendlyName").map(str::to_string),
            // TotalTime 缺失时按 0 记录而不是丢掉这条——
            // "系统记录了它慢但没给出耗时"本身也是有价值的信息。
            duration_ms: ev.get_u64("TotalTime").unwrap_or(0),
            degradation_ms: ev.get_u64("DegradationTime").unwrap_or(0),
            event_id: ev.event_id,
        });
    }

    // 最慢的排前面
    out.sort_by_key(|s| std::cmp::Reverse(s.degradation_ms));
    out
}

/// 把时间轴数据对应回具体启动项。
///
/// 两种精度分别处理，**绝不含糊**：
/// - 命中慢启动事件的项 → `Measured`，带真实耗时（来自 Event 103 等服务）
/// - 其余项 → `Estimated`，**只给一个估算起点，不给时长**
///
/// 为什么不给估算时长：Windows 根本不记录单个自启项的耗时。
/// 相位区间是"N 项挤在这段时间里"，把区间长度摊给每一项，
/// 得到的数字看着精确、实际是编的。产品原则里"不伪造精度"指的就是这个。
pub fn apply_to_items(items: &mut [StartupItem], timeline: &BootTimeline) {
    for item in items.iter_mut() {
        // ── 第一优先：命中系统实测的慢启动记录 ──
        if let Some(slow) = find_slow_entry(item, &timeline.slow_services) {
            item.timing = ItemTiming {
                confidence: Confidence::Measured,
                start_estimate_ms: None,
                duration_ms: Some(slow.duration_ms),
                source_event_id: Some(slow.event_id),
            };

            if slow.degradation_ms >= MIN_REPORTED_DEGRADATION_MS {
                item.diagnostics.push(DiagnosticInfo {
                    code: crate::diag::codes::SLOW_START.to_string(),
                    severity: RiskLevel::Medium,
                    message: format!(
                        "系统记录了它上一次开机启动花了 {:.1} 秒，其中 {:.1} 秒是超出正常水平的。",
                        slow.duration_ms as f64 / 1000.0,
                        slow.degradation_ms as f64 / 1000.0
                    ),
                    evidence: Some(format!(
                        "EventID={}，TotalTime={}ms，DegradationTime={}ms",
                        slow.event_id, slow.duration_ms, slow.degradation_ms
                    )),
                });
            }
            continue;
        }

        // ── 第二优先：按开机相位给一个估算起点 ──
        //
        // 只在确实拿到了时间轴时给。没有时间轴还硬给一个起点，
        // 等于凭空编一个"它在这一刻启动"。
        if timeline.phases.is_empty() {
            continue;
        }

        let phase = item.boot_phase;
        if phase == BootPhase::Unknown {
            continue;
        }

        if let Some(span) = timeline.phases.iter().find(|p| p.name == phase) {
            item.timing = ItemTiming {
                confidence: Confidence::Estimated,
                start_estimate_ms: Some(span.start_ms),
                // 刻意留空：见函数头注释
                duration_ms: None,
                source_event_id: Some(100),
            };
        }
    }
}

/// 一条慢启动记录是否对应这个启动项。
///
/// 匹配规则按事件类型的语义分别定，不能一概用"名字包含"——
/// 那会把 `eventlog` 匹配到 `EventLogViewer.exe` 之类的无关项上。
fn find_slow_entry<'a>(
    item: &StartupItem,
    slow: &'a [SlowService],
) -> Option<&'a SlowService> {
    slow.iter().find(|s| matches_item(item, s))
}

fn matches_item(item: &StartupItem, slow: &SlowService) -> bool {
    let name = slow.name.trim();
    if name.is_empty() {
        return false;
    }

    // 服务型事件（103）：事件里的 Name 就是**服务名**，
    // 而服务扫描器写进 `name` 的正是服务名，所以这一条是精确匹配。
    if slow.event_id == 103 && item.source == SourceKind::Service {
        return item.name.eq_ignore_ascii_case(name);
    }

    // 应用型（101）/ 驱动型（102）：事件里的 Name 是文件名（可带扩展名）。
    // 按目标路径的文件名比。
    let file_name = item
        .resolved_path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
        .trim();

    if file_name.is_empty() {
        return false;
    }

    // 两边都可能带或不带扩展名（`windefend` vs `windefend.exe`），
    // 所以剥掉扩展名后再比。
    let strip = |s: &str| {
        s.rsplit_once('.')
            .map(|(stem, _)| stem)
            .unwrap_or(s)
            .to_ascii_lowercase()
    };

    strip(file_name) == strip(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::boot_log::BootLogOutcome;

    fn ev(id: u32, pairs: &[(&str, &str)]) -> RawEvent {
        RawEvent {
            event_id: id,
            data: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            time_created: None,
            computer: None,
        }
    }

    /// 一组自洽的相位锚点（数字参照公开的真实事件样本量级）
    fn boot_event() -> RawEvent {
        ev(
            100,
            &[
                ("BootStartTime", "2026-03-17T18:13:15Z"),
                ("BootTime", "165331"),
                ("MainPathBootTime", "151680"),
                ("BootPostBootTime", "13651"),
                ("BootPNPInitStartTimeMS", "38"),
                ("SystemPNPInitStartTimeMS", "6925"),
                ("SessionInitStartTimeMS", "14143"),
                ("WinLogonStartTimeMS", "36087"),
                ("UserLogonWaitDuration", "9820"),
                ("BootUserProfileProcessingTime", "4098"),
                ("BootExplorerInitTime", "104515"),
            ],
        )
    }

    #[test]
    fn phases_are_built_from_absolute_anchors() {
        let tl = build_from_events(&[boot_event()]);

        let names: Vec<BootPhase> = tl.phases.iter().map(|p| p.name).collect();
        assert_eq!(
            &names[..5],
            &[
                BootPhase::Kernel,
                BootPhase::Driver,
                BootPhase::Devices,
                BootPhase::Smss,
                BootPhase::UserAuth
            ]
        );

        // Kernel 必须从 0 开始，否则甘特图的左边缘是悬空的
        assert_eq!(tl.phases[0].start_ms, 0);
        assert_eq!(tl.phases[0].end_ms, 38);

        // 相邻相位首尾相接（Driver 结束 = Devices 开始）
        assert_eq!(tl.phases[1].end_ms, tl.phases[2].start_ms);
    }

    #[test]
    fn post_boot_becomes_its_own_phase() {
        let tl = build_from_events(&[boot_event()]);

        let last = tl.phases.last().unwrap();
        assert_eq!(last.name, BootPhase::Logon);
        assert_eq!(last.start_ms, 151680, "登录后阶段应从主路径结束处开始");
        assert_eq!(last.end_ms, 165331);

        // 这一段的存在与否，直接决定"用户等了多少"是否被看见
        assert!(
            last.end_ms - last.start_ms == 13651,
            "PostBoot 段长度应等于 BootPostBootTime"
        );
    }

    #[test]
    fn missing_anchor_drops_only_that_phase() {
        // 去掉 SessionInitStartTimeMS：Smss 段失去起点，但不该影响别的段
        let mut e = boot_event();
        e.data.remove("SessionInitStartTimeMS");

        let tl = build_from_events(&[e]);
        let names: Vec<BootPhase> = tl.phases.iter().map(|p| p.name).collect();

        assert!(!names.contains(&BootPhase::Smss), "缺锚点的段不该出现");
        assert!(names.contains(&BootPhase::Driver), "其他段不该受牵连");
        assert!(names.contains(&BootPhase::Logon), "PostBoot 不依赖该锚点");
    }

    #[test]
    fn no_invented_phases_when_anchors_are_missing() {
        // 只给 BootTime 与 MainPathBootTime，一个绝对锚点都没有。
        //
        // 结果里**只该有**由这两个值直接得出的"登录后"阶段，
        // 而不能凭空造出 Kernel / Driver / Smss 这些需要锚点才能定位的段。
        // 一段真实的空白，比一段编出来的连续更诚实。
        let e = ev(100, &[("BootTime", "90000"), ("MainPathBootTime", "80000")]);

        let tl = build_from_events(&[e]);

        assert_eq!(tl.phases.len(), 1, "只应有一个由主路径/总时长得出的阶段");
        assert_eq!(tl.phases[0].name, BootPhase::Logon);
        assert_eq!(tl.phases[0].start_ms, 80000);
        assert_eq!(tl.phases[0].end_ms, 90000);
        assert_eq!(tl.total_boot_ms, Some(90000), "总时长仍应如实给出");
    }

    #[test]
    fn bare_event_with_total_falls_back_to_total() {
        // 兜底语义：Event 100 只有总时长（没有分段锚点，也没有主路径）时，
        // 耗时页必须能看到这段时长——不能因为拆不出相位就变成"没记录"。
        let e2 = ev(100, &[("BootTime", "120000")]);
        let tl2 = build_from_events(&[e2]);
        assert_eq!(tl2.phases.len(), 1);
        assert_eq!(tl2.phases[0].name, BootPhase::Logon);
        assert_eq!(tl2.phases[0].start_ms, 0);
        assert_eq!(tl2.phases[0].end_ms, 120000);
        assert_eq!(tl2.total_boot_ms, Some(120000));
    }

    #[test]
    fn phases_beyond_total_are_clamped() {
        // 字段错位导致相位越过总时长时，宁可截断也不能画出超界的条
        let mut e = boot_event();
        e.data.insert("WinLogonStartTimeMS".into(), "999999".into());
        e.data.insert("BootExplorerInitTime".into(), "0".into());

        let tl = build_from_events(&[e]);
        let total = tl.total_boot_ms.unwrap();
        assert!(tl.phases.iter().all(|p| p.end_ms <= total));
    }

    #[test]
    fn access_denied_is_reported_as_elevation_need() {
        // 这是本模块最重要的行为：读不到时必须说清是"没权限"，
        // 而不是返回空时间轴让用户以为开机耗时为 0。
        let tl = build(&BootLogOutcome::AccessDenied);

        assert!(tl.needs_elevation, "权限问题要标记出来，界面才能给提权入口");
        assert!(tl.unavailable_reason.is_some());
        assert!(tl.phases.is_empty());
    }

    #[test]
    fn unavailable_log_is_not_reported_as_elevation_need() {
        // 反面：日志本身没数据时，给提权按钮是没用的
        let tl = build(&BootLogOutcome::Unavailable("日志未启用".into()));
        assert!(!tl.needs_elevation, "这不是权限问题");
        assert!(tl.unavailable_reason.unwrap().contains("日志未启用"));
    }

    // ─────────── 慢启动匹配 ───────────

    fn svc_item(name: &str) -> StartupItem {
        StartupItem {
            id: "x".into(),
            source: SourceKind::Service,
            identity_key: "k".into(),
            name: name.into(),
            kind: crate::model::ItemKind::Service,
            display_name: None,
            name_from: None,
            summary: None,
            command: String::new(),
            resolved_path: String::new(),
            args: vec![],
            location: String::new(),
            scope: crate::model::Scope::Machine,
            enabled: true,
            signer: Default::default(),
            icon_data: None,
            risk: RiskLevel::Safe,
            risk_reasons: vec![],
            diagnostics: vec![],
            boot_phase: BootPhase::Smss,
            timing: Default::default(),
            validity: crate::model::ValidityStatus::Ok,
            validity_detail: None,
            recommendation: None,
            duplicate_of: None,
            raw: serde_json::Value::Null,
            desired: Default::default(),
            snapshot_ref: None,
        }
    }

    fn slow(name: &str, event_id: u32, total: u64, degrad: u64) -> SlowService {
        SlowService {
            name: name.into(),
            friendly_name: None,
            duration_ms: total,
            degradation_ms: degrad,
            event_id,
        }
    }

    #[test]
    fn service_slow_event_matches_by_service_name() {
        assert!(matches_item(
            &svc_item("windefend"),
            &slow("windefend", 103, 326, 234)
        ));
        // 大小写不该影响匹配
        assert!(matches_item(
            &svc_item("WinDefend"),
            &slow("windefend", 103, 326, 234)
        ));
    }

    #[test]
    fn service_event_does_not_match_by_file_name() {
        // 事件 103 的 Name 是服务名。若按文件名比，
        // 一个叫 "eventlog" 的文件会被误配到同名的其它东西上。
        let mut it = svc_item("MyService");
        it.resolved_path = r"C:\Program Files\eventlog\eventlog.exe".into();

        assert!(
            !matches_item(&it, &slow("eventlog", 103, 900, 700)),
            "服务型事件只该按服务名匹配"
        );
    }

    #[test]
    fn app_event_matches_by_file_name_with_or_without_extension() {
        let mut it = svc_item("Whatever");
        it.source = SourceKind::RunUser;
        it.resolved_path = r"C:\Program Files\App\MsMpEng.exe".into();

        assert!(matches_item(&it, &slow("MsMpEng.exe", 101, 5632, 2278)));
        assert!(matches_item(&it, &slow("MsMpEng", 101, 5632, 2278)));
        assert!(!matches_item(&it, &slow("OtherApp.exe", 101, 1, 1)));
    }

    #[test]
    fn measured_timing_is_applied_from_event() {
        let mut items = vec![svc_item("windefend")];
        let tl = BootTimeline {
            slow_services: vec![slow("windefend", 103, 326, 234)],
            phases: vec![],
            ..Default::default()
        };

        apply_to_items(&mut items, &tl);

        assert_eq!(items[0].timing.confidence, Confidence::Measured);
        assert_eq!(items[0].timing.duration_ms, Some(326));
        assert_eq!(items[0].timing.source_event_id, Some(103));
    }

    #[test]
    fn trivial_slowdown_does_not_produce_a_diagnostic() {
        // Event 103 的门槛很低（公开样本里有 326ms 这种）。全都提示出来
        // 会让"慢启动"这个信号贬值，真正严重的那条反而没人看。
        let mut items = vec![svc_item("windefend")];
        let tl = BootTimeline {
            slow_services: vec![slow("windefend", 103, 326, 234)],
            ..Default::default()
        };

        apply_to_items(&mut items, &tl);

        assert!(items[0].diagnostics.is_empty(), "轻微的记录不该成为提示");
        assert_eq!(
            items[0].timing.confidence,
            Confidence::Measured,
            "但不该丢掉数据本身"
        );
    }

    #[test]
    fn significant_slowdown_produces_a_diagnostic_with_evidence() {
        let mut items = vec![svc_item("windefend")];
        let tl = BootTimeline {
            slow_services: vec![slow("windefend", 103, 5200, 4800)],
            ..Default::default()
        };

        apply_to_items(&mut items, &tl);

        let d = &items[0].diagnostics[0];
        assert_eq!(d.code, crate::diag::codes::SLOW_START);
        // 证据里要有可复核的原始数值
        assert!(d.evidence.as_deref().unwrap().contains("4800ms"));
    }

    #[test]
    fn estimated_timing_gives_a_start_but_never_a_duration() {
        // 这是"诚实原则"最关键的一处：Windows 不记录单个自启项的耗时，
        // 所以除了实测命中项之外，一律不给时长。
        let mut it = svc_item("SomeAutoService");
        it.boot_phase = BootPhase::Smss;
        let mut items = vec![it];

        let tl = build_from_events(&[boot_event()]);
        apply_to_items(&mut items, &tl);

        assert_eq!(items[0].timing.confidence, Confidence::Estimated);
        assert!(
            items[0].timing.duration_ms.is_none(),
            "估算绝不能给出具体耗时——那个数字会是编的"
        );
        assert_eq!(items[0].timing.start_estimate_ms, Some(14143));
    }

    #[test]
    fn unknown_phase_gets_no_estimate() {
        let mut it = svc_item("Mystery");
        it.boot_phase = BootPhase::Unknown;
        let mut items = vec![it];

        apply_to_items(&mut items, &build_from_events(&[boot_event()]));

        assert_eq!(items[0].timing.confidence, Confidence::None);
        assert!(items[0].timing.start_estimate_ms.is_none());
    }

    #[test]
    fn no_timeline_means_no_estimates() {
        // 读不到数据时，必须什么都不给——而不是拿一个空时间轴硬算
        let mut items = vec![svc_item("SomeAutoService")];
        let tl = build(&BootLogOutcome::AccessDenied);

        apply_to_items(&mut items, &tl);

        assert_eq!(items[0].timing.confidence, Confidence::None);
        assert!(items[0].timing.start_estimate_ms.is_none());
    }

    #[test]
    fn slow_services_are_deduplicated_keeping_the_latest() {
        // 日志保留多次开机记录，同一个服务可能出现多遍。
        // 事件是倒序读的，所以应保留第一条（最近一次）。
        let events = vec![
            ev(103, &[("Name", "windefend"), ("TotalTime", "999"), ("DegradationTime", "888")]),
            ev(103, &[("Name", "windefend"), ("TotalTime", "111"), ("DegradationTime", "100")]),
            ev(103, &[("Name", "eventlog"), ("TotalTime", "500"), ("DegradationTime", "400")]),
        ];

        let tl = build_from_events(
            &[boot_event()]
                .into_iter()
                .chain(events)
                .collect::<Vec<_>>(),
        );

        assert_eq!(tl.slow_services.len(), 2, "同一服务不该出现两次");
        let wd = tl
            .slow_services
            .iter()
            .find(|s| s.name == "windefend")
            .unwrap();
        assert_eq!(wd.duration_ms, 999, "应保留最近一次（排序后靠前的那条）");
    }

    /// 真机探测：打印一次真实的 timeline，确认耗时页到底有没有相位可画。
    /// 读真实系统且结果与机器状态相关 → 标 ignore,真机手动跑。
    /// 运行：cargo test --lib probe_real_timeline -- --ignored --nocapture
    #[test]
    #[ignore]
    fn probe_real_timeline() {
        let outcome = crate::diag::boot_log::read_boot_events();
        match &outcome {
            BootLogOutcome::Events(evs) => {
                let tl = build(&outcome);
                println!(
                    "PROBE events={} total={:?} phases={}",
                    evs.len(),
                    tl.total_boot_ms,
                    tl.phases.len()
                );
                for p in &tl.phases {
                    println!("  phase {:?} {}→{}", p.name, p.start_ms, p.end_ms);
                }
                println!("  unavailable_reason={:?}", tl.unavailable_reason);
            }
            other => println!("PROBE outcome={:?}", other),
        }
    }
}
