//! 把开机开销归因到**具体启动项**。
//!
//! ## 四条通路，各自如实标注
//!
//! 启动链上的三个执行者——Explorer 拉注册表 Run 键、SCM 拉服务、
//! Task Scheduler 拉计划任务——**谁都不为自己的孩子打点**，
//! 所以"某个启动项花了多少毫秒"这个量并不存在。但"它有多重"是存在的，
//! 而且有四份精度不同的数据，本模块**分档如实标注**，绝不混为一谈：
//!
//! | 档 | 来源 | 数据 | 界面措辞 |
//! |---|---|---|---|
//! | 实测耗时 | Event 101/102/103 | `TotalTime` | 「启动花了 3.2 秒」 |
//! | **实测影响** | **WDI `StartupInfo`** | **CPU 时间 + 磁盘 IO** | 「启动影响：高 · CPU 1.4s · 磁盘 8.2MB」 |
//! | 实测出现时刻 | 进程快照（内核） | 进程创建时刻 | 「开机后第 12.4 秒出现」 |
//! | 估算 | Event 100 相位 | 所属相位的起点 | 「~」前缀，不给时长 |
//!
//! **为什么没有第五档"估算耗时"**：把相位区间长度摊给区间里的每一项，
//! 得到的数字看着精确、实际是编的。同相位 N 项拿到同一个值，排序都排不出来。
//! 产品原则里"不伪造精度"针对的就是这个。
//!
//! ⚠️ 第三档与第二档**不是一回事，也不能互相换算**：CPU 时间可以跨核累加，
//! 一项的 CPU 时间超过窗口长度是正常的；而"出现时刻"甚至不是耗时。
//! 三者都拿到时，详情面板要**分开**陈述，不许合成一个数字。
//!
//! ## 匹配为什么不能只按名字包含
//!
//! `svchost.exe` 承载几十个服务、注册表里同一个程序可能有多条记录——
//! 按名字匹配会把一条服务记录同时挂到十几个项上，界面看起来一切正常，
//! 实际全是错的（这个坑本项目在 `StartupItem.id` 上已经踩过一次）。
//! 所以观测归因有一条硬规则：**一个可执行文件只能对应唯一一个启动项**，
//! 否则一律放弃归因，退到估算档。宁可不给，也不给错的。

use std::collections::HashMap;

use crate::diag::boot_log::RawEvent;
use crate::diag::proc_snapshot::{ProcSnapshot, Snapshot};
use crate::diag::startup_info::{StartupInfoRecord, StartupInfoReport, level_of};
use crate::model::{
    BootPhase, BootTimeline, Confidence, DiagnosticInfo, ImpactOverview, ItemImpact, ItemTiming,
    RiskLevel, SlowService, SourceKind, StartupItem,
};

/// 产生"值得看一眼"提示的门槛（毫秒）。
///
/// 取 500ms 的理由：这是在实机上"能感知到"的下限——
/// 300ms 的额外等待用户的注意力根本捕捉不到，提示出来只会稀释信号。
/// 数据本身不受影响，`slow_services` 里仍然完整保留。
///
/// ⚠️ 它**只影响要不要出一条诊断**，不影响数据准确性——
/// 低于门槛的记录照样写进 `timing`（见 `attribute`）。
const MIN_REPORTED_DEGRADATION_MS: u64 = 500;

/// 把三类数据归因到启动项，就地写入 `item.timing` 与诊断。
///
/// ## 两个字段是**互相独立**的，可以同时存在
///
/// `duration_ms`（事件日志测的耗时）与 `observed_start_ms`（内核记的出现时刻）
/// 来自两条互不相干的通路。一个项**完全可能两者都有**——而那一对组合恰好是
/// 最有用的一条数据：*它在开机后第 12.4 秒出现，然后花了 3.2 秒*。
///
/// 所以这里的规则是：**每个字段只在真的拿到该数据时才填**，不因为"已经归到
/// 高档了"就丢掉低档那一份。`confidence` 只决定界面**以哪个为主**展示，
/// 不决定哪个字段能填。
///
/// （早期版本是 `if 实测 { 填实测; continue; }`，那会把已经拿到的出现时刻
/// 白白扔掉，于是甘特图上这一项连位置都没有。）
pub fn attribute(items: &mut [StartupItem], timeline: &BootTimeline, snapshot: &Snapshot) {
    let exclusive = exclusive_exe_owners(items);

    for item in items.iter_mut() {
        // `impact` 与时间轴无关，属于另一条轴，不能被下面的 `ItemTiming::default()`
        // 抹掉。这里显式保存/恢复，让两条归因的**调用顺序不影响结果**——
        // 否则将来谁调换一下顺序，影响数据就静默消失了。
        let impact = item.timing.impact.take();

        let measured = find_slow_entry(item, &timeline.slow_services);
        let observed = observe(item, snapshot, &exclusive);

        match measured {
            // 系统真的测过这一项的耗时 —— 耗时是用户最想知道的量，以它为主。
            Some(slow) => {
                apply_measured(item, slow);
                if let Some(proc) = observed {
                    // 顺手把出现时刻也填上：这一项在时间轴上就有了真实位置。
                    fill_observed(item, proc, snapshot);
                }
            }
            // 没有耗时记录，但内核记了它在什么时候出现。
            None => match observed {
                Some(proc) => {
                    item.timing = ItemTiming {
                        confidence: Confidence::Observed,
                        ..Default::default()
                    };
                    fill_observed(item, proc, snapshot);
                }
                // 两样都没有：只能给"它属于哪个相位"。
                None => apply_phase_estimate(item, timeline),
            },
        }

        item.timing.impact = impact;
    }
}

/// 把 Windows 自记的「启动影响」归因到启动项，返回对上几项。
///
/// ## 为什么它和 `attribute` 分开
///
/// 数据来源、可用条件、失败方式三者都不同：`attribute` 依赖事件日志与进程快照，
/// **不需要提权**；这一条依赖 WDI 文件，**需要提权**，且只覆盖登录窗口。
/// 合在一起的话，提权失败会把另一条通路也拖成"无数据"。
///
/// ## 匹配规则
///
/// 沿用同一条硬规则：**该 exe 只能被一个启动项声明**，否则放弃。
/// 但这里的"多条记录"要区别对待——同一个 exe 在窗口里出现多次是**正常的**
/// （一个程序起了两个进程），那不是歧义，是同一个项的开销，应当**合计**。
/// 歧义只发生在"多个启动项声明同一个 exe"的时候。
pub fn attribute_impact(
    items: &mut [StartupItem],
    report: &StartupInfoReport,
    overview: &mut ImpactOverview,
) {
    overview.record_count = report.records.len();
    overview.matched_count = 0;

    if report.records.is_empty() {
        return;
    }

    // 按 exe 归拢记录：同一 exe 的多条记录是同一个项在不同进程上的开销。
    let mut by_exe: HashMap<&str, Vec<&StartupInfoRecord>> = HashMap::new();
    for rec in &report.records {
        by_exe.entry(rec.exe.as_str()).or_default().push(rec);
    }

    let exclusive = exclusive_exe_owners(items);
    let mut matched = 0usize;

    for item in items.iter_mut() {
        let Some(key) = exe_key(&item.resolved_path) else {
            continue;
        };
        if !exclusive.contains_key(&key) {
            continue;
        }
        let Some(recs) = by_exe.get(key.as_str()) else {
            continue;
        };

        let impact = aggregate(recs);
        // 影响是**独立的一栏**，不参与 `confidence` 的选档：
        // `confidence` 说的是"时间轴上以哪个为准"，而影响说的是"有多重"。
        // 一个项可以时间轴上是估算档、同时有确凿的实测影响。
        if let Some(d) = high_impact_diagnostic(&impact, &item.resolved_path) {
            // 同一项重复扫描时不要堆积重复诊断。
            if !item.diagnostics.iter().any(|x| x.code == d.code) {
                item.diagnostics.push(d);
            }
        }
        item.timing.impact = Some(impact);
        matched += 1;
    }

    overview.matched_count = matched;
}

/// 把一个 exe 的多条进程记录合成一条影响。
///
/// CPU 与磁盘**求和**：同一个程序起两个进程时，它对开机的总占用就是两者之和，
/// 取最大值会低估、取第一条会漏掉。启动时刻取**最早**的一条——
/// "它从第几秒开始出现"。
fn aggregate(recs: &[&StartupInfoRecord]) -> ItemImpact {
    let cpu_ms = recs.iter().map(|r| r.cpu_ms).sum();
    let disk_bytes = recs.iter().map(|r| r.disk_bytes).sum();
    let started_in_trace_ms = recs.iter().filter_map(|r| r.started_in_trace_ms).min();

    ItemImpact {
        cpu_ms,
        disk_bytes,
        level: level_of(cpu_ms, disk_bytes),
        started_in_trace_ms,
        process_count: recs.len(),
    }
}

/// 高档影响给一条诊断。中低档不给——阈值已经是微软筛过的，
/// 我们再叠一层判断只会让两边口径不一致。
fn high_impact_diagnostic(impact: &ItemImpact, path: &str) -> Option<DiagnosticInfo> {
    if impact.level != crate::model::ImpactLevel::High {
        return None;
    }

    let mb = impact.disk_bytes as f64 / 1_048_576.0;
    // 措辞刻意点明"这是任务管理器的口径"：用户能自己复核，
    // 而复核得到一样的结果，正是这个软件值得信的地方。
    Some(DiagnosticInfo {
        code: crate::diag::codes::HIGH_STARTUP_IMPACT.to_string(),
        severity: RiskLevel::Medium,
        message: format!(
            "开机登录后这段时间里，它用了 {:.1} 秒 CPU、读了 {:.1} MB 磁盘，\
             属于「高启动影响」——任务管理器里它的启动影响也是「高」。",
            impact.cpu_ms as f64 / 1000.0,
            mb
        ),
        evidence: Some(format!(
            "WDI StartupInfo：CpuUsage={}ms，DiskUsage={}bytes，进程数={}，映像={}",
            impact.cpu_ms, impact.disk_bytes, impact.process_count, path
        )),
    })
}

/// 填入实测的出现时刻与资源累计。**不动 `confidence`**，也不碰 `duration_ms`。
fn fill_observed(item: &mut StartupItem, proc: &ProcSnapshot, snapshot: &Snapshot) {
    item.timing.observed_start_ms = Some(proc.start_offset_ms);
    item.timing.read_bytes = Some(proc.read_bytes);
    item.timing.observed_at_ms = snapshot.captured_at_offset_ms;
}

/// 写入系统实测的耗时，并按"多花的时间"决定是否给一条提示。
fn apply_measured(item: &mut StartupItem, slow: &SlowService) {
    item.timing = ItemTiming {
        confidence: Confidence::Measured,
        duration_ms: Some(slow.duration_ms),
        source_event_id: Some(slow.event_id),
        ..Default::default()
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
}

/// 只有"所属相位已知"时的退路。
///
/// 交给界面的是一个**相位起点**，不是这一项自己的启动时刻：同相位所有项
/// 都会拿到同一个数字。界面必须按估算显示（虚线 + `~` 前缀）。
fn apply_phase_estimate(item: &mut StartupItem, timeline: &BootTimeline) {
    // 没有时间轴还硬给一个起点，等于凭空编一个"它在这一刻启动"。
    if timeline.phases.is_empty() {
        return;
    }
    if item.boot_phase == BootPhase::Unknown {
        return;
    }
    if let Some(span) = timeline.phases.iter().find(|p| p.name == item.boot_phase) {
        item.timing = ItemTiming {
            confidence: Confidence::Estimated,
            start_estimate_ms: Some(span.start_ms),
            source_event_id: Some(100),
            ..Default::default()
        };
    }
}

/// 统计每个可执行文件名被几个启动项声明。
///
/// 只保留**恰好被一个项声明**的那些——这是能安全归因的前提。
/// 见模块头"匹配为什么不能只按名字包含"。
fn exclusive_exe_owners(items: &[StartupItem]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for item in items {
        if let Some(key) = exe_key(&item.resolved_path) {
            *counts.entry(key).or_insert(0) += 1;
        }
    }
    counts.retain(|_, n| *n == 1);
    counts
}

/// 可执行文件名小写（归因用的键）。路径取不到文件名时返回 `None`。
fn exe_key(path: &str) -> Option<String> {
    let base = path.rsplit(['\\', '/']).next()?.trim();
    if base.is_empty() {
        None
    } else {
        Some(base.to_ascii_lowercase())
    }
}

/// 在快照里找这个启动项对应的进程。
///
/// 两道闸门都必须过：这个 exe 只能被这一个项声明（否则无法区分是谁），
/// 且快照里确实有这个进程（否则它这次没被拉起，或已退出）。
fn observe<'a>(
    item: &StartupItem,
    snapshot: &'a Snapshot,
    exclusive: &HashMap<String, usize>,
) -> Option<&'a ProcSnapshot> {
    let key = exe_key(&item.resolved_path)?;
    if !exclusive.contains_key(&key) {
        return None;
    }
    snapshot.procs.iter().find(|p| p.name == key)
}

/// 一条慢启动记录是否对应这个启动项。
///
/// 匹配规则按事件类型的语义分别定，不能一概用"名字包含"——
/// 那会把 `eventlog` 匹配到 `EventLogViewer.exe` 之类的无关项上。
fn find_slow_entry<'a>(item: &StartupItem, slow: &'a [SlowService]) -> Option<&'a SlowService> {
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

/// 从事件里收集慢项记录（供 `timeline::build` 调用）。
pub fn collect_slow(events: &[RawEvent]) -> Vec<SlowService> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::boot_log::RawEvent;
    use crate::model::{BootTimeline, PhaseSpan, SignerInfo, ValidityStatus};

    fn svc_item(name: &str) -> StartupItem {
        item_with(name, SourceKind::Service, r"C:\Windows\System32\svchost.exe")
    }

    fn item_with(name: &str, source: SourceKind, path: &str) -> StartupItem {
        StartupItem {
            id: format!("test-{name}"),
            source,
            identity_key: name.to_string(),
            name: name.to_string(),
            kind: crate::model::ItemKind::Service,
            display_name: None,
            name_from: None,
            summary: None,
            command: path.to_string(),
            resolved_path: path.to_string(),
            args: vec![],
            location: String::new(),
            scope: crate::model::Scope::Machine,
            enabled: true,
            signer: SignerInfo {
                is_signed: false,
                publisher: None,
                is_microsoft: false,
                cert_valid: None,
                is_os_component: false,
            },
            icon_data: None,
            risk: RiskLevel::Safe,
            risk_reasons: vec![],
            diagnostics: vec![],
            boot_phase: BootPhase::Unknown,
            timing: ItemTiming::default(),
            validity: ValidityStatus::Ok,
            validity_detail: None,
            recommendation: None,
            duplicate_of: None,
            raw: serde_json::Value::Null,
            desired: Default::default(),
            snapshot_ref: None,
        }
    }

    fn slow(name: &str, total: u64, degradation: u64) -> SlowService {
        SlowService {
            name: name.to_string(),
            friendly_name: None,
            duration_ms: total,
            degradation_ms: degradation,
            event_id: 103,
        }
    }

    fn snapshot_of(procs: &[(&str, u64, u64)], captured_at: u64) -> Snapshot {
        Snapshot {
            captured_at_offset_ms: Some(captured_at),
            procs: procs
                .iter()
                .map(|(name, start, read)| ProcSnapshot {
                    name: name.to_string(),
                    start_offset_ms: *start,
                    read_bytes: *read,
                    cpu_ms: 0,
                })
                .collect(),
            unavailable_reason: None,
        }
    }

    #[test]
    fn measured_and_observed_coexist() {
        // 两条通路互不相干，一个项完全可能两者都有——
        // 而"开机后 2.1 秒出现、花了 0.3 秒"正是最有用的一条数据。
        // 早期实现用 `if 实测 { ...; continue; }`，会把已经拿到的出现时刻扔掉。
        let mut items = vec![item_with(
            "Defender",
            SourceKind::Service,
            r"C:\ProgramData\Defender\MsMpEng.exe",
        )];
        let tl = BootTimeline {
            slow_services: vec![slow("Defender", 326, 234)],
            ..Default::default()
        };
        let snap = snapshot_of(&[("msmpeng.exe", 2_100, 4_000_000)], 30_000);

        attribute(&mut items, &tl, &snap);

        assert_eq!(items[0].timing.confidence, Confidence::Measured, "以耗时为主");
        assert_eq!(items[0].timing.duration_ms, Some(326));
        assert_eq!(
            items[0].timing.observed_start_ms,
            Some(2_100),
            "出现时刻不该因为已经有耗时就被丢掉"
        );
        assert_eq!(items[0].timing.read_bytes, Some(4_000_000));
    }

    #[test]
    fn measured_without_process_keeps_duration_only() {
        let mut items = vec![svc_item("windefend")];
        let tl = BootTimeline {
            slow_services: vec![slow("windefend", 326, 234)],
            ..Default::default()
        };

        attribute(&mut items, &tl, &snapshot_of(&[], 30_000));

        assert_eq!(items[0].timing.confidence, Confidence::Measured);
        assert_eq!(items[0].timing.duration_ms, Some(326));
        assert_eq!(
            items[0].timing.observed_start_ms, None,
            "没匹配到进程就不该有出现时刻"
        );
    }

    #[test]
    fn measured_is_preferred_as_headline() {
        // 两档都有时，界面以**实测耗时**为主（用户最想知道的是"花了多久"）。
        // 注意：这只影响 `confidence`，不影响字段能不能填——见
        // `measured_and_observed_coexist`。
        let mut items = vec![svc_item("windefend")];
        let tl = BootTimeline {
            slow_services: vec![slow("windefend", 326, 234)],
            ..Default::default()
        };
        let snap = snapshot_of(&[("windefend.exe", 5_000, 1_000)], 60_000);

        attribute(&mut items, &tl, &snap);

        assert_eq!(items[0].timing.confidence, Confidence::Measured);
        assert_eq!(items[0].timing.duration_ms, Some(326));
    }

    #[test]
    fn observation_gives_real_start_offset_not_a_duration() {
        let mut items = vec![item_with(
            "Acme Sync",
            SourceKind::RunUser,
            r"C:\Program Files\Acme\acmesync.exe",
        )];
        let snap = snapshot_of(&[("acmesync.exe", 12_400, 8_388_608)], 45_000);

        attribute(&mut items, &BootTimeline::default(), &snap);

        assert_eq!(items[0].timing.confidence, Confidence::Observed);
        assert_eq!(items[0].timing.observed_start_ms, Some(12_400));
        assert_eq!(items[0].timing.read_bytes, Some(8_388_608));
        assert_eq!(items[0].timing.observed_at_ms, Some(45_000));
        assert_eq!(
            items[0].timing.duration_ms, None,
            "观测到出现时刻 ≠ 知道它花了多久，绝不能填一个时长"
        );
    }

    #[test]
    fn shared_executable_is_not_attributed() {
        // svchost 承载多个服务：拿它的创建时刻当成每个服务的启动时刻是错的。
        let mut items = vec![
            svc_item("windefend"),
            svc_item("eventlog"),
            svc_item("bits"),
        ];
        let snap = snapshot_of(&[("svchost.exe", 3_000, 100)], 60_000);

        attribute(&mut items, &BootTimeline::default(), &snap);

        for it in &items {
            assert_ne!(
                it.timing.confidence,
                Confidence::Observed,
                "共用宿主进程的项不能按进程归因（{}）",
                it.name
            );
        }
    }

    #[test]
    fn exclusive_executable_is_attributed() {
        // 反面对照：同一个 exe 只被一个项声明时，归因成立。
        let mut items = vec![
            item_with("Defender", SourceKind::Service, r"C:\x\MsMpEng.exe"),
            item_with("Acme", SourceKind::RunUser, r"C:\y\acme.exe"),
        ];
        let snap = snapshot_of(&[("msmpeng.exe", 2_100, 999), ("acme.exe", 9_000, 123)], 30_000);

        attribute(&mut items, &BootTimeline::default(), &snap);

        assert_eq!(items[0].timing.observed_start_ms, Some(2_100));
        assert_eq!(items[1].timing.observed_start_ms, Some(9_000));
    }

    #[test]
    fn missing_process_falls_back_to_phase_estimate() {
        // 快照里没有它的进程（本次没跑 / 已退出）→ 不能编时刻，只能退到相位。
        let mut items = vec![item_with(
            "Acme Sync",
            SourceKind::RunUser,
            r"C:\Program Files\Acme\acmesync.exe",
        )];
        items[0].boot_phase = BootPhase::Logon;
        let tl = BootTimeline {
            phases: vec![PhaseSpan {
                name: BootPhase::Logon,
                start_ms: 30_000,
                end_ms: 60_000,
            }],
            ..Default::default()
        };

        attribute(&mut items, &tl, &snapshot_of(&[], 45_000));

        assert_eq!(items[0].timing.confidence, Confidence::Estimated);
        assert_eq!(items[0].timing.start_estimate_ms, Some(30_000));
        assert_eq!(items[0].timing.observed_start_ms, None);
    }

    #[test]
    fn no_data_leaves_timing_none() {
        let mut items = vec![item_with("Acme", SourceKind::RunUser, r"C:\a\a.exe")];
        attribute(&mut items, &BootTimeline::default(), &Snapshot::default());

        assert_eq!(items[0].timing.confidence, Confidence::None);
        assert_eq!(items[0].timing.observed_start_ms, None);
        assert_eq!(items[0].timing.start_estimate_ms, None);
    }

    #[test]
    fn trivial_slowdown_does_not_produce_a_diagnostic() {
        // Event 103 的门槛很低（公开样本里有 326ms 这种）。全都提示出来
        // 会让"慢启动"这个信号贬值，真正严重的那条反而没人看。
        let mut items = vec![svc_item("windefend")];
        let tl = BootTimeline {
            slow_services: vec![slow("windefend", 326, 234)],
            ..Default::default()
        };

        attribute(&mut items, &tl, &Snapshot::default());

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
            slow_services: vec![slow("windefend", 5200, 4800)],
            ..Default::default()
        };

        attribute(&mut items, &tl, &Snapshot::default());

        let d = &items[0].diagnostics[0];
        assert_eq!(d.code, crate::diag::codes::SLOW_START);
        // 证据里要有可复核的原始数值
        assert!(d.evidence.as_deref().unwrap().contains("4800ms"));
    }

    #[test]
    fn empty_name_slow_entry_never_matches() {
        let mut items = vec![svc_item("windefend")];
        let tl = BootTimeline {
            slow_services: vec![slow("   ", 5000, 5000)],
            ..Default::default()
        };

        attribute(&mut items, &tl, &Snapshot::default());

        assert_eq!(items[0].timing.confidence, Confidence::None);
        assert!(items[0].diagnostics.is_empty());
    }

    #[test]
    fn collect_slow_dedupes_case_insensitively() {
        let ev = |id: u32, name: &str, total: &str| RawEvent {
            event_id: id,
            data: [
                ("Name".to_string(), name.to_string()),
                ("TotalTime".to_string(), total.to_string()),
            ]
            .into_iter()
            .collect(),
            time_created: None,
            computer: None,
        };

        let out = collect_slow(&[ev(103, "windefend", "300"), ev(103, "WinDefend", "400")]);

        assert_eq!(out.len(), 1, "同名记录只保留一条");
    }

    #[test]
    fn collect_slow_keeps_record_without_total_time() {
        let ev = RawEvent {
            event_id: 103,
            data: [("Name".to_string(), "windefend".to_string())]
                .into_iter()
                .collect(),
            time_created: None,
            computer: None,
        };

        let out = collect_slow(&[ev]);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].duration_ms, 0, "\"记录了但没给耗时\"本身也有价值");
    }

    /* ───────────────── 慢项匹配：为什么不能一律按名字比 ───────────────── */

    fn slow_ev(name: &str, event_id: u32) -> SlowService {
        SlowService {
            name: name.into(),
            friendly_name: None,
            duration_ms: 0,
            degradation_ms: 0,
            event_id,
        }
    }

    #[test]
    fn service_slow_event_matches_by_service_name() {
        assert!(matches_item(
            &svc_item("windefend"),
            &slow_ev("windefend", 103)
        ));
        // 大小写不该影响匹配
        assert!(matches_item(
            &svc_item("WinDefend"),
            &slow_ev("windefend", 103)
        ));
    }

    #[test]
    fn service_event_does_not_match_by_file_name() {
        // 事件 103 的 Name 是服务名。若按文件名比，
        // 一个叫 "eventlog" 的文件会被误配到同名的其它东西上。
        let mut it = svc_item("MyService");
        it.resolved_path = r"C:\Program Files\eventlog\eventlog.exe".into();

        assert!(
            !matches_item(&it, &slow_ev("eventlog", 103)),
            "服务型事件只该按服务名匹配"
        );
    }

    #[test]
    fn app_event_matches_by_file_name_with_or_without_extension() {
        let mut it = svc_item("Whatever");
        it.source = SourceKind::RunUser;
        it.resolved_path = r"C:\Program Files\App\MsMpEng.exe".into();

        assert!(matches_item(&it, &slow_ev("MsMpEng.exe", 101)));
        assert!(matches_item(&it, &slow_ev("MsMpEng", 101)));
        assert!(!matches_item(&it, &slow_ev("OtherApp.exe", 101)));
    }

    /* ───────────────── 启动影响归因（WDI StartupInfo）───────────────── */

    fn wdi(exe: &str, pid: u32, cpu_ms: u64, disk: u64, start_ms: Option<u64>) -> StartupInfoRecord {
        StartupInfoRecord {
            exe: exe.to_string(),
            image: format!(r"C:\x\{exe}"),
            pid,
            started_in_trace_ms: start_ms,
            cpu_ms,
            disk_bytes: disk,
        }
    }

    fn report(records: Vec<StartupInfoRecord>) -> StartupInfoReport {
        StartupInfoReport {
            window_ms: Some(90_000),
            source_sid: Some("S-1-5-21-1-2-3-1001".into()),
            is_current_user: true,
            source_file: Some("S-1-5-21-1-2-3-1001_StartupInfo3.xml".into()),
            records,
            ..Default::default()
        }
    }

    #[test]
    fn impact_is_attributed_to_exclusive_owner() {
        let mut items = vec![item_with(
            "Acme Sync",
            SourceKind::RunUser,
            r"C:\Program Files\Acme\acmesync.exe",
        )];
        let mut ov = ImpactOverview::default();

        attribute_impact(&mut items, &report(vec![wdi("acmesync.exe", 7001, 1402, 8_388_608, Some(12_400))]), &mut ov);

        let impact = items[0].timing.impact.as_ref().expect("应当归因到唯一声明者");
        assert_eq!(impact.cpu_ms, 1402);
        assert_eq!(impact.disk_bytes, 8_388_608);
        assert_eq!(impact.started_in_trace_ms, Some(12_400));
        assert_eq!(impact.process_count, 1);
        assert_eq!(ov.matched_count, 1);
        assert_eq!(ov.record_count, 1);
    }

    #[test]
    fn multiple_processes_of_one_exe_are_summed_not_picked() {
        // 同一个程序起两个进程时，它对开机的总占用是两者之和。
        // 取最大值会低估，取第一条会漏——两种都会让用户觉得这个数字"不对"。
        let mut items = vec![item_with("Acme", SourceKind::RunUser, r"C:\x\acme.exe")];
        let mut ov = ImpactOverview::default();

        attribute_impact(
            &mut items,
            &report(vec![
                wdi("acme.exe", 100, 400, 1_000_000, Some(9_000)),
                wdi("acme.exe", 200, 700, 2_000_000, Some(4_000)),
            ]),
            &mut ov,
        );

        let impact = items[0].timing.impact.as_ref().unwrap();
        assert_eq!(impact.cpu_ms, 1_100, "两次 CPU 应合计");
        assert_eq!(impact.disk_bytes, 3_000_000);
        assert_eq!(impact.process_count, 2, "界面要能说明这是合计值");
        assert_eq!(
            impact.started_in_trace_ms,
            Some(4_000),
            "出现时刻取最早那条——'它从第几秒开始出现'"
        );
        assert_eq!(ov.matched_count, 1, "两条记录只对应一个启动项");
    }

    #[test]
    fn shared_executable_gets_no_impact() {
        // 三个服务共用一个 exe 时，那份开销属于谁说不清——放弃，不摊派。
        let mut items = vec![
            svc_item("windefend"),
            svc_item("eventlog"),
            svc_item("bits"),
        ];
        let mut ov = ImpactOverview::default();

        attribute_impact(&mut items, &report(vec![wdi("svchost.exe", 900, 5000, 9_000_000, None)]), &mut ov);

        assert!(items.iter().all(|i| i.timing.impact.is_none()));
        assert_eq!(ov.matched_count, 0);
        assert_eq!(
            ov.record_count, 1,
            "记录本身要算进总数，否则界面会把'没匹配上'说成'没读到'"
        );
    }

    #[test]
    fn only_high_impact_produces_a_diagnostic() {
        // 阈值是微软筛过的，我们不再叠一层自己的判断——
        // 否则同一个项在两个软件里的档位会不一致。
        let mut items = vec![
            item_with("Heavy", SourceKind::RunUser, r"C:\x\heavy.exe"),
            item_with("Light", SourceKind::RunUser, r"C:\x\light.exe"),
        ];
        let mut ov = ImpactOverview::default();

        attribute_impact(
            &mut items,
            &report(vec![
                wdi("heavy.exe", 1, 1500, 100, None),
                wdi("light.exe", 2, 320, 500_000, None),
            ]),
            &mut ov,
        );

        assert!(items[0].timing.impact.is_some());
        assert!(items[1].timing.impact.is_some(), "中档也要给数据");
        assert_eq!(items[0].diagnostics.len(), 1, "高档该给一条提示");
        assert_eq!(items[0].diagnostics[0].code, crate::diag::codes::HIGH_STARTUP_IMPACT);
        assert!(
            items[1].diagnostics.is_empty(),
            "中档不该给提示——那会让'高'这个信号贬值"
        );
    }

    #[test]
    fn high_impact_evidence_carries_raw_numbers() {
        let mut items = vec![item_with("Heavy", SourceKind::RunUser, r"C:\x\heavy.exe")];
        let mut ov = ImpactOverview::default();

        attribute_impact(&mut items, &report(vec![wdi("heavy.exe", 1, 1500, 8_388_608, None)]), &mut ov);

        let ev = items[0].diagnostics[0].evidence.as_deref().unwrap();
        assert!(ev.contains("CpuUsage=1500ms"), "证据要能拿去复核：{ev}");
        assert!(ev.contains("DiskUsage=8388608bytes"));
    }

    #[test]
    fn impact_survives_the_timing_rewrite() {
        // `attribute` 内部会整体替换 `item.timing`。如果顺序调换一下，
        // 影响数据就会静默消失——界面少一栏，没有任何报错。
        // 这条测试锁住"两条归因的调用顺序不影响结果"。
        let mut items = vec![item_with(
            "Acme",
            SourceKind::RunUser,
            r"C:\Program Files\Acme\acmesync.exe",
        )];
        let mut ov = ImpactOverview::default();

        // 先写影响，再跑时间轴归因（故意反过来）
        attribute_impact(
            &mut items,
            &report(vec![wdi("acmesync.exe", 1, 900, 4_000_000, Some(12_000))]),
            &mut ov,
        );
        attribute(&mut items, &BootTimeline::default(), &Snapshot::default());

        let impact = items[0].timing.impact.as_ref().expect("影响不该被时间轴归因抹掉");
        assert_eq!(impact.cpu_ms, 900);
    }

    #[test]
    fn no_records_means_no_impact_and_no_match() {
        let mut items = vec![item_with("Acme", SourceKind::RunUser, r"C:\x\acme.exe")];
        let mut ov = ImpactOverview::default();

        attribute_impact(
            &mut items,
            &StartupInfoReport::unavailable("需要管理员权限"),
            &mut ov,
        );

        assert!(items[0].timing.impact.is_none());
        assert_eq!(ov.matched_count, 0);
        assert_eq!(ov.record_count, 0);
        assert!(
            ov.unavailable_reason.is_none(),
            "这份 overview 由调用方从报告生成；这里只验证归因不清空已有原因"
        );
    }

    #[test]
    fn impact_and_timing_axes_coexist() {
        // 一个项可以同时在时间轴上排在很后面（出现时刻晚），又是全场最重的。
        // 合成一个数字就两头都说不清，所以必须是两栏。
        let mut items = vec![item_with(
            "Acme Sync",
            SourceKind::RunUser,
            r"C:\Program Files\Acme\acmesync.exe",
        )];
        let mut ov = ImpactOverview::default();
        let snap = snapshot_of(&[("acmesync.exe", 12_400, 5_000)], 45_000);

        attribute_impact(
            &mut items,
            &report(vec![wdi("acmesync.exe", 1, 1400, 9_000_000, Some(12_400))]),
            &mut ov,
        );
        attribute(&mut items, &BootTimeline::default(), &snap);

        assert_eq!(items[0].timing.confidence, Confidence::Observed);
        assert_eq!(items[0].timing.observed_start_ms, Some(12_400));
        assert_eq!(items[0].timing.impact.as_ref().unwrap().cpu_ms, 1400);
        assert_eq!(
            items[0].timing.duration_ms, None,
            "出现时刻与影响都不等于'耗时'，这一栏仍然必须是空的"
        );
    }
}
