//! 扫描后处理管线。
//!
//! 每个扫描器只负责一件事：**把系统里的原始记录读出来**。
//! 它们产出的 `StartupItem` 是不完整的——没有 id、没做有效性检查、
//! 不知道有没有重复、也没给建议。
//!
//! 这层专门补齐这些，顺序不能换：
//!
//! ```text
//! 扫描器产出原始项
//!      ↓ ① 归一化身份键 + 派生稳定 id   （去重与快照的前提）
//!      ↓ ② 统一类型判定                 （依赖签名信息，必须在有效性之前）
//!      ↓ ③ 有效性检测                   （判断目标是否还在）
//!      ↓ ④ 跨来源去重                   （依赖 ③ 的结论：坏项不参与）
//!      ↓ ⑤ 开机开销归因                 （实测耗时 / 进程观测 / 相位估算）
//!      ↓ ⑥ 风险评级                     （读全部诊断，必须在 ⑤ 之后）
//!      ↓ ⑦ 生成处置建议                 （读 ②③④⑥ 的结论，必须在 ⑥ 之后）
//! ```
//!
//! ⚠️ ⑤ 与 ⑥ 的先后不能调换：风险评级里有一条是"命中系统实测的慢启动"，
//! 而那条诊断正是 ⑤ 注入的。放在前面算，所有项都拿不到这条证据。
//!
//! ⚠️ ⑥ 与 ⑦ 的先后也不能调换：`advise` 会读 `risk` 来判断影响面
//! （见 `advise::needs_attention_when_unsigned`）。若建议先生成，
//! 它看到的 `risk` 永远是构造时的默认值 `Safe`，那条判断整体失效——
//! 而且失效得毫无声息，不会有任何测试报错。
//!
//! 把顺序写死在这里而不是散在各扫描器里，是为了避免"改了一个扫描器，
//! 忘了它也要做去重"这类不一致。

use crate::model::{derive_kind, BootTimeline, StartupItem, ValidityStatus};
use crate::{advise, dedupe, diag, valid};

#[derive(Debug, Default, Clone, Copy)]
pub struct PipelineStats {
    pub total: usize,
    /// 目标已不存在的项数
    pub dead: usize,
    pub not_executable: usize,
    pub unreachable: usize,
    pub duplicates: usize,
    /// 给出了建议的项数
    pub advised: usize,
    /// 被评到 Locked（禁改区）的项数
    pub locked: usize,
    /// 拿到了**单项实测耗时**的项数（系统只对判慢的项记录耗时，通常很少）
    pub measured: usize,
    /// 被内核的进程创建时刻观测到的项数（"开机后第几秒出现"）
    pub observed: usize,
    /// 拿到了**Windows 自记的启动影响**的项数（WDI StartupInfo）
    pub impacted: usize,
}

/// 就地补全所有派生字段。
///
/// 参数收 `&mut [StartupItem]` 而不是 `&mut Vec<..>`：本函数只逐项改写字段，
/// 不增删元素，用切片表达更准确。
///
/// `timeline` 由调用方先读好再传进来，而不是在这里读——读事件日志需要
/// 提权、可能失败、有明确的"读不到"语义，这些属于调用方的职责。
/// 管线只负责"拿到时间轴之后该做什么"。
///
/// `snapshot` 是开机时刻的**进程采样**（创建时刻 + 累计读盘/CPU）。
/// 它和 `timeline` 是两条独立通路：时间轴要提权、还常常整条没有数据；
/// 进程采样不需要任何权限、每次开机都拿得到。所以**即使时间轴完全读不到**，
/// 单项的"开机后第几秒出现"依然能算出来——这正是"拆到单项"的落点。
pub fn finalize(
    items: &mut [StartupItem],
    timeline: &BootTimeline,
    snapshot: &crate::diag::proc_snapshot::Snapshot,
    startup_info: &crate::diag::startup_info::StartupInfoReport,
    impact_overview: &mut crate::model::ImpactOverview,
) -> PipelineStats {
    let mut stats = PipelineStats {
        total: items.len(),
        ..Default::default()
    };

    // ① 身份归一化 —— 必须在去重之前完成
    //
    // ⚠️ id 的种子必须带上「来源内区分符」。只用「来源 + 身份键」的话，
    // 同一命令行承载的多个服务（一个 `svchost.exe -k netsvcs` 就是十几个）
    // 会拿到同一个 id —— 本机实测 273 项里只有 157 个唯一 id。
    // 那不是显示问题：列表 key 冲突、勾选串味、详情面板张冠李戴。
    // 详见 `dedupe::discriminator_of` 的注释。
    for it in items.iter_mut() {
        it.identity_key = dedupe::identity_key(&it.resolved_path, &it.args);
        let did = dedupe::stable_id(
            source_key(it.source),
            &it.identity_key,
            &dedupe::discriminator_of(it),
        );
        it.id = did;
    }

    // ② 统一类型判定
    //    扫描器会先给一个初判，这里**一律覆盖**：判定链只应该有一处实现，
    //    否则四个扫描器各写一套，迟早出现同一类东西被归到不同分组。
    for it in items.iter_mut() {
        it.kind = derive_kind(it.source, &it.signer);
    }

    // ③ 有效性检测
    for it in items.iter_mut() {
        let check = valid::check(&it.resolved_path, it.source);

        // 扫描器可能已经给出更具体的判定（例如它知道这是"停用残留"），
        // 那种信息比我们在这里猜的准，不覆盖
        if it.validity == ValidityStatus::Ok || it.validity == ValidityStatus::Unknown {
            it.validity = check.status;
        }

        if it.validity_detail.is_none() {
            it.validity_detail = check.detail.clone();
        }

        // 引号问题不改变有效性结论，但值得让用户看到——
        // 它是"程序明明有却没启动"这类疑难杂症的常见原因
        if it.validity == ValidityStatus::Ok {
            if let Some(note) = valid::check_command_quotes(&it.command) {
                it.validity_detail = Some(note);
            }
        }

        match it.validity {
            ValidityStatus::MissingTarget => stats.dead += 1,
            ValidityStatus::NotExecutable => stats.not_executable += 1,
            ValidityStatus::Unreachable => stats.unreachable += 1,
            _ => {}
        }
    }

    // ④ 跨来源去重（内部会跳过坏项）
    stats.duplicates = dedupe::mark_duplicates(items);

    // ⑤ 开机开销归因：实测耗时 → 进程观测 → 相位估算，三档如实标注。
    //    这一步只读 source/name/resolved_path/boot_phase，都是扫描器给的原始信息。
    diag::item_cost::attribute(items, timeline, snapshot);

    // ⑤b 「启动影响」归因 —— 独立一条轴，必须排在 ⑤ 之后
    //
    // 排后面的原因：⑤ 内部会整体替换 `item.timing`（`ItemTiming { ..default() }`），
    // 先写的 impact 会被抹掉。（`attribute` 里也做了保存/恢复，两道保险。）
    // 走的是 WDI 那份数据，失败方式与 ⑤ 互不相干，所以单独计数、单独上报。
    diag::item_cost::attribute_impact(items, startup_info, impact_overview);

    // ⑥ 风险评级 —— 读 ⑤ 注入的诊断，所以必须在它之后
    for it in items.iter_mut() {
        let (level, reasons) = diag::risk::assess(it);
        // 编译期防线：SystemHook（AppInit_DLLs / IFEO 注入）无论评估结果如何，
        // 一律顶格 Locked —— 这类来源在类型层就禁止任何写入口（写护栏 guard.rs 兜底）。
        it.risk = crate::snapshot::guard::mark_locked(it.source, level);
        it.risk_reasons = reasons;
        if level == crate::model::RiskLevel::Locked {
            stats.locked += 1;
        }
        match it.timing.confidence {
            crate::model::Confidence::Measured => stats.measured += 1,
            crate::model::Confidence::Observed => stats.observed += 1,
            _ => {}
        }
        if it.timing.impact.is_some() {
            stats.impacted += 1;
        }
    }

    // ⑦ 建议 —— 必须在最后，因为它要读 ②③④⑥ 的结论
    for it in items.iter_mut() {
        it.recommendation = advise::advise(it);
        if it.recommendation.is_some() {
            stats.advised += 1;
        }
    }

    stats
}

/// 来源的稳定字符串表示。
///
/// 用它而不是 `format!("{:?}")`，是因为 Debug 输出属于实现细节，
/// 万一将来改了枚举名，所有 id 都会跟着变、快照历史全部失效。
fn source_key(s: crate::model::SourceKind) -> &'static str {
    use crate::model::SourceKind::*;
    match s {
        StartupFolderUser => "sf-user",
        StartupFolderMachine => "sf-machine",
        RunUser => "run-user",
        RunMachine => "run-machine",
        RunMachine32 => "run-machine32",
        RunOnceUser => "runonce-user",
        RunOnceMachine => "runonce-machine",
        RunOnceMachine32 => "runonce-machine32",
        ScheduledTask => "task",
        Service => "svc",
        SystemHook => "hook",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::proc_snapshot::Snapshot;
    use crate::model::*;

    fn item(name: &str, path: &str) -> StartupItem {
        StartupItem {
            id: String::new(),
            source: SourceKind::RunUser,
            identity_key: String::new(),
            name: name.into(),
            kind: ItemKind::App,
            display_name: None,
            name_from: None,
            summary: None,
            command: path.into(),
            resolved_path: path.into(),
            args: vec![],
            location: "HKCU\\...\\Run".into(),
            scope: Scope::User,
            enabled: true,
            signer: SignerInfo::default(),
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

    #[test]
    fn finalize_fills_ids_and_marks_duplicates() {
        // 同一个不存在的程序注册了两次 —— 目标缺失会先被识别出来
        let dead = "D:\\__bootflow_nonexistent__\\x.exe";
        let mut items = vec![item("a", dead), item("b", dead)];

        let stats = finalize(
            &mut items,
            &BootTimeline::default(),
            &Snapshot::default(),
            &crate::diag::startup_info::StartupInfoReport::default(),
            &mut crate::model::ImpactOverview::default(),
        );

        assert_eq!(stats.total, 2);
        assert!(items.iter().all(|i| !i.id.is_empty()), "id 应被派生");
        // 目标不存在 → 先去重阶段被排除，因此不算重复，但两条都应是坏项
        assert_eq!(stats.dead, 2, "目标不存在的项应全部被标记");
        assert!(items
            .iter()
            .all(|i| i.validity == ValidityStatus::MissingTarget));
    }

    #[test]
    fn finalize_is_stable_across_runs() {
        let dead = "D:\\__bootflow_nonexistent__\\y.exe";
        let mut a = vec![item("a", dead)];
        let mut b = vec![item("a", dead)];
        finalize(
            &mut a,
            &BootTimeline::default(),
            &Snapshot::default(),
            &crate::diag::startup_info::StartupInfoReport::default(),
            &mut crate::model::ImpactOverview::default(),
        );
        finalize(
            &mut b,
            &BootTimeline::default(),
            &Snapshot::default(),
            &crate::diag::startup_info::StartupInfoReport::default(),
            &mut crate::model::ImpactOverview::default(),
        );
        assert_eq!(a[0].id, b[0].id, "同一台机器反复扫描，id 必须稳定");
    }

    #[test]
    fn finalize_derives_kind_from_source() {
        let mut items = vec![item("a", "D:\\__bootflow_nonexistent__\\z.exe")];
        items[0].source = SourceKind::Service;
        finalize(
            &mut items,
            &BootTimeline::default(),
            &Snapshot::default(),
            &crate::diag::startup_info::StartupInfoReport::default(),
            &mut crate::model::ImpactOverview::default(),
        );
        assert_eq!(
            items[0].kind,
            ItemKind::Service,
            "类型判定必须由管线统一派生，不受扫描器初判影响"
        );
    }
}
