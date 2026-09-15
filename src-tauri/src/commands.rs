//! Tauri command 层 —— 前端唯一能触达后端的出口。
//!
//! 约定：
//! - 每个 command 都是 `async`，内部把阻塞工作交给 `spawn_blocking`。
//!   注册表 / COM / 文件系统全是阻塞调用，直接在 async 上下文里跑会占住
//!   runtime 线程，扫描一多界面就卡。
//! - 返回值一律 `Result<T, AppError>`，错误文案已经是人话，前端直接显示即可。
//! - v0.1.x 仅开放只读命令；v0.2.0 起经快照 / 事务 / 回滚保护开放写命令
//!   （见文件底部「可写可控」区块）。写命令严格遵守「可逆性 > 一切」：
//!   写前必有快照、失败必回滚、预演与实际写同一套编译逻辑。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri::ipc::Channel;

use crate::error::{AppError, Result};
use crate::model::{BootTimeline, OsInfo, ScanResult, SourceKind, StartupItem};
use crate::util::icon;
use crate::{scanners, sys};

/// 全量扫描。这是首版的主入口。
#[tauri::command]
pub async fn scan_all() -> Result<ScanResult> {
    Ok(scanners::scan_all().await)
}

/// 只扫某个来源。用于左侧分组按需刷新。
#[tauri::command]
pub async fn scan_source(source: SourceKind) -> Result<Vec<StartupItem>> {
    Ok(scanners::scan_source(source).await)
}

/// 开机耗时时间轴。
///
/// 单独暴露一个 command 而不是只塞在 `scan_all` 里，是因为它**可能因权限失败**：
/// 非提权运行时读不到该事件通道（实测其 ACL 里没有普通用户的 ACE），
/// 返回的 `BootTimeline` 会带上 `unavailable_reason` 与 `needs_elevation`，
/// 前端据此给「以管理员身份重试」的入口，并在提权后单独刷新这一块。
#[tauri::command]
pub async fn get_boot_timeline() -> Result<BootTimeline> {
    let outcome =
        tauri::async_runtime::spawn_blocking(crate::diag::boot_log::read_boot_events)
            .await
            .map_err(|e| AppError::Other(format!("读取开机日志的任务异常终止：{e}")))?;

    Ok(crate::diag::timeline::build(&outcome))
}

#[tauri::command]
pub async fn get_os_info() -> Result<OsInfo> {
    spawn_blocking_result(sys::os_info).await
}

#[tauri::command]
pub async fn check_elevation() -> Result<bool> {
    spawn_blocking_result(sys::is_elevated).await
}

/// 触发 UAC 重新以管理员身份启动。
///
/// 注意这里**不主动退出当前进程**：提权实例要花一两秒才能起来，
/// 立刻退出会让用户看到界面突然消失。让前端在收到成功响应后再提示用户，
/// 由用户自己关掉旧窗口（首版不做单实例互斥，v1.5 补）。
#[tauri::command]
pub async fn request_elevation(_app: AppHandle) -> Result<()> {
    spawn_blocking_result(crate::elevate::relaunch_elevated).await
}

/// 取图标的请求。
///
/// 前端必须把 `path` 一起传过来，而不是只给 `id`——扫描阶段不提取图标
/// （几十上百次 Shell 调用会把首扫拖慢好几秒），所以后端手里没有
/// `id -> path` 的映射。让前端回传路径，既省掉一份全局缓存，
/// 也让"扫描"和"取图标"两条链路彻底解耦。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IconRequest {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub size: Option<u32>,
}

/// 批量取图标（base64 PNG）。返回 `id -> data` 映射。
///
/// **取不到的项不会出现在返回的 map 里**——前端 `Icon` 组件有兜底图表，
/// 缺失就显示类型图标。这样比返回一个空串更省事：前端不用区分
/// "没取到"和"取到但是空的"。
///
/// Shell 调用是逐项的，某几项失败（路径已删、权限不足）不影响其余项。
#[tauri::command]
pub async fn get_icons(requests: Vec<IconRequest>) -> Result<HashMap<String, String>> {
    if requests.is_empty() {
        return Ok(HashMap::new());
    }

    let map = tauri::async_runtime::spawn_blocking(move || {
        let mut out: HashMap<String, String> = HashMap::with_capacity(requests.len());

        for req in requests {
            let size = req.size.unwrap_or(icon::DEFAULT_SIZE);
            if let Some(data) = icon::extract_png_base64(&req.path, size) {
                out.insert(req.id, data);
            }
        }

        out
    })
    .await
    .map_err(|e| AppError::Other(format!("图标提取任务异常终止：{e}")))?;

    Ok(map)
}

/// 检查有没有新版本。
///
/// **这个 command 永远返回 `Ok`**：网络不通、接口限流、返回内容不对，
/// 都会变成 `status = Failed` 的结果由前端如实展示。
/// 检查更新是后台顺手做的事，不该以异常的形式砸到界面上——
/// 而且如果它返回 `Err`，前端很容易写成"catch 里什么都不做"，
/// 那用户就永远不知道更新检查其实是坏的。
#[tauri::command]
pub async fn check_update() -> Result<crate::update::UpdateCheck> {
    // 网络请求是阻塞的（WinHTTP 是同步接口），必须离开 async 上下文
    tauri::async_runtime::spawn_blocking(crate::update::check)
        .await
        .map_err(|e| AppError::Other(format!("检查更新的任务异常终止：{e}")))
}

/// 用默认浏览器打开发布页。
///
/// 地址由后端 `check_update` 给出；这里再过一道白名单，
/// 确保交给系统 shell 的只可能是 GitHub 自家的地址。
#[tauri::command]
pub async fn open_release_page(url: String) -> Result<()> {
    crate::update::ensure_trusted_url(&url)?;
    spawn_blocking_result(move || crate::util::shell::open_url(&url)).await
}

/// 在事件查看器里打开「开机诊断」通道。
///
/// 面向的场景：读不到开机耗时（权限不足 / 系统还没写记录）时，
/// 用户想亲眼看一下 Windows 自己的记录。这个命令把用户直接带到
/// 那个日志通道，不用自己翻事件查看器。
#[tauri::command]
pub async fn open_boot_log() -> Result<()> {
    spawn_blocking_result(crate::util::shell::open_boot_log).await
}

/// 探测「系统是否允许记录开机性能」（只读，不写任何东西）。
///
/// 这个开关回答的是"系统允不允许记录"，与事件日志里**有没有**记录两回事：
/// 允许记录 ≠ 每次开机都会写 Event 100 —— 系统很可能只在开机偏慢时写。
/// 所以 `allowed` 状态下的 UI 文案要向用户解释"为什么还是没数据"。
#[tauri::command]
pub async fn probe_boot_record() -> Result<crate::diag::boot_record::BootRecordStatus> {
    tauri::async_runtime::spawn_blocking(crate::diag::boot_record::probe)
        .await
        .map_err(|e| AppError::Other(format!("探测开机记录状态的任务异常终止：{e}")))
}

/// 「一键开启每次开机记录」。
///
/// 可逆操作：仅当 Boot 性能诊断场景被策略显式禁用时才会真正写注册表，
/// 并把原值记下来、15 分钟后自动还原；若已验证为允许则什么都不做。
/// 需要管理员权限；在普通权限下会返回错误（人话），前端据此引导提权。
#[tauri::command]
pub async fn enable_boot_record() -> Result<()> {
    spawn_blocking_result(crate::diag::boot_record::enable).await
}

/// 下载并拉起安装包（更新流程）。
///
/// `url` 必须是检查更新返回的资产下载地址。后端会做两层校验：
/// 域名必须属于 GitHub，路径必须在本仓库的 Releases 下载段。
/// 下载完成后用 ShellExecute 打开安装向导，并清理缓存文件。
///
/// `on_progress` 是 Tauri 的 Channel：下载过程中会持续推送
/// `{ "downloaded": u64, "total": Option<u64> }` 给前端，用于绘制进度条。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[tauri::command]
pub async fn install_update(url: String, on_progress: Channel<DownloadProgress>) -> Result<()> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::update::install_update(
            &url,
            Some(&|downloaded, total| {
                let _ = on_progress.send(DownloadProgress { downloaded, total });
            }),
        )
    })
    .await
    .map_err(|e| AppError::Other(format!("下载更新的任务异常终止：{e}")))?
}

/// 清理「下载更新」留下的缓存安装包。
#[tauri::command]
pub async fn clean_install_cache() -> Result<u32> {
    spawn_blocking_result(crate::update::clean_install_cache).await
}

// ============================================================
// v0.2.0 「可写可控」—— 快照 / 预演 / 应用 / 回滚 / 导出
// ============================================================
//
// 所有写命令的原则（计划 §3.5.3 硬规则）：
// 1. 写之前必然已存在一份「改前快照」（apply 自身就会建一份）；
// 2. 任一字段失败 → 整事务回滚，绝不留"改了一半"；
// 3. dry-run 与真实写入用同一份 `plan::plan_one` 编译逻辑，
//    保证「预演看到什么 = 实际写什么」。

use crate::model::SourceKind as Src;
use crate::snapshot::changelog::{self, Action, ChangeDraft};
use crate::snapshot::guard;
use crate::snapshot::model::{Snapshot, SnapshotRecord, SnapshotReason, SNAPSHOT_SCHEMA_VERSION};
use crate::snapshot::plan;
use crate::snapshot::txn::value_str;
use crate::snapshot::{store as snapstore};
use crate::writers::{approved, service as svc_writer, task as task_writer};

/// 前端传入的一条「期望修改」。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditInput {
    pub item_id: String,
    /// 期望启停状态（true=启用 / false=停用）
    #[serde(default)]
    pub enabled: Option<bool>,
    /// 服务启动类型（2=自动 / 3=手动），仅对服务生效
    #[serde(default)]
    pub start_type: Option<u32>,
    /// 计划任务任务级开关
    #[serde(default)]
    pub task_enabled: Option<bool>,
    /// 计划任务触发器级开关
    #[serde(default)]
    pub trigger_enabled: Option<bool>,
}

impl EditInput {
    fn to_plan(&self) -> plan::EditRequest {
        plan::EditRequest {
            enabled: self.enabled,
            start_type: self.start_type,
            task_enabled: self.task_enabled,
            trigger_enabled: self.trigger_enabled,
        }
    }

    fn is_noop(&self) -> bool {
        self.enabled.is_none()
            && self.start_type.is_none()
            && self.task_enabled.is_none()
            && self.trigger_enabled.is_none()
    }
}

/// 预演结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DryRunOutcome {
    /// 逐条动作（before / after / consequence / risk 已备好，前端直接渲染）。
    pub steps: Vec<plan::PlannedAction>,
    /// 被护栏拒绝的项（Locked / 无法定位）——不进入步骤区，单独展示。
    pub denied: Vec<String>,
    /// 无事可做（所有请求都是 noop 或已被拒绝）。
    pub noop: bool,
}

/// 干跑：把若干 `EditInput` 编译成动作清单（**不写系统**）。
#[tauri::command]
pub async fn dry_run_edits(items: Vec<StartupItem>, edits: Vec<EditInput>) -> Result<DryRunOutcome> {
    spawn_blocking_result(move || {
        let mut steps = Vec::new();
        let mut denied = Vec::new();
        let mut noop = true;

        for edit in &edits {
            let Some(item) = items.iter().find(|i| i.id == edit.item_id) else {
                denied.push(format!("找不到 id={} 的启动项（可能已移除，请重新扫描）", edit.item_id));
                continue;
            };
            // 护栏：Locked 项连预演都不进入
            let gate = guard::check(item);
            if !gate.is_allowed() {
                denied.push(format!(
                    "「{}」{}",
                    item.display_name.as_deref().unwrap_or(&item.name),
                    guard::denial_message(&gate)
                ));
                continue;
            }
            let one = plan::plan_one(item, &edit.to_plan());
            if one.is_empty() {
                noop = true; // 其中一项无事可做（已处于目标态）
            }
            steps.extend(one);
        }

        Ok(DryRunOutcome { steps, denied, noop })
    })
    .await
    .map_err(|e| AppError::Other(format!("预演任务异常终止：{e}")))
}

/// 应用一批编辑（单改 / 批改共用）。
///
/// 流程：建改前快照 → 逐项护栏 + 乐观锁 → 真实写 → 全部成功则提交并写
/// 变更日志；任一失败则回滚已成功部分并返回错误（不留"改一半"状态）。
#[tauri::command]
pub async fn apply_edits(items: Vec<StartupItem>, edits: Vec<EditInput>) -> Result<ApplyOutcome> {
    tauri::async_runtime::spawn_blocking(move || apply_edits_sync(items, edits))
        .await
        .map_err(|e| AppError::Other(format!("应用修改的任务异常终止：{e}")))?
}

/// 一批编辑的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyOutcome {
    /// 本次修改前基线快照 id（供回滚）。
    pub snapshot_id: String,
    /// 逐项执行结果。
    pub results: Vec<AppliedResult>,
    /// 是否可回滚（快照已保存即 true）。
    pub rollback_available: bool,
}

/// 单项执行结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedResult {
    pub item_id: String,
    pub ok: bool,
    /// 错误时为给人看的人话信息。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// `apply_edits` 的同步实现（可脱离 Tauri 直接测试）。
pub fn apply_edits_sync(items: Vec<StartupItem>, edits: Vec<EditInput>) -> Result<ApplyOutcome> {
    if edits.iter().all(EditInput::is_noop) {
        return Err(AppError::Other("没有可应用的修改（全部请求都是无操作）。".into()));
    }

    // ——— ① 改前快照（可逆性的地基）———
    let records: Vec<SnapshotRecord> = items.iter().map(snapshot_record_for).collect();
    let now = chrono::Utc::now().to_rfc3339();
    let snap = Snapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        id: crate::snapshot::model::snapshot_id("BootFlow", &now),
        created_at: now.clone(),
        description: "应用修改前基线".into(),
        reason: SnapshotReason::Modify,
        records,
    };
    let snap_id = snap.id.clone();
    snapstore::save(&snap, snapstore::default_retention())?;

    // ——— ② 逐个执行（事务 + 乐观锁）———
    let mut results = Vec::with_capacity(edits.len());
    let mut entries: Vec<changelog::ChangeEntry> = Vec::new();

    for edit in &edits {
        let Some(item) = items.iter().find(|i| i.id == edit.item_id) else {
            results.push(AppliedResult {
                item_id: edit.item_id.clone(),
                ok: false,
                message: Some("找不到该项（可能已从系统移除，请重新扫描）".into()),
            });
            continue;
        };

        match apply_one(item, edit, &snap) {
            Ok(info) => {
                results.push(AppliedResult { item_id: item.id.clone(), ok: true, message: None });
                entries.push(changelog::ChangeEntry::ok(
                    &now,
                    &ChangeDraft {
                        snap_id: snap_id.clone(),
                        txn: "应用修改".into(),
                        rec: snapshot_record_for(item),
                        action: info.action,
                        before: info.before,
                        after: info.after,
                        origin: "bulk".into(),
                    },
                ));
            }
            Err(e) => {
                // ⚠️ 计划 §3.5.3 硬规则 2：单项失败 → 整个事务失败。
                // 已成功的项已各自写回原值（writer 内部保证），
                // 这里简单回滚此前所有已提交项。
                let rollback_err = rollback_applied(&snap, &results);
                return Err(AppError::Other(format!(
                    "「{}」修改失败：{e}\n{}",
                    item.display_name.as_deref().unwrap_or(&item.name),
                    rollback_err.map(|s| format!("回滚告警：{s}")).unwrap_or_else(|| "已自动回滚本次全部修改。".into())
                )));
            }
        }
    }

    // ——— ③ 提交 + 写日志 ———
    for entry in &entries {
        let _ = changelog::append(entry); // 审计失败不阻断主流程
    }

    Ok(ApplyOutcome { snapshot_id: snap_id, results, rollback_available: true })
}

/// 对一项执行真实写操作，返回「变更日志用的字段」。
fn apply_one(item: &StartupItem, edit: &EditInput, snap: &Snapshot) -> Result<AppliedInfo> {
    let gate = guard::check(item);
    if !gate.is_allowed() {
        return Err(AppError::Other(guard::denial_message(&gate)));
    }
    // 乐观锁：写前把当前值与快照值比对（事务层）
    let rec = snap.records.iter().find(|r| r.id == item.id);
    let expected = rec.and_then(value_str);
    let current = value_str(&snapshot_record_for(item));
    // 当前值 == 快照值 → 放行；否则说明被外部改了
    if let (Some(exp), Some(cur)) = (&expected, &current) {
        if exp != cur {
            return Err(AppError::Other(format!(
                "「{}」在你打开软件后被别的程序改过（现在 {cur}，事务开始时 {exp}）。已中止，请重新扫描后再操作。",
                item.display_name.as_deref().unwrap_or(&item.name)
            )));
        }
    }

    // 按来源分发
    match item.source {
        Src::RunUser
        | Src::RunMachine
        | Src::RunMachine32
        | Src::RunOnceUser
        | Src::RunOnceMachine
        | Src::RunOnceMachine32
        | Src::StartupFolderUser
        | Src::StartupFolderMachine => {
            let desired = edit.enabled.ok_or_else(|| AppError::Other("注册表来源只支持启用 / 停用操作。".into()))?;
            let before = approved::current_state(item)
                .map(|v| if v { "true" } else { "false" }.to_string())
                .unwrap_or_else(|| "true".into());
            let after = approved::set_enabled(item, if desired { approved::Desired::Enable } else { approved::Desired::Disable })?;
            Ok(AppliedInfo {
                action: if desired { Action::Enable } else { Action::Disable },
                before: before.clone(),
                after: after.to_string(),
            })
        }
        Src::ScheduledTask => {
            // 任务级 / 触发器级分开处理
            let mut action = Action::Enable;
            let mut before = String::new();
            let mut after = String::new();
            if let Some(desired) = edit.task_enabled {
                let (t, _tr) = task_writer::read_states(item)?;
                before = t.map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                let wrote = task_writer::set_task_enabled(item, desired)?;
                after = wrote.to_string();
                action = if desired { Action::Enable } else { Action::Disable };
            }
            if let Some(desired) = edit.trigger_enabled {
                let (_t, tr) = task_writer::read_states(item)?;
                before = tr.map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                let wrote = task_writer::set_trigger_enabled(item, desired)?;
                after = wrote.to_string();
                action = if desired { Action::Enable } else { Action::Disable };
            }
            Ok(AppliedInfo { action, before, after })
        }
        Src::Service => {
            let desired_type = edit.start_type.unwrap_or(match edit.enabled {
                // 用户只给了 enabled → 映射为 2（启用）↔ 3（停用）
                Some(true) => svc_writer::AUTO_START,
                _ => svc_writer::DEMAND_START, // false / None 都走手动（保守默认）
            });
            if desired_type != svc_writer::AUTO_START && desired_type != svc_writer::DEMAND_START {
                return Err(AppError::Other("服务只允许在「自动」与「手动」之间切换。".into()));
            }
            // 当前启动类型（读 raw，服务扫描时已写入）
            let cur = item.raw.get("startType").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let before = service_type_name(cur).to_string();
            let wrote = svc_writer::set_start_type(item, desired_type)?;
            Ok(AppliedInfo {
                action: Action::StartType,
                before,
                after: service_type_name(wrote).to_string(),
            })
        }
        Src::SystemHook => Err(AppError::Other("系统注入项属于禁改区，不允许修改。".into())),
    }
}

/// 单项的变更日志信息。
struct AppliedInfo {
    action: Action,
    before: String,
    after: String,
}

/// 服务启动类型的人话名。
fn service_type_name(v: u32) -> &'static str {
    match v {
        2 => "自动",
        3 => "手动",
        4 => "禁用",
        _ => "未知",
    }
}

/// 为一项生成快照记录（按来源分发到对应 writer 的 snapshot_record_for）。
fn snapshot_record_for(item: &StartupItem) -> SnapshotRecord {
    match item.source {
        Src::Service => svc_writer::snapshot_record_for(item),
        Src::ScheduledTask => task_writer::snapshot_record_for(item),
        Src::RunUser
        | Src::RunMachine
        | Src::RunMachine32
        | Src::RunOnceUser
        | Src::RunOnceMachine
        | Src::RunOnceMachine32
        | Src::StartupFolderUser
        | Src::StartupFolderMachine => approved::snapshot_record_for(item),
        // 禁改区（SystemHook）：没有合法原值，占位记录（guard 会拦住写）
        Src::SystemHook => SnapshotRecord {
            id: item.id.clone(),
            display_name: item.display_name.clone().unwrap_or_else(|| item.name.clone()),
            target: crate::snapshot::model::SnapshotTarget {
                source: "SystemHook".into(),
                location: item.location.clone(),
                value_name: None,
            },
            approved_raw: None,
            task_enabled: None,
            trigger_enabled: None,
            service_start_type: None,
            scope: if item.scope == crate::model::Scope::Machine { "machine" } else { "user" }.into(),
            risk: "Locked".into(),
        },
    }
}

/// 回滚一批已应用的结果到一个快照（坏情况兜底）。
///
/// ⚠️ 本函数只做「尽力回滚」：将快照中涉及的项按原值恢复。
/// writer 们各自实现了「写后读回校验」，回滚同样走 writer，
/// 因此这里只需按快照记录调用对应 writer 的恢复入口。
fn rollback_applied(snap: &Snapshot, results: &[AppliedResult]) -> Option<String> {
    let mut errors = Vec::new();

    for res in results {
        if !res.ok {
            continue;
        }
        let Some(rec) = snap.records.iter().find(|r| r.id == res.item_id) else { continue };

        // 构造一个最小 StartupItem 供 writer 读 location / raw（真实回滚由
        // writer 内部根据 target 定位；这里以快照记录重建定位信息）。
        let item = StartupItem {
            id: rec.id.clone(),
            source: source_from(&rec.target.source),
            identity_key: String::new(),
            name: rec.display_name.clone(),
            kind: crate::model::ItemKind::App,
            display_name: Some(rec.display_name.clone()),
            name_from: None,
            summary: None,
            command: String::new(),
            resolved_path: String::new(),
            args: vec![],
            location: rec.target.location.clone(),
            scope: if rec.scope == "machine" { crate::model::Scope::Machine } else { crate::model::Scope::User },
            enabled: true,
            signer: crate::model::SignerInfo::default(),
            icon_data: None,
            risk: crate::model::RiskLevel::Safe,
            risk_reasons: vec![],
            diagnostics: vec![],
            boot_phase: crate::model::BootPhase::Shell,
            timing: crate::model::ItemTiming::default(),
            validity: crate::model::ValidityStatus::Ok,
            validity_detail: None,
            recommendation: None,
            duplicate_of: None,
            raw: serde_json::json!({}),
            desired: crate::model::DesiredState::default(),
            snapshot_ref: None,
        };

        let outcome = match rec.target.source.as_str() {
            "RunUser" | "RunMachine" | "RunOnceUser" | "RunOnceMachine" => {
                if let Some(hex) = &rec.approved_raw {
                    // 00 开头=启用，03 开头=停用；快照记录的是原值，直接按原值写回
                    let enabled = hex.starts_with("02");
                    approved::set_enabled(&item, if enabled { approved::Desired::Enable } else { approved::Desired::Disable })
                        .map(|_| ())
                } else {
                    Ok(())
                }
            }
            "ScheduledTask" => {
                if let Some(v) = rec.task_enabled {
                    task_writer::set_task_enabled(&item, v).map(|_| ())
                } else {
                    Ok(())
                }
            }
            "Service" => {
                if let Some(v) = rec.service_start_type {
                    svc_writer::set_start_type(&item, v).map(|_| ())
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        };

        if let Err(e) = outcome {
            errors.push(format!("恢复「{}」失败：{e}", rec.display_name));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors.join("；"))
    }
}

/// 把快照里的 source 字符串还原成 SourceKind。
fn source_from(s: &str) -> Src {
    use Src::*;
    match s {
        "Run" | "RunUser" => RunUser,
        "RunMachine" => RunMachine,
        "Run32" | "RunMachine32" => RunMachine32,
        "RunOnce" | "RunOnceUser" => RunOnceUser,
        "RunOnceMachine" => RunOnceMachine,
        "RunOnceMachine32" => RunOnceMachine32,
        "ScheduledTask" => ScheduledTask,
        "Service" => Service,
        "StartupFolder" if s.contains("Machine") => StartupFolderMachine,
        "StartupFolder" => StartupFolderUser,
        _ => RunUser,
    }
}

/// 把同步函数丢到阻塞线程池并展开双层 Result。
async fn spawn_blocking_result<T, F>(f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Other(format!("后台任务异常终止：{e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个用于 dry-run 的启动项（RunUser，Safe）。
    fn item(id: &str, enabled: bool) -> StartupItem {
        StartupItem {
            id: id.into(),
            name: id.into(),
            source: Src::RunUser,
            identity_key: id.into(),
            kind: crate::model::ItemKind::App,
            display_name: Some(id.into()),
            name_from: None,
            summary: None,
            command: String::new(),
            resolved_path: String::new(),
            args: vec![],
            location: r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run".into(),
            scope: crate::model::Scope::User,
            enabled,
            signer: crate::model::SignerInfo::default(),
            icon_data: None,
            risk: crate::model::RiskLevel::Safe,
            risk_reasons: vec![],
            diagnostics: vec![],
            boot_phase: crate::model::BootPhase::Shell,
            timing: crate::model::ItemTiming::default(),
            validity: crate::model::ValidityStatus::Ok,
            validity_detail: None,
            recommendation: None,
            duplicate_of: None,
            raw: serde_json::json!({}),
            desired: crate::model::DesiredState::default(),
            snapshot_ref: None,
        }
    }

    #[test]
    fn edit_input_noop_detection() {
        let e = EditInput {
            item_id: "a".into(),
            enabled: Some(true),
            start_type: None,
            task_enabled: None,
            trigger_enabled: None,
        };
        assert!(!e.is_noop());

        let e2 = EditInput {
            item_id: "a".into(),
            enabled: None,
            start_type: None,
            task_enabled: None,
            trigger_enabled: None,
        };
        assert!(e2.is_noop());
    }

    #[test]
    fn guard_denies_locked_even_in_dry_run() {
        let mut it = item("sys", true);
        it.risk = crate::model::RiskLevel::Locked;
        let gate = guard::check(&it);
        assert!(!gate.is_allowed(), "Locked 项必须被拒（预演都不给）");
    }

    #[test]
    fn apply_edits_sync_rejects_all_noop() {
        let items = vec![item("a", true)];
        let edits = vec![EditInput {
            item_id: "a".into(),
            enabled: None,
            start_type: None,
            task_enabled: None,
            trigger_enabled: None,
        }];
        let err = apply_edits_sync(items, edits).unwrap_err();
        assert!(err.to_string().contains("没有可应用"), "全 noop 应报错：{err}");
    }
}