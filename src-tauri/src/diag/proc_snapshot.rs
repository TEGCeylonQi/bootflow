//! 进程快照 —— 「单项在开机后第几秒出现」的实测数据源。
//!
//! ## 先说清什么是**不存在**的（这条曾经写错过，留档）
//!
//! Windows **没有单个启动项的墙钟耗时计时器**。启动链上的三个执行者
//! （Explorer 拉注册表 Run 键、SCM 拉服务、Task Scheduler 拉计划任务）
//! 谁都不为自己的"孩子"打点。Event 100 给的是 12 个**相位**耗时，其中
//! `BootExplorerInitTime` 把"所有自启项"合成一个数字。
//!
//! ⚠️ 早期版本的注释到此为止，于是得出了"单项数据在物理上不存在"的结论——
//! **那个结论是错的**。Windows 确实为每一项留了数据，只是不在事件日志里：
//! 见 `diag::startup_info`（WDI 的 `StartupInfo`，任务管理器「启动影响」列
//! 读的就是它，里面有逐项的 CPU 时间与磁盘 IO）。
//!
//! 准确的切分是：
//!
//! | 量 | 存在吗 | 在哪 |
//! |---|---|---|
//! | 单项**墙钟耗时** | 只有被判慢的项有 | Event 101/102/103 |
//! | 单项**资源占用**（CPU/磁盘） | **每一项都有** | WDI `StartupInfo` |
//! | 单项**出现时刻** | **每一项都在的进程都有** | 本模块（内核） |
//!
//! ## 本模块负责的那一份
//!
//! 用 `GetProcessTimes` 拿进程创建时刻，换算成相对本次开机起点的毫秒——
//! 也就是"**它在开机后第几秒出现**"。另外顺带取 `GetProcessIoCounters`
//! 与 CPU 累计值回答"谁重"。
//!
//! 与 `startup_info` 的分工很清楚：
//!
//! | 模块 | 需要提权 | 零点 | 覆盖范围 |
//! |---|---|---|---|
//! | 本模块 | 否 | **内核启动** | 所有进程 |
//! | `startup_info` | **是** | **登录会话** | 登录窗口内拉起的进程 |
//!
//! 两者都要，因为失败方式互不相干：快照在任何权限下都有，
//! 但它的读盘量是累计值；`startup_info` 精确限定在窗口内，但没提权就完全没有。
//!
//! ## 诚实的边界（界面文案必须跟着这几条说）
//!
//! * `read_bytes` / `cpu_ms` 是**进程启动至今的累计值**，不是"开机阶段消耗"。
//!   拍得越早越接近开机消耗，所以快照带上 `captured_at_offset_ms`，
//!   界面必须把它一起说出来——否则用户会把"开机 3 分钟后累计读了 200MB"
//!   当成"开机时读了 200MB"。**想要"限定在登录窗口内"的数字，用 `startup_info`。**
//! * **不做**"拿累计值反推耗时"的换算。那是伪精度。
//! * 匹配不到进程的项（不以独立进程运行的注入类、本次没被拉起的项）
//!   一律如实标"未观测到"，绝不补一个看起来完整的数字。
//! * 只读，不需要管理员权限：`PROCESS_QUERY_LIMITED_INFORMATION` 对同用户
//!   进程即可；受保护进程（杀软自我保护等）打不开就跳过，不报错、不中断。

use serde::{Deserialize, Serialize};

/// 单个进程的实测采样点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcSnapshot {
    /// 小写可执行文件名（如 `chrome.exe`）。归因时按它对应启动项。
    pub name: String,
    /// 进程创建时刻，相对**本次开机起点**的毫秒数（内核给出，实测）。
    pub start_offset_ms: u64,
    /// 拍快照那一刻，该进程**从启动至今**累计从磁盘读取的字节数。
    pub read_bytes: u64,
    /// 拍快照那一刻，该进程**从启动至今**累计消耗的 CPU 毫秒（内核态 + 用户态）。
    pub cpu_ms: u64,
}

/// 一次进程快照。
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// 快照拍摄于开机后多久（毫秒）。
    ///
    /// `None` 表示连"本次开机起点"都没拿到，此时 `procs` 必定为空——
    /// 没有零点就没有可比的偏移量，硬编一个只会产出错的数字。
    pub captured_at_offset_ms: Option<u64>,
    /// 已按开机时刻升序排列。
    pub procs: Vec<ProcSnapshot>,
    /// 不可用时的原因（人话）。可用时为空。
    pub unavailable_reason: Option<String>,
}

impl Snapshot {
    fn unavailable(why: impl Into<String>) -> Self {
        Self {
            captured_at_offset_ms: None,
            procs: Vec::new(),
            unavailable_reason: Some(why.into()),
        }
    }

    /// 是否有可用的观测数据。
    pub fn is_available(&self) -> bool {
        self.captured_at_offset_ms.is_some()
    }
}

/// 单次快照保留多少个进程。
///
/// 本机实测活动进程数百个，其中绝大多数与开机无关（用户后来手动开的）。
/// 排序后截断而不是全收：快照要跟着启动项列表一起进内存和界面，
/// 没有理由让"记事本"占一个位置。
const MAX_PROCS: usize = 300;

/// 只保留开机后这段窗口内启动的进程（毫秒）。
///
/// 取 10 分钟的余量：Windows 自己把"开机后 10 秒"算作 post-boot，
/// 这里放宽到 10 分钟是为了覆盖开机很慢的机器。再往后启动的进程
/// （用户手动打开的程序、定时任务在上午十点跑的东西）不属于"开机开销"，
/// 收进来只会让时间轴被拉长到看不清。
const BOOT_WINDOW_MS: u64 = 10 * 60 * 1000;

/// 拍当前进程快照。**只读，任何失败都退化为"不可用"，绝不 panic。**
pub fn capture() -> Snapshot {
    // 零点用系统日志里的**本次开机**时刻，不用 GetTickCount64。
    // 原因见 `boot_log::read_last_boot_start`：tick 在快速启动下不重置，
    // 拿它当零点会把"开机后 8 秒"算成"开机后三天"。
    let Some(boot_start) = boot_start() else {
        return Snapshot::unavailable(
            "读不到本次开机的起点时刻，无法把进程创建时刻换算成\"开机后第几秒\"。".to_string(),
        );
    };

    let now = chrono::Local::now();
    let captured_at_offset_ms = (now - boot_start).num_milliseconds().max(0) as u64;

    #[cfg(windows)]
    let mut procs = windows_impl::enumerate(&boot_start);
    #[cfg(not(windows))]
    let mut procs: Vec<ProcSnapshot> = Vec::new();

    procs.retain(|p| p.start_offset_ms <= BOOT_WINDOW_MS);

    // 按创建时刻升序：时间轴逐项定位时天然就是顺序的，
    // 也顺带成了归因模块的一个可预期输入。
    procs.sort_by_key(|p| p.start_offset_ms);
    procs.dedup_by(|a, b| a.name == b.name && a.start_offset_ms == b.start_offset_ms);
    procs.truncate(MAX_PROCS);

    Snapshot {
        captured_at_offset_ms: Some(captured_at_offset_ms),
        procs,
        unavailable_reason: None,
    }
}

/// 本次开机的起点（本地时区）。读不到返回 `None`。
fn boot_start() -> Option<chrono::DateTime<chrono::Local>> {
    let iso = crate::diag::boot_log::read_last_boot_start().ok()?;
    chrono::DateTime::parse_from_rfc3339(&iso)
        .ok()
        .map(|t| t.with_timezone(&chrono::Local))
}

#[cfg(windows)]
mod windows_impl {
    //! Windows 侧实现。全部是**只读**查询，且逐个进程都能失败退出。

    use super::ProcSnapshot;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, MAX_PATH};
    use windows::Win32::System::ProcessStatus::EnumProcesses;
    use windows::Win32::System::Threading::{
        GetProcessIoCounters, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// 1601-01-01 到 1970-01-01 之间有多少个 100ns 单位。
    /// `FILETIME` 的纪元是 1601，`SystemTime` 的是 1970，换算要用它。
    const EPOCH_DIFF_100NS: u64 = 116_444_736_000_000_000;

    /// 一次枚举多少个 PID。首次分配得宽一点，超了再重试一次更大的。
    const FIRST_BATCH: usize = 2048;
    const SECOND_BATCH: usize = 16384;

    fn ft_to_u64(ft: FILETIME) -> u64 {
        ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
    }

    /// 当前时刻的 FILETIME 口径（100ns，1601 纪元）。
    ///
    /// 用 `SystemTime` 换算而不是 `GetSystemTimeAsFileTime`：两者等价，
    /// 但前者不引入一个新的 unsafe 调用与 feature 依赖，且精度够。
    fn now_ft() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        let since_epoch_100ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64 / 100)
            .unwrap_or(0);
        EPOCH_DIFF_100NS + since_epoch_100ns
    }

    /// 取进程主模块的完整路径，再取文件名小写。
    /// 路径包含非 UTF-8 时按 lossy 处理——文件名参与的是匹配，不是展示。
    fn exe_name(handle: HANDLE) -> Option<String> {
        let mut buf = [0u16; MAX_PATH as usize];
        let mut len = buf.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_FORMAT(0),
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            )
            .ok()?;
        }
        if len == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let base = path.rsplit(['\\', '/']).next()?;
        if base.is_empty() {
            None
        } else {
            Some(base.to_ascii_lowercase())
        }
    }

    /// 列出全部 PID；缓冲区不够大时按更大的尺寸重试一次。
    fn all_pids() -> Vec<u32> {
        for cap in [FIRST_BATCH, SECOND_BATCH] {
            let mut pids = vec![0u32; cap];
            let mut needed: u32 = 0;
            let bytes = (pids.len() * std::mem::size_of::<u32>()) as u32;
            let ok = unsafe { EnumProcesses(pids.as_mut_ptr(), bytes, &mut needed) };
            if ok.is_err() {
                return Vec::new();
            }
            let count = needed as usize / std::mem::size_of::<u32>();
            // 没被填满说明这次就够用，可以收工。
            if count < cap {
                pids.truncate(count);
                return pids;
            }
        }
        Vec::new()
    }

    /// 枚举全部进程并采样。任何单个进程取不到数据就跳过它。
    pub fn enumerate(boot_start: &chrono::DateTime<chrono::Local>) -> Vec<ProcSnapshot> {
        let pids = all_pids();
        if pids.is_empty() {
            return Vec::new();
        }

        let now = now_ft();
        let boot_utc = boot_start.with_timezone(&chrono::Utc);
        let boot_ft = boot_utc.timestamp() as i128 * 10_000_000
            + boot_utc.timestamp_subsec_nanos() as i128 / 100
            + EPOCH_DIFF_100NS as i128;

        let self_pid = std::process::id();
        let mut out = Vec::new();

        for pid in pids {
            // 0 与 4 是 Idle / System 两个伪进程，没有"在哪一刻启动"可言。
            if pid == 0 || pid == 4 || pid == self_pid {
                continue;
            }

            let Ok(handle) = (unsafe {
                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            }) else {
                // 受保护进程 / 已退出：拿不到就跳过，这是常态不是错误。
                continue;
            };

            let sampled = sample(handle, now, boot_ft);
            let _ = unsafe { CloseHandle(handle) };

            if let Some(p) = sampled {
                out.push(p);
            }
        }

        out
    }

    fn sample(handle: HANDLE, now: u64, boot_ft: i128) -> Option<ProcSnapshot> {
        let mut create = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe {
            GetProcessTimes(handle, &mut create, &mut exit, &mut kernel, &mut user).ok()?;
        }

        let create_ft = ft_to_u64(create) as i128;
        // 创建时刻早于开机起点 = 内核从 hiberfil 恢复出来的进程
        // （快速启动场景）。它的"开机后第几秒"是 0，不是负数。
        let start_offset_ms = ((create_ft - boot_ft).max(0) / 10_000) as u64;

        // 累计 CPU：内核态 + 用户态。
        let cpu_ms = (ft_to_u64(kernel) + ft_to_u64(user)) / 10_000;

        let mut io = windows::Win32::System::Threading::IO_COUNTERS::default();
        // I/O 计数取不到不算失败——创建时刻才是主角，读盘量只是补充。
        let read_bytes = match unsafe { GetProcessIoCounters(handle, &mut io) } {
            Ok(()) => io.ReadTransferCount,
            Err(_) => 0,
        };

        let name = exe_name(handle)?;

        // 年龄只用于兜底：正常情况下创建时刻本身就是答案。
        let _ = now;

        Some(ProcSnapshot {
            name,
            start_offset_ms,
            read_bytes,
            cpu_ms,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn filetime_round_trip() {
            let ft = FILETIME {
                dwLowDateTime: 0x1234_5678,
                dwHighDateTime: 0x9abc_def0,
            };
            assert_eq!(ft_to_u64(ft), 0x9abc_def0_1234_5678);
        }

        #[test]
        fn epoch_diff_matches_known_value() {
            // 1970-01-01 的 FILETIME 值是固定的公开常量，用来锁住换算常数。
            assert_eq!(EPOCH_DIFF_100NS, 116_444_736_000_000_000);
        }

        /// 真机探针：打印本机快照的前若干条，人工确认数字合理
        /// （创建时刻应在几分钟内，读盘量应为正）。
        ///
        /// 跑法：`cargo test --lib probe_real_snapshot -- --ignored --nocapture`
        #[test]
        #[ignore]
        fn probe_real_snapshot() {
            let snap = super::super::capture();
            println!("=== 进程快照真机探针 ===");
            println!("可用: {}", snap.is_available());
            if let Some(why) = &snap.unavailable_reason {
                println!("不可用原因: {why}");
                return;
            }
            println!(
                "拍摄于开机后 {:.1}s，共 {} 个进程（窗口内）",
                snap.captured_at_offset_ms.unwrap_or(0) as f64 / 1000.0,
                snap.procs.len()
            );
            let mut by_io: Vec<_> = snap.procs.iter().collect();
            by_io.sort_by_key(|p| std::cmp::Reverse(p.read_bytes));
            println!("--- 按累计读盘排序前 10 ---");
            for p in by_io.iter().take(10) {
                println!(
                    "{:>8.1}s  {:>9.1}MB  cpu {:>6}ms  {}",
                    p.start_offset_ms as f64 / 1000.0,
                    p.read_bytes as f64 / 1_048_576.0,
                    p.cpu_ms,
                    p.name
                );
            }
            println!("--- 最早出现的 10 个 ---");
            for p in snap.procs.iter().take(10) {
                println!(
                    "{:>8.1}s  {:>9.1}MB  cpu {:>6}ms  {}",
                    p.start_offset_ms as f64 / 1000.0,
                    p.read_bytes as f64 / 1_048_576.0,
                    p.cpu_ms,
                    p.name
                );
            }
        }
    }
}
