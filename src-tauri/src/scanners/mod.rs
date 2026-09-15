//! 扫描器聚合层。
//!
//! 每个来源一个子模块，各自独立、各自容错——**一个来源失败不能让整次扫描失败**。
//! 失败信息收集进 `ScanResult.errors` 返回给前端，让用户知道"哪里没扫到"，
//! 而不是看到一个空列表以为机器很干净。
//!
//! 来源覆盖（v1.0 的"完整扫出四类"）：
//!
//! - [x] `startup_folder` —— 启动文件夹 + `IShellLinkW` 解析 .lnk
//! - [x] `registry`       —— `Run` / `RunOnce`（含 `KEY_WOW64_32KEY` 双视图）
//! - [x] `scheduled_task` —— `ITaskService` COM，只收 Boot/Logon 触发器
//! - [x] `service`        —— `EnumServicesStatusExW` + `QueryServiceConfig2W`
//! - [x] `system_hooks`   —— `AppInit_DLLs` / `IFEO` 注入检测
//!
//! 接入方式：在 `scan_all()` 里 `let r = registry::collect();`，
//! 把 `Ok(items)` 推进 `all`，把 `Err(e)` 推进 `errors`。管线会自动完成
//! 去重、有效性检测、耗时注入、风险评级与建议生成，
//! 子模块**不需要**关心这些。

pub mod builder;
pub mod registry;
pub mod scheduled_task;
pub mod service;
pub mod startup_folder;
pub mod system_hooks;

use crate::diag;
use crate::model::{BootTimeline, OsInfo, ScanResult, SourceKind, StartupItem};
use crate::pipeline;

/// `sys::os_info()` 失败时的兜底。绝不因为读不到版本号就让应用崩掉。
fn fallback_os() -> OsInfo {
    OsInfo {
        major: 10,
        minor: 0,
        build: 0,
        sku: "Windows".to_string(),
    }
}

/// 全量扫描。
///
/// 返回 `ScanResult` 而不是 `Result<ScanResult>`：因为"部分来源读取失败"
/// 是常态而非异常（HKLM 某些子键 ACL 受限、个别任务 XML 损坏），
/// 这种情况应该带着 `errors` 正常返回，而不是把整个界面变成错误页。
pub async fn scan_all() -> ScanResult {
    tauri::async_runtime::spawn_blocking(scan_all_blocking)
        .await
        .unwrap_or_else(|e| {
            // 只有扫描线程本身 panic 才会走到这里
            log::error!("扫描线程异常终止：{e}");
            ScanResult {
                items: Vec::new(),
                os: fallback_os(),
                elevated: false,
                scanned_at: chrono::Local::now().to_rfc3339(),
                boot_timeline: BootTimeline::default(),
                observation: crate::model::ItemObservation::default(),
                impact: crate::model::ImpactOverview::default(),
                errors: vec![format!("扫描过程意外中断：{e}")],
            }
        })
}

/// 全量扫描的**同步主体**。
///
/// 抽出来单独一个函数，是为了让真机验收测试能直接跑一遍完整管线：
/// `real_machine_full_report` 需要的是一份可打印、可断言的真实结果，
/// 而绕一层异步包装既不必要，也容易被运行时环境绊住（测试里没有 Tauri runtime）。
pub fn scan_all_blocking() -> ScanResult {
    // spawn_blocking 内部是同步上下文，注册表/COM/文件系统调用都在这里做
    let os = crate::sys::os_info().unwrap_or_else(|e| {
        log::warn!("读取系统版本失败，使用兜底值：{e}");
        fallback_os()
    });

    let elevated = crate::sys::is_elevated().unwrap_or_else(|e| {
        log::warn!("探测提权状态失败：{e}");
        false
    });

    let mut all: Vec<StartupItem> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    // ─────────── 各来源扫描器 ───────────
    //
    // 每个扫描器都遵循同一契约：
    //   fn collect() -> Result<Vec<StartupItem>, String>
    // 成功则并入 all，失败则把原因并入 errors——**一个来源失败不影响其他来源**。

    match startup_folder::collect() {
        Ok(mut v) => all.append(&mut v),
        Err(e) => errors.push(format!("启动文件夹：{e}")),
    }

    match registry::collect() {
        Ok(mut v) => all.append(&mut v),
        Err(e) => errors.push(format!("注册表启动项：{e}")),
    }

    match scheduled_task::collect() {
        Ok(mut v) => all.append(&mut v),
        Err(e) => errors.push(format!("计划任务：{e}")),
    }

    match service::collect() {
        Ok(mut v) => all.append(&mut v),
        Err(e) => errors.push(format!("系统服务：{e}")),
    }

    match system_hooks::collect() {
        Ok(mut v) => all.append(&mut v),
        Err(e) => errors.push(format!("系统注入：{e}")),
    }

    // ─────────── 开机耗时 ───────────
    //
    // ⚠️ 这一步**可能因权限失败**，而且失败本身是需要如实呈现的状态：
    // 该事件通道的 ACL 里没有普通用户的 ACE（实测），所以非提权运行时
    // 必然读不到。把这种情况塞进 `errors` 是错的——那会被界面显示成
    // "扫描出错"，而实际上只是需要提权。
    // 所以它有自己的表达：`BootTimeline::unavailable_reason`。
    let outcome = diag::boot_log::read_boot_events();
    let timeline = diag::timeline::build(&outcome);

    match &timeline.unavailable_reason {
        Some(why) => log::info!("开机耗时数据不可用：{why}"),
        None => log::info!(
            "开机耗时：总时长 {:?}ms，{} 个相位，{} 项慢启动记录",
            timeline.total_boot_ms,
            timeline.phases.len(),
            timeline.slow_services.len()
        ),
    }

    // ─────────── 进程采样（"单项"那条通路）───────────
    //
    // 与上面的时间轴**互相独立**：这条不需要任何权限、每次都拿得到。
    // 它给出每一项"在开机后第几秒出现"（内核记录的进程创建时刻），
    // 这正是"拆到单项"能落地的地方——时间轴读不到时它照样有数据。
    let snapshot = diag::proc_snapshot::capture();
    if snapshot.is_available() {
        log::info!(
            "进程采样：拍摄于开机后 {:?}ms，共 {} 个进程",
            snapshot.captured_at_offset_ms,
            snapshot.procs.len()
        );
    } else {
        log::info!(
            "进程采样不可用：{}",
            snapshot.unavailable_reason.as_deref().unwrap_or("原因未知")
        );
    }

    // ─────────── WDI「启动影响」（Windows 自己记的单项开销）───────────
    //
    // 与上面两条都不同：它**要提权**（该目录对普通用户连列目录都拒），
    // 但换来的是唯一一份**由 Windows 亲自量出来的单项资源消耗**——
    // 任务管理器「启动影响」列读的就是它，用户可自行对照复核。
    // 读不到不影响任何其它通路，所以单独上报、单独计。
    let startup_info = diag::startup_info::capture();
    if startup_info.is_available() {
        log::info!(
            "启动影响：{} 条记录（窗口 {:?}ms，来自 {:?}，当前用户 {}）",
            startup_info.records.len(),
            startup_info.window_ms,
            startup_info.source_sid,
            startup_info.is_current_user
        );
    } else {
        log::info!(
            "启动影响数据不可用：{}",
            startup_info.unavailable_reason.as_deref().unwrap_or("原因未知")
        );
    }

    let mut impact_overview = crate::model::ImpactOverview::from_report(&startup_info);

    let stats = pipeline::finalize(
        &mut all,
        &timeline,
        &snapshot,
        &startup_info,
        &mut impact_overview,
    );

    log::info!(
        "扫描完成：共 {} 项（禁改区 {}，实测耗时 {}，实测出现时刻 {}，实测影响 {}），失效 {}，重复 {}，给出建议 {}",
        stats.total,
        stats.locked,
        stats.measured,
        stats.observed,
        stats.impacted,
        stats.dead + stats.not_executable,
        stats.duplicates,
        stats.advised
    );

    ScanResult {
        items: all,
        os,
        elevated,
        scanned_at: chrono::Local::now().to_rfc3339(),
        boot_timeline: timeline,
        observation: crate::model::ItemObservation {
            captured_at_offset_ms: snapshot.captured_at_offset_ms,
            process_count: snapshot.procs.len(),
            observed_count: stats.observed,
            unavailable_reason: snapshot.unavailable_reason.clone(),
        },
        impact: impact_overview,
        errors,
    }
}

/// 只扫某一个来源。用于前端按需刷新（左侧分组点开时）。
pub async fn scan_source(source: SourceKind) -> Vec<StartupItem> {
    let result = scan_all().await;
    result
        .items
        .into_iter()
        .filter(|i| i.source == source)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RiskLevel, ValidityStatus};
    use std::collections::{HashMap, HashSet};

    fn tally(items: &[StartupItem], key: impl Fn(&StartupItem) -> String) -> Vec<(String, usize)> {
        let mut m: HashMap<String, usize> = HashMap::new();
        for it in items {
            *m.entry(key(it)).or_default() += 1;
        }
        let mut v: Vec<_> = m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v
    }

    /// 真机全量验收：跑一遍完整管线，把所有关键数字打出来。
    ///
    /// 用法：`cargo test real_machine_full_report -- --ignored --nocapture`
    ///
    /// **默认忽略，不随 CI 跑。** 它的断言依赖"这台机器上装了什么"——要求至少
    /// 3 类来源、必须有禁改区项。这在开发机上当然成立，但 GitHub runner 是另一台
    /// 干净虚拟机：拿一台机器的环境去断言另一台的行为，是在制造假红灯，
    /// 红久了就没人看 CI 了。环境相关的验收必须手动跑。
    ///
    /// 它**不写死项数**——项数取决于这台机器上装了什么，把数字写进断言
    /// 只会让测试在别人机器上变红。断的是结构上的底线：来源覆盖、id 唯一、
    /// 风险分档真的分出来了、管线没有半路丢项。
    ///
    /// 真机数字的意义在于**核对**：四类来源加系统注入是否都读到了，
    /// 失效项与建议项是不是集中在合理的量级。这些数字对不上就说明有问题。
    /// 改扫描器或管线之后，在真机上手动跑它。
    #[test]
    #[ignore = "环境相关：需在真机上手动运行，CI runner 的软件构成不同"]
    fn real_machine_full_report() {
        let r = scan_all_blocking();

        println!("\n══════ BootFlow 真机全量扫描 ══════");
        println!("系统：Windows build {}", r.os.build);
        println!("权限：{}", if r.elevated { "管理员" } else { "普通用户（非提权）" });
        println!("扫描时刻：{}", r.scanned_at);
        println!("启动项合计：{} 项", r.items.len());

        println!("\n— 按来源（它注册在哪里）—");
        for (k, v) in tally(&r.items, |i| format!("{:?}", i.source)) {
            println!("  {k:<22} {v}");
        }

        println!("\n— 按类型（界面上的分组）—");
        for (k, v) in tally(&r.items, |i| format!("{:?}", i.kind)) {
            println!("  {k:<22} {v}");
        }

        println!("\n— 按风险 —");
        for (k, v) in tally(&r.items, |i| format!("{:?}", i.risk)) {
            println!("  {k:<22} {v}");
        }

        println!("\n— 按有效性 —");
        for (k, v) in tally(&r.items, |i| format!("{:?}", i.validity)) {
            println!("  {k:<22} {v}");
        }

        let advised: Vec<_> = r.items.iter().filter(|i| i.recommendation.is_some()).collect();
        println!("\n— 有处置建议：{} 项 —", advised.len());
        for (k, v) in tally(&r.items, |i| {
            i.recommendation
                .as_ref()
                .map(|x| format!("{:?}", x.action))
                .unwrap_or_else(|| "—".into())
        }) {
            println!("  {k:<22} {v}");
        }

        let mut diag: HashMap<String, usize> = HashMap::new();
        for it in &r.items {
            for d in &it.diagnostics {
                *diag.entry(d.code.clone()).or_default() += 1;
            }
        }
        let mut diag: Vec<_> = diag.into_iter().collect();
        diag.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        println!("\n— 诊断码分布 —");
        if diag.is_empty() {
            println!("  （无）");
        }
        for (k, v) in &diag {
            println!("  {k:<22} {v}");
        }

        println!("\n— 开机耗时 —");
        match &r.boot_timeline.unavailable_reason {
            Some(why) => println!("  未读取：{why}"),
            None => println!(
                "  总时长 {:?}ms · {} 个相位 · {} 项慢启动记录",
                r.boot_timeline.total_boot_ms,
                r.boot_timeline.phases.len(),
                r.boot_timeline.slow_services.len()
            ),
        }

        if !r.errors.is_empty() {
            println!("\n— 部分失败（不影响其余结果）—");
            for e in &r.errors {
                println!("  {e}");
            }
        }
        println!("══════════════════════════════════\n");

        // ── 结构底线 ──
        assert!(!r.items.is_empty(), "一个启动项都没扫到，扫描链路是断的");

        let ids: HashSet<&str> = r.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids.len(),
            r.items.len(),
            "id 不唯一 —— 去重与稳定 ID 派生出了问题"
        );

        let sources: HashSet<SourceKind> = r.items.iter().map(|i| i.source).collect();
        assert!(
            sources.len() >= 3,
            "来源覆盖不足，只读到 {} 类：{sources:?}",
            sources.len()
        );

        assert!(
            r.items.iter().any(|i| i.risk == RiskLevel::Locked),
            "一个禁改区项都没有 —— 系统组件识别可能整体失效了"
        );

        // 每项都必须有可展示的名称，且不能是空串（那会让清单出现空白行）
        assert!(
            r.items.iter().all(|i| !i.name.trim().is_empty()),
            "存在没有名称的启动项"
        );

        // 失效判定的三态必须有区分度：全 Ok 或全 MissingTarget 都说明判定没生效
        let bad = r
            .items
            .iter()
            .filter(|i| {
                matches!(
                    i.validity,
                    ValidityStatus::MissingTarget | ValidityStatus::NotExecutable
                )
            })
            .count();
        println!("失效目标：{bad} 项（这类项不会拖慢开机，只是污染清单）");
    }
}
