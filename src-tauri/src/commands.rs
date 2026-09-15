//! Tauri command 层 —— 前端唯一能触达后端的出口。
//!
//! 约定：
//! - 每个 command 都是 `async`，内部把阻塞工作交给 `spawn_blocking`。
//!   注册表 / COM / 文件系统全是阻塞调用，直接在 async 上下文里跑会占住
//!   runtime 线程，扫描一多界面就卡。
//! - 返回值一律 `Result<T, AppError>`，错误文案已经是人话，前端直接显示即可。
//! - **首版全部是只读操作**。没有任何写系统配置的 command，
//!   这是刻意的——写操作要等 v1.5 有了快照与回滚才开放。

use std::collections::HashMap;

use serde::Deserialize;
use tauri::AppHandle;

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
/// **取不到的项不会出现在返回的 map 里**——前端 `Icon` 组件有兜底图标，
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

/// 下载并拉起安装包（更新流程）。
///
/// `url` 必须是检查更新返回的资产下载地址。后端会做两层校验：
/// 域名必须属于 GitHub，路径必须在本仓库的 Releases 下载段。
/// 下载完成后用 ShellExecute 打开安装向导，并清理缓存文件。
#[tauri::command]
pub async fn install_update(url: String) -> Result<()> {
    spawn_blocking_result(move || crate::update::install_update(&url)).await
}

/// 清理「下载更新」留下的缓存安装包。
#[tauri::command]
pub async fn clean_install_cache() -> Result<u32> {
    spawn_blocking_result(crate::update::clean_install_cache).await
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
