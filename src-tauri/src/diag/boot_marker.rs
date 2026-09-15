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
//! ## 写入位置与文件
//!
//! `%LOCALAPPDATA%\BootFlow\boot-records.json`,滚动保留最近 N 条,天然可随版本
//! 存活,也便于导出。

use std::fs;
use std::path::PathBuf;

/// 保留最近多少条记录。
const MAX_RECORDS: usize = 50;

/// 自定义 app 数据目录名(不与 Tauri 官方 data dir 混淆)。
const APP_DIR: &str = "BootFlow";
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
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join(APP_DIR).join(RECORDS_FILE))
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

/// 「开机自记账」的 Run 键值名。
const RUN_VALUE_NAME: &str = "BootFlowBootMarker";

/// 注册开机自启（写 HKCU Run，幂等）。
///
/// 值形如：`"C:\...\BootFlow\bootflow.exe" --mark-boot`
/// 需要给exe加引号（路径可能带空格），参数恒为 `--mark-boot`。
///
/// 该函数只写**自己的** Run 值，不动任何人的其它值；写失败返回中文文案，
/// 由上层决定是否提示（正常 UI 下不打断用户）。
pub fn ensure_autostart() -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE, KEY_READ};
    use winreg::RegKey;

    // 当前进程的可执行文件路径。release 下带引号必要。
    let current_exe = std::env::current_exe().map_err(|e| format!("无法定位自身路径：{e}"))?;
    let Some(exe_str) = current_exe.to_str() else {
        return Err("自身路径非 UTF-8 无法注册".to_string());
    };
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

    run_key.set_value(RUN_VALUE_NAME, &command).map_err(|e| format!("写入 HKCU Run 失败：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 改进程级 `LOCALAPPDATA` 的用例必须串行：环境变量是全局的，
    /// 并行跑会让两条用例互相读到对方的目录，出现"随机"失败。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 在隔离的临时 `LOCALAPPDATA` 下执行，绝不污染真机记录。
    fn with_isolated_store<T>(name: &str, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
}