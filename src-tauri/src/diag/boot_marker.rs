//! 「每次开机自记账」——不依赖 Event 100、与快速启动无关的开机耗时记录。
//!
//! ## 为什么需要它
//!
//! Windows 的 Event 100(开机主事件)只在**完整引导 + 系统检测到开机慢**时才写入:
//! 快速启动、开机较快、或日志被清理过的机器常年没有 Event 100,耗时页只能空白。
//! 「自记账」绕开这个限制:开机时由自启动条目悄悄拉起我们(BootFlow 不弹窗),
//! 用 `GetTickCount64()` 记下"内核从启动到这一瞬间走过的毫秒数",落一条 JSON。
//! 只要系统能开机,这条记录就在——不依赖快速启动开关、不依赖任何系统策略。
//!
//! ## 数据语义(诚实边界)
//!
//! `total_ms` = 本次开机起点 → 登录自启项被拉起这一刻。开机起点优先取系统日志里
//! **本次开机**的那条事件(System 通道，普通用户可读)，它是实测；
//! 读不到时才退回 `GetTickCount64` 推算，此时 `basis` 标为 `tick`，界面按**估算**呈现。
//!
//! **为什么不能直接用 `GetTickCount64`**:它在**快速启动(混合关机)下不会重置**，
//! 只有完整重启才重置。开了快速启动的机器上，`当前时间 − tick` 会指向若干天前
//! 那次完整引导，算出来的"开机耗时"其实是累计运行时长——正是本功能要服务的
//! 那批用户，反而最容易踩中。详见 `boot_log::read_last_boot_start`。
//!
//! 它**不是**"各相位/各启动项的分解"—那不是用户态能拿到的，Event 100 有才显示，
//! 没有就只显示总时长。我们只记账，不编造明细。
//!
//! ## 它是一个**用户可选**的功能，默认关
//!
//! 自记账要在系统里常驻一个自启条目。哪怕它完全无害，"装个工具就悄悄加了开机自启"
//! 本身就是越界——用户没同意，也没地方关。所以它由
//! `crate::settings::Settings::boot_recording` 控制，**默认 false**；
//! 用户在设置里打开才注册，关掉时 `remove_autostart()` 把两个入口一起清掉。
//!
//! 关掉它**不会**让"单项"功能失效：每项"开机后第几秒出现"来自
//! `diag::proc_snapshot`，那个不需要自启条目、也不需要任何权限。
//! 自记账只负责"本次开机总共用了多久"这一条曲线。
//!
//! ## 写入位置与文件
//!
//! `%LOCALAPPDATA%\BootFlow\boot-records.json`,滚动保留最近 N 条,天然可随版本
//! 存活,也便于导出。

use std::fs;
use std::path::PathBuf;

/// 保留最近多少条记录。
const MAX_RECORDS: usize = 50;

/// 自定义 app 数据目录名(不与 Tauri 官方 data dir 混淆)。
///
/// 目录名本身在 `crate::paths` 里定义——不要在这里另写一份字面量。
const RECORDS_FILE: &str = "boot-records.json";

/// 判定「同一次开机」时，两条记录的开机起点允许的最大偏差（秒）。
///
/// 同一会话内 `now - GetTickCount64()` 应恒定，容差只为吸收时钟微调；
/// 取 120 秒既能容忍 NTP 校正，又远小于任何两次真实开机的间隔。
const SAME_SESSION_TOLERANCE_SECS: i64 = 120;

/// 一条自记账记录。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootRecordEntry {
    /// ISO8601 开机起点。
    ///
    /// 优先取自系统日志里**本次开机**的那条事件（实测）；
    /// 系统日志读不到时才退回 `记录时刻 − GetTickCount64`（推算）。
    /// 两者对不上时以系统日志为准——原因见 `capture()`。
    pub boot_started_at: String,
    /// 记录落盘时的墙钟时间（ISO8601）。
    pub recorded_at: String,
    /// 从开机起点到落盘这一刻的毫秒数。就是用户要的「本次开机用了多久」。
    pub total_ms: u64,
    /// 数据来源：恒为 `marker`（自记账）。Event 100 走 timeline，不写这里。
    pub source: String,
    /// 开机起点是怎么来的，取值见 `BASIS_LOG` / `BASIS_TICK`。
    ///
    /// 这一项的用途只有一个：**让界面能如实区分实测与推算**。
    /// 只有 `tick` 这条路在快速启动下会给出错得离谱的数字，
    /// 不标出来就等于拿推算冒充实测。
    #[serde(default = "default_basis")]
    pub basis: String,
}

/// 开机起点由系统日志确认（实测）。
pub const BASIS_LOG: &str = "log";

/// 开机起点由 `GetTickCount64` 推算（系统日志读不到时的退路，视为**估算**）。
pub const BASIS_TICK: &str = "tick";

/// `basis` 缺失时的默认值。
///
/// 默认取 `tick` 而不是 `log`：早期版本只有 tick 一条路，它写下的记录
/// 没有任何日志佐证。把"没标依据的旧记录"当成实测，正是这次要避免的错误。
fn default_basis() -> String {
    BASIS_TICK.to_string()
}

impl BootRecordEntry {
    /// 生成一条当前开机记录。
    fn capture() -> Self {
        let now = chrono::Local::now();

        // 内核视角的"系统启动以来毫秒数"（无权限要求）。
        let tick_ms = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
        let tick_start = now - chrono::Duration::milliseconds(tick_ms as i64);

        // 系统日志里最新一条开机事件。它每次开机都写（含快速启动）。
        let log_start = crate::diag::boot_log::read_last_boot_start()
            .ok()
            .and_then(|iso| chrono::DateTime::parse_from_rfc3339(&iso).ok())
            .map(|t| t.with_timezone(&chrono::Local));

        let (started, basis) = match log_start {
            Some(logged) => {
                // 正常情况下 tick 推出来的起点应当**略早于**日志时刻：
                // 内核先跑起来，日志服务才启动，差几秒到几十秒。
                //
                // 若 tick 起点早出一大截，说明这台机器自上次**完整引导**以来
                // 又用快速启动开了好几次机——快速启动不重置 tick，
                // 于是 `tick_start` 还停在几天前，拿它算出来的"开机耗时"
                // 其实是跨多次开关机的累计运行时长。这种情况必须以日志为准。
                let delta = logged.signed_duration_since(tick_start);
                let consistent = delta >= -chrono::Duration::seconds(60)
                    && delta <= chrono::Duration::minutes(10);

                if consistent {
                    // 两者一致：用 tick 起点，它更贴近"内核开始运转"这一刻，
                    // 比日志事件早的那几秒也是真实开机时间的一部分。
                    (tick_start, BASIS_LOG)
                } else {
                    (logged, BASIS_LOG)
                }
            }
            // 日志读不到（极少见）：退回 tick 推算，但**标明是推算**。
            None => (tick_start, BASIS_TICK),
        };

        let total_ms = now
            .signed_duration_since(started)
            .num_milliseconds()
            .max(0) as u64;

        Self {
            boot_started_at: started.to_rfc3339(),
            recorded_at: now.to_rfc3339(),
            total_ms,
            source: "marker".to_string(),
            basis: basis.to_string(),
        }
    }
}

/// 存储文件路径。
fn records_path() -> Option<PathBuf> {
    crate::paths::app_data_file(RECORDS_FILE)
}

/// 给前端展示的完整记录列表（最近 50 条，旧 → 新）。
pub fn read_records() -> Vec<BootRecordEntry> {
    let Some(path) = records_path() else { return vec![] };
    let Ok(text) = fs::read_to_string(&path) else {
        return vec![];
    };
    match serde_json::from_str::<Vec<BootRecordEntry>>(&text) {
        Ok(v) => keep_latest(v),
        Err(_) => vec![],
    }
}

/// 按开机起点升序排序，再**丢掉最旧的**，只保留最近 `MAX_RECORDS` 条。
///
/// ⚠️ 必须先升序后 `drain(0..)`（丢队首），不能"升序 + `truncate`"——
/// 后者丢的是**最新**的记录，留下最旧的一批，恰是自记账最不该丢的数据。
fn keep_latest(mut v: Vec<BootRecordEntry>) -> Vec<BootRecordEntry> {
    sort_by_start_asc(&mut v);
    if v.len() > MAX_RECORDS {
        let drop = v.len() - MAX_RECORDS;
        v.drain(0..drop);
    }
    v
}

/// 按开机起点升序。优先按**解析出的时间点**比较，而不是 ISO8601 字符串——
/// 字符串比较在夏令时/时区变更导致 offset 不同（`+08:00` vs `+09:00`）时会排错。
/// 解析失败时退回字符串比较，保证仍有确定的全序。
fn sort_by_start_asc(v: &mut [BootRecordEntry]) {
    v.sort_by(|a, b| {
        match (
            parse_rfc3339(&a.boot_started_at),
            parse_rfc3339(&b.boot_started_at),
        ) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => a.boot_started_at.cmp(&b.boot_started_at),
        }
    });
}

/// 写一条新记录(追加,裁剪到 MAX_RECORDS)。
///
/// ## 同一次开机只保留「最早被观测到」的那条
///
/// `total_ms` 的语义是「记录那一刻的已开机时长」。正常路径是开机自启跑一次,
/// 此时它 ≈ 开机耗时。但**手动重跑/多次启动会把同一次开机记成 75 分钟的运行时长**——
/// 那不是开机耗时。所以同一开机会话(开机起点相差 ≤ 容差)内只保留 `total_ms`
/// 最小的那条:越早观测越接近真实开机,重复写入是幂等的。
pub fn append_record(entry: BootRecordEntry) -> std::io::Result<()> {
    let mut all = read_records();

    if let Some(slot) = all.iter_mut().find(|e| same_boot_session(e, &entry)) {
        if entry.total_ms < slot.total_ms {
            *slot = entry;
        }
        // 否则丢弃本次：已有更早、更接近真实开机的观测，重复写入无意义。
    } else {
        all.push(entry);
    }

    // 先裁剪再落盘：文件本身也不该无限增长。
    let all = keep_latest(all);

    let Some(path) = records_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "找不到 LOCALAPPDATA",
        ));
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(
        path,
        serde_json::to_string_pretty(&all).unwrap_or_else(|_| "[]".to_string()),
    )
}

/// 两条记录是否属于同一次开机。
///
/// 判据是「开机起点时刻相近」而非精确相等:`boot_started_at` 由
/// `now - GetTickCount64()` 推出,同一会话内该值恒定,只受墙钟微调/浮点格式化影响,
/// 因此给一个宽松容差即可。用容差而非精确比较,是为了在时钟被 NTP 校正、
/// 或两次调用跨越格式化精度边界时仍然合并。
fn same_boot_session(a: &BootRecordEntry, b: &BootRecordEntry) -> bool {
    match (parse_rfc3339(&a.boot_started_at), parse_rfc3339(&b.boot_started_at)) {
        (Some(x), Some(y)) => (x - y).num_seconds().abs() <= SAME_SESSION_TOLERANCE_SECS,
        // 任一时间解析失败时不合并：宁可多留一条，也不误删用户的真实历史。
        _ => false,
    }
}

fn parse_rfc3339(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(s).ok()
}

/// 自启执行的入口：追加一条记录，立即返回。失败（写不了）不 panic、不弹窗。
pub fn record_once() -> Result<(), String> {
    append_record(BootRecordEntry::capture()).map_err(|e| e.to_string())
}

/// 「开机自记账」计划任务的路径（任务计划程序根文件夹下）。
const TASK_PATH: &str = r"\BootFlowBootMarker";

/// 旧版本用过的 `HKCU\...\Run` 值名（非提权路径仍用它，故不再叫 "legacy"）。
const RUN_VALUE_NAME: &str = "BootFlowBootMarker";

/// 自启入口的决策结果。纯数据，不碰注册表 / 任务库。
///
/// 单独抽出来是因为「选哪个入口」是这段逻辑里唯一容易错的地方——
/// 选错就会两个入口并存，于是每次登录把程序启动两次；而它完全可以
/// 脱离 Windows 单测。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutostartPlan {
    /// 计划任务已经在：保持它，顺手清掉 Run 值。
    KeepTask,
    /// 注册登录计划任务（进程已提权时）。
    RegisterTask,
    /// 写 `HKCU\...\Run`（提不了权时的唯一可行路径）。
    WriteRunKey,
}

/// 按「任务是否存在 + 当前是否提权」决定用哪个入口。
///
/// **任务只要存在就一律让路**：两个入口并存会让每次登录启动两次。
fn plan_autostart(task_exists: bool, elevated: bool) -> AutostartPlan {
    if task_exists {
        AutostartPlan::KeepTask
    } else if elevated {
        AutostartPlan::RegisterTask
    } else {
        AutostartPlan::WriteRunKey
    }
}

/// 注册开机自启（幂等）。**同一时刻只让一个入口生效**。
///
/// ## 两个入口为什么必须互斥
///
/// 计划任务和 `HKCU\...\Run` 都能在登录时拉起本程序。两个同时存在，每次登录
/// 就会**启动两次**——用户会看到两个进程。所以这里始终保证只留一个。
///
/// ## 选择顺序
///
/// 1. **已经有计划任务** → 就用它，并清掉 Run 值。任务入口更稳，
///    不受「企业策略隐藏 Run 键」影响。
/// 2. **没有任务、且当前进程已提权** → 注册一个 `LeastPrivilege` 的登录任务，
///    再清掉 Run 值。微软文档明确：任务的运行级别由 `RunLevel` 属性决定，
///    **不由 exe 的 manifest 决定**，所以登录时它静默运行、绝不弹 UAC。
///    这是 Windows 上「免 UAC 静默自启」的正规做法。
/// 3. **其余情况** → 写 `HKCU\...\Run`。非提权进程注册任务会被系统拒绝
///    （实测 `0x80070005`），所以这是唯一可行的路；而当前用户本来就提不了权，
///    登录拉起也不会弹 UAC。
///
/// 发布版 manifest 是 `asInvoker`（见 `build.rs`），所以第 3 条才是绝大多数
/// 用户的实际路径——双击运行即可，不需要管理员权限，也不会弹 UAC。
pub fn ensure_autostart() -> Result<(), String> {
    let current_exe =
        std::env::current_exe().map_err(|e| format!("无法定位自身路径：{e}"))?;
    let Some(exe_str) = current_exe.to_str() else {
        return Err("自身路径非 UTF-8，无法注册自启".to_string());
    };

    #[cfg(windows)]
    let plan = plan_autostart(
        task_impl::exists(TASK_PATH),
        crate::sys::is_elevated().unwrap_or(false),
    );
    #[cfg(not(windows))]
    let plan = AutostartPlan::WriteRunKey;

    match plan {
        // 任务入口已在：清掉 Run 值，否则两个入口都会在登录时拉起、启动两次。
        AutostartPlan::KeepTask => {
            remove_run_entry();
            Ok(())
        }
        AutostartPlan::RegisterTask => {
            #[cfg(windows)]
            {
                task_impl::register(TASK_PATH, &task_xml(exe_str), exe_str)?;
                // 任务建好了才清旧值：迁移过程中不能出现"两边都没有"的空档。
                remove_run_entry();
                Ok(())
            }
            #[cfg(not(windows))]
            {
                write_run_entry(exe_str)
            }
        }
        AutostartPlan::WriteRunKey => write_run_entry(exe_str),
    }
}

/// 拆掉自记账的**全部**自启入口（任务 + `HKCU\Run` 值）。幂等。
///
/// 用户关掉「每次开机自记账」时调它。必须两个都清：这个功能历史上先后用过
/// 两条路（早期只有 Run 键，后来加了计划任务），只清一条会让关掉之后
/// 依然每次开机被拉起——用户看到"明明关了还在"，就再也不会信这个开关。
///
/// 清理失败不返回错误：调用方（设置页）需要的是"尽力清干净"，
/// 而不是因为一个删不掉的任务把开关回滚掉——那会让用户陷在"关不掉"里。
pub fn remove_autostart() -> Result<(), String> {
    let mut first_err: Option<String> = None;

    #[cfg(windows)]
    {
        // 任务不存在时 `delete` 直接 Ok，所以这里不必先 `exists`——
        // 也避免了 exists/delete 之间的竞态。
        if let Err(e) = task_impl::delete(TASK_PATH) {
            first_err = Some(e);
        }
    }

    remove_run_entry();

    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// 自记账任务的任务定义 XML。
///
/// 几个刻意的取值：
/// - `RunLevel=LeastPrivilege`：**这行是整件事的关键**，见 `ensure_autostart`。
/// - `LogonType=InteractiveToken`：随用户登录运行；记账要写到该用户的
///   `%LOCALAPPDATA%`，用交互令牌才拿得到正确的用户上下文。
/// - `DisallowStartIfOnBatteries=false`：笔记本不接电源时也得记，
///   否则"最近几次开机"会莫名其妙缺几条。
/// - `ExecutionTimeLimit=PT2M`：它只写一个小 JSON，给个上限防止万一卡住。
/// - `StartWhenAvailable=true`：错过触发点（比如登录时机器繁忙）也补跑一次。
fn task_xml(exe_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>BootFlow 开机自记账：登录时记一条本次开机耗时。只写自己的数据文件，不需要管理员权限。</Description>
    <URI>{task_path}</URI>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT2M</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>--mark-boot</Arguments>
    </Exec>
  </Actions>
</Task>"#,
        task_path = TASK_PATH,
        exe = escape_xml(exe_path),
    )
}

/// XML 文本转义。
///
/// 路径里出现 `&` 是常事（`C:\Tools\A & B\app.exe`），不转义会让整个
/// 任务 XML 解析失败、注册整个失败。**顺序要紧**：`&` 必须最先替换，
/// 否则后面替换出来的实体又会被再转义一遍。
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// 写入 `HKCU\...\Run` 自启项（非提权路径）。
///
/// 值形如 `"C:\...\bootflow.exe" --mark-boot`。路径带空格时必须加引号，
/// 否则 `Run` 的解析会把它当成多个 token。
fn write_run_entry(exe_str: &str) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    let command = format!("\"{exe_str}\" --mark-boot");

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = hkcu
        .open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            KEY_READ | KEY_WRITE,
        )
        .map_err(|e| format!("无法打开 HKCU Run 键：{e}"))?;

    // 已存在且指向同一命令 → 幂等返回。
    if let Ok(existing) = run_key.get_value::<String, _>(RUN_VALUE_NAME) {
        if existing == command {
            return Ok(());
        }
    }

    run_key
        .set_value(RUN_VALUE_NAME, &command)
        .map_err(|e| format!("写入 HKCU Run 失败：{e}"))
}

/// 删掉 `HKCU\Run` 里我们自己的值（迁移到计划任务后清理，以及非提权路径不再需要它时）。
///
/// 只删自己的值名，且要求值里带 `--mark-boot` 才动手——
/// 万一将来有别的程序用了同名值，也不能误删。
/// 删不掉只记日志：清理失败不该阻断已经成功的注册。
fn remove_run_entry() {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(run_key) = hkcu.open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        KEY_READ | KEY_WRITE,
    ) else {
        return;
    };

    if let Ok(existing) = run_key.get_value::<String, _>(RUN_VALUE_NAME) {
        if existing.contains("--mark-boot") {
            if let Err(e) = run_key.delete_value(RUN_VALUE_NAME) {
                log::warn!("清理旧的 Run 自启项失败：{e}");
            }
        }
    }
}

#[cfg(windows)]
mod task_impl {
    use crate::util::com::ComGuard;
    use windows::core::{BSTR, VARIANT};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows::Win32::System::TaskScheduler::{
        ITaskService, TASK_CREATE_OR_UPDATE, TASK_LOGON_INTERACTIVE_TOKEN, TaskScheduler,
    };

    /// 查询自记账任务是否已存在（**只读**，不注册、不修改）。
    ///
    /// 非提权进程也能读任务库，所以两条路径上都用它来避免「两个入口同时生效」。
    /// 读不到（COM 失败、任务库不可访问）时一律返回 `false`：
    /// 判据保守一点，最坏是退回 Run 键，而不是漏注册导致完全不自启。
    pub fn exists(task_path: &str) -> bool {
        let _com = ComGuard::new();

        let service: ITaskService = match unsafe {
            CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
        } {
            Ok(s) => s,
            Err(_) => return false,
        };

        let empty = VARIANT::default();
        if unsafe { service.Connect(&empty, &empty, &empty, &empty) }.is_err() {
            return false;
        }

        let Ok(folder) = (unsafe { service.GetFolder(&BSTR::from("\\")) }) else {
            return false;
        };

        unsafe { folder.GetTask(&BSTR::from(task_path)) }.is_ok()
    }

    /// 删除自记账任务。任务不存在时视为成功（幂等）。
    pub fn delete(task_path: &str) -> Result<(), String> {
        let _com = ComGuard::new();

        let service: ITaskService =
            unsafe { CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| format!("连接任务计划程序失败：{e}"))?;

        let empty = VARIANT::default();
        unsafe { service.Connect(&empty, &empty, &empty, &empty) }
            .map_err(|e| format!("初始化任务计划程序失败：{e}"))?;

        let folder = unsafe { service.GetFolder(&BSTR::from("\\")) }
            .map_err(|e| format!("打开任务根文件夹失败：{e}"))?;

        // 删除前先确认确实是我们建的那个任务：只看"任务存在"就删太危险，
        // 万一将来有别的程序用了同名任务，我们会把它的删掉。
        let Ok(existing) = (unsafe { folder.GetTask(&BSTR::from(task_path)) }) else {
            return Ok(());
        };
        let is_ours = unsafe { existing.Xml() }
            .map(|x| x.to_string().contains("--mark-boot"))
            .unwrap_or(false);
        if !is_ours {
            log::warn!("同名任务存在但不含 --mark-boot，判定不是我们的，跳过删除");
            return Ok(());
        }

        unsafe { folder.DeleteTask(&BSTR::from(task_path), 0) }
            .map_err(|e| format!("删除自记账任务失败：{e}"))
    }

    /// 注册（或更新）自记账任务。已存在且指向同一个 exe 时直接返回。
    pub fn register(task_path: &str, xml: &str, exe_path: &str) -> Result<(), String> {
        // COM 按线程初始化，而这个函数可能落在任意线程池线程上。
        let _com = ComGuard::new();

        let service: ITaskService =
            unsafe { CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| format!("连接任务计划程序失败：{e}"))?;

        let empty = VARIANT::default();
        unsafe { service.Connect(&empty, &empty, &empty, &empty) }
            .map_err(|e| format!("初始化任务计划程序失败：{e}"))?;

        let folder = unsafe { service.GetFolder(&BSTR::from("\\")) }
            .map_err(|e| format!("打开任务根文件夹失败：{e}"))?;

        // 幂等：任务已存在、且 XML 里就是我们这个 exe → 什么都不做。
        if let Ok(existing) = unsafe { folder.GetTask(&BSTR::from(task_path)) } {
            if let Ok(current) = unsafe { existing.Xml() } {
                if current.to_string().contains(exe_path) {
                    return Ok(());
                }
            }
        }

        // userId / password / sddl 都留空：文档规定 `TASK_LOGON_INTERACTIVE_TOKEN`
        // 且未提供凭据时，任务以**调用者**的身份登记——正是我们要的。
        unsafe {
            folder.RegisterTask(
                &BSTR::from(task_path),
                &BSTR::from(xml),
                TASK_CREATE_OR_UPDATE.0,
                &VARIANT::default(),
                &VARIANT::default(),
                TASK_LOGON_INTERACTIVE_TOKEN,
                &VARIANT::default(),
            )
        }
        .map_err(|e| format!("注册开机自记账任务失败：{e}"))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 改进程级 `LOCALAPPDATA` 的用例必须串行：环境变量是全局的，
    /// 并行跑会让两条用例互相读到对方的目录，出现"随机"失败。
    /// 这里用 `crate::testenv` 的**全局**锁，而不是模块内自建——
    /// `update::cache_dir` 等地方也读 `LOCALAPPDATA`，各持一把等于没锁。
    fn with_isolated_store<T>(name: &str, f: impl FnOnce() -> T) -> T {
        let _guard = crate::testenv::lock();
        let tmp = std::env::temp_dir().join(format!("bootflow-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        std::env::set_var("LOCALAPPDATA", &tmp);
        let out = f();
        let _ = fs::remove_dir_all(&tmp);
        out
    }

    fn entry(started: &str, total_ms: u64) -> BootRecordEntry {
        BootRecordEntry {
            boot_started_at: started.to_string(),
            recorded_at: started.to_string(),
            total_ms,
            source: "marker".to_string(),
            basis: BASIS_LOG.to_string(),
        }
    }

    #[test]
    fn capture_is_well_formed() {
        let e = BootRecordEntry::capture();
        assert!(e.total_ms > 0, "开机时长应大于 0");
        assert_eq!(e.source, "marker");
        assert!(
            e.basis == BASIS_LOG || e.basis == BASIS_TICK,
            "basis 必须是 log / tick 之一，实际 {}",
            e.basis
        );

        // 开机起点必须**早于**记录时刻，且两者之差恰为 total_ms。
        // 这条断言锁住"减号方向"——写反了会得到未来的开机时间。
        let started = parse_rfc3339(&e.boot_started_at).expect("起点可用 RFC3339 解析");
        let recorded = parse_rfc3339(&e.recorded_at).expect("记录时刻可用 RFC3339 解析");
        assert!(started < recorded, "开机起点应早于记录时刻");
        assert_eq!(
            (recorded - started).num_milliseconds(),
            e.total_ms as i64,
            "两者之差应恰为已开机时长"
        );
    }

    /// 旧版本的记录（没有 `basis` 字段）必须仍能读出来，且被当作 `tick`（推算）。
    ///
    /// 这条不是洁癖：早期版本只有 tick 一条路，若把它们默认为"系统日志确认"，
    /// 界面上就会把可能跨了好几次开关机的数字标成实测——正是要避免的那种谎。
    #[test]
    fn legacy_records_without_basis_deserialize_as_tick() {
        let json = r#"[{
            "bootStartedAt": "2026-05-01T08:00:00+08:00",
            "recordedAt": "2026-05-01T08:00:02+08:00",
            "totalMs": 2100,
            "source": "marker"
        }]"#;
        let v: Vec<BootRecordEntry> = serde_json::from_str(json).expect("旧记录应可解析");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].basis, BASIS_TICK, "缺 basis 的旧记录按推算处理");
        assert_eq!(v[0].total_ms, 2100);
    }

    #[test]
    fn different_sessions_are_kept_separately() {
        with_isolated_store("sessions", || {
            append_record(entry("2026-05-01T08:00:00+08:00", 2100)).unwrap();
            append_record(entry("2026-05-02T09:30:00+08:00", 33000)).unwrap();

            let got = read_records();
            assert_eq!(got.len(), 2);
            assert_eq!(got[0].total_ms, 2100, "按开机起点升序");
            assert_eq!(got[1].total_ms, 33000);
        });
    }

    /// 核心不变量：同一次开机里反复跑自记账，不能把「运行时长」写成「开机耗时」。
    #[test]
    fn same_session_keeps_earliest_observation() {
        with_isolated_store("dedupe", || {
            // 开机自启跑一次：2.1 秒。
            append_record(entry("2026-05-01T08:00:00+08:00", 2100)).unwrap();
            // 用户 5 分钟后手动又点了一次：此刻"已开机时长"是 5 分钟，不是开机耗时。
            append_record(entry("2026-05-01T08:00:00+08:00", 302100)).unwrap();
            // 再跑一次，更长。
            append_record(entry("2026-05-01T08:00:00+08:00", 400_000)).unwrap();

            let got = read_records();
            assert_eq!(got.len(), 1, "同一次开机只应保留一条");
            assert_eq!(got[0].total_ms, 2100, "保留最早的观测（最接近真实开机耗时）");
        });
    }

    /// 容差边界：轻微时钟漂移算同一次，明显间隔算两次。
    #[test]
    fn tolerance_boundary_merges_drift_but_splits_real_reboots() {
        with_isolated_store("tolerance", || {
            append_record(entry("2026-06-01T10:00:00+08:00", 5000)).unwrap();

            // 差 60 秒 ≤ 容差：视为同一次开机的时钟微调，合并（保留更早那条）。
            append_record(entry("2026-06-01T10:01:00+08:00", 90_000)).unwrap();
            assert_eq!(read_records().len(), 1);

            // 差 10 分钟 > 容差：是另一次真实开机，各留一条。
            append_record(entry("2026-06-01T10:10:00+08:00", 7000)).unwrap();
            assert_eq!(read_records().len(), 2);
        });
    }

    /// 时间戳解析不了时**不合并**：宁可多留一条，也不误删用户真实历史。
    #[test]
    fn unparsable_timestamp_never_merges() {
        with_isolated_store("unparsable", || {
            append_record(entry("2026-06-01T10:00:00+08:00", 5000)).unwrap();
            append_record(entry("not-a-date", 6000)).unwrap();
            assert_eq!(read_records().len(), 2);
        });
    }

    /// 历史超过上限时滚掉最旧的，保留最近的 N 条。
    #[test]
    fn records_are_capped() {
        with_isolated_store("cap", || {
            // 每条间隔 2 小时：远超会话容差，保证是 55 次**不同**开机。
            let base = parse_rfc3339("2026-07-01T08:00:00+08:00").unwrap();
            for i in 0..(MAX_RECORDS + 5) as i64 {
                let t = (base + chrono::Duration::hours(i * 2)).to_rfc3339();
                append_record(BootRecordEntry {
                    boot_started_at: t.clone(),
                    recorded_at: t,
                    total_ms: (i as u64 + 1) * 100,
                    source: "marker".to_string(),
                    basis: BASIS_LOG.to_string(),
                })
                .unwrap();
            }
            let got = read_records();
            assert_eq!(got.len(), MAX_RECORDS, "超出上限时应滚掉最旧的");
            // 滚掉的必须是最早的那批：最小的 total_ms 应为第 6 条（100*6）。
            assert_eq!(got[0].total_ms, 600, "保留的是最近 N 条");
        });
    }

    /// 任务 XML 的三条关键内容：**LeastPrivilege**（不弹 UAC 的全部依据）、
    /// 登录触发、以及只跑 `--mark-boot`。
    ///
    /// 这条测试是给"以后有人顺手改成 Highest"准备的——那一下造成的后果
    /// 是每次开机弹一次 UAC，而且只在真机上才看得出来。
    #[test]
    fn task_xml_is_least_privilege_and_logon_triggered() {
        let xml = task_xml(r"C:\Program Files\BootFlow\bootflow.exe");

        assert!(
            xml.contains("<RunLevel>LeastPrivilege</RunLevel>"),
            "必须以最低权限运行，否则每次登录会弹 UAC"
        );
        assert!(!xml.contains("HighestAvailable"), "绝不能改成最高权限");
        assert!(xml.contains("<LogonTrigger>"), "必须是登录触发");
        assert!(xml.contains("<Arguments>--mark-boot</Arguments>"),
            "必须只跑记账分支，不能把主界面拉起来");
        assert!(xml.contains(r"<Command>C:\Program Files\BootFlow\bootflow.exe</Command>"));
        // 笔记本不插电时也要记，否则历史会莫名缺条
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
    }

    /// 路径里的 `&` 必须被转义，否则整份 XML 解析失败、注册整个失败。
    #[test]
    fn task_xml_escapes_ampersand_in_path() {
        let xml = task_xml(r"C:\Tools\A & B\bootflow.exe");
        assert!(xml.contains("A &amp; B"), "路径里的 & 必须转义");
        assert!(!xml.contains("A & B"), "不允许出现未转义的 &");
    }

    /// XML 转义顺序：`&` 必须最先替换，否则后续生成的实体会被二次转义。
    #[test]
    fn escape_xml_does_not_double_escape() {
        assert_eq!(escape_xml("a&b"), "a&amp;b");
        assert_eq!(escape_xml("<a>"), "&lt;a&gt;");
        assert_eq!(escape_xml("\"q\"'"), "&quot;q&quot;&apos;");
    }

    /*
     * ——— 真机探针（`#[ignore]`，需要真实 Windows 环境）———
     *
     * 跑法：cargo test --lib probe_real_boot_basis -- --ignored --nocapture
     *
     * 这条探针的价值在于把那三个来源**并排打出来**：系统日志给的开机时刻、
     * tick 推算的开机时刻、以及两者的差。快速启动是否导致 tick 不重置，
     * 在这张表里一眼可见——比读文档可靠。
     */
    #[test]
    #[ignore]
    fn probe_real_boot_basis() {
        let now = chrono::Local::now();
        let tick_ms = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
        let tick_start = now - chrono::Duration::milliseconds(tick_ms as i64);

        println!("=== 自记账真机探针 ===");
        println!("现在            : {}", now.to_rfc3339());
        println!("tick 已运行     : {} ms ({:.1} 小时)", tick_ms, tick_ms as f64 / 3_600_000.0);
        println!("tick 推算起点   : {}", tick_start.to_rfc3339());

        match crate::diag::boot_log::read_last_boot_start() {
            Ok(iso) => {
                println!("系统日志开机时刻: {iso}");
                if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&iso) {
                    let local = t.with_timezone(&chrono::Local);
                    let delta = local.signed_duration_since(tick_start);
                    println!("日志本地化      : {}", local.to_rfc3339());
                    println!(
                        "日志 − tick     : {} 秒（正常应为正的小值；若达数小时/天，说明 tick 未重置）",
                        delta.num_seconds()
                    );
                }
            }
            Err(e) => println!("系统日志读取失败: {e}（将退回 tick 推算，basis=tick）"),
        }

        let e = BootRecordEntry::capture();
        println!("------ 实际落盘的那条 ------");
        println!("开机起点 : {}", e.boot_started_at);
        println!("记录时刻 : {}", e.recorded_at);
        println!("开机时长 : {} ms ({:.1} 秒)", e.total_ms, e.total_ms as f64 / 1000.0);
        println!("依据     : {}（log=实测 / tick=推算）", e.basis);
    }

    /*
     * 自启注册的真机探针。**只读，绝不写系统。**
     *
     * 曾经这条探针直接调 `ensure_autostart()`，结果把**测试二进制**的路径
     * 写进了 `HKCU\Run`：
     *   "…\target\debug\deps\bootflow_lib-<hash>.exe" --mark-boot
     * 开机时它会去跑测试套件，而带 hash 的文件名迟早被 cargo 清掉，
     * 之后每次开机就是一次静默失败。所以这里只看、不动。
     *
     * 端到端验证交给**发布版首次启动**（它会自己注册），探针只负责回答
     * "这台机器现在是什么状态、下一步会走哪条路"。
     */
    #[test]
    #[ignore]
    fn probe_real_autostart() {
        println!("=== 自启注册真机探针（只读）===");
        let elevated = crate::sys::is_elevated().unwrap_or(false);
        println!("提权运行: {elevated}");
        println!("自身路径: {:?}", std::env::current_exe());

        #[cfg(windows)]
        println!("计划任务 {TASK_PATH} 是否存在: {}", task_impl::exists(TASK_PATH));
        println!("HKCU Run 值 {RUN_VALUE_NAME}: {:?}", read_run_entry());

        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(str::to_string))
            .unwrap_or_else(|| r"C:\Program Files\BootFlow\bootflow.exe".to_string());
        println!("------ 发布版将会注册的任务 XML ------");
        println!("{}", task_xml(&exe));

        println!("------ 决策 ------");
        let plan = plan_autostart(
            {
                #[cfg(windows)]
                {
                    task_impl::exists(TASK_PATH)
                }
                #[cfg(not(windows))]
                {
                    false
                }
            },
            elevated,
        );
        println!("在**发布版**里下一步会走: {plan:?}");
        println!("（本探针不执行注册：用测试二进制注册只会污染系统。）");
    }

    /// 读回 `HKCU\Run` 里我们自己的值（只读，供探针报告现状用）。
    fn read_run_entry() -> Option<String> {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
        use winreg::RegKey;

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let run_key = hkcu
            .open_subkey_with_flags(
                r"Software\Microsoft\Windows\CurrentVersion\Run",
                KEY_READ,
            )
            .ok()?;
        run_key.get_value::<String, _>(RUN_VALUE_NAME).ok()
    }

    /// 决策表必须**永远只选一个入口**。
    ///
    /// 选错的表现是"每次登录启动两次"，很难从界面上发现。
    #[test]
    fn autostart_plan_never_picks_two_entry_points() {
        // 任务已在 → 不论是否提权都让路，绝不再写 Run 键
        assert_eq!(plan_autostart(true, false), AutostartPlan::KeepTask);
        assert_eq!(plan_autostart(true, true), AutostartPlan::KeepTask);
        // 没有任务：提权走计划任务（静默、无 UAC），非提权走 Run 键
        assert_eq!(plan_autostart(false, true), AutostartPlan::RegisterTask);
        assert_eq!(plan_autostart(false, false), AutostartPlan::WriteRunKey);
    }
}