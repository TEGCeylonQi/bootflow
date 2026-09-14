//! 服务扫描器。
//!
//! Windows 服务是自启机制里**最容易被误解**的一类：
//! `svchost.exe` 一个进程承载几十个服务，但那是"宿主进程"而不是"这个服务"。
//! 所以这里显示的是**服务**，不是承载它的 exe。
//!
//! ## 收哪些（这是本模块最需要交代清楚的决策）
//!
//! | 条件 | 为什么收 |
//! |---|---|
//! | `Start = 2`（自动） | 这是"开机自启"的正面定义，含延迟启动 |
//! | `Start = 3`（手动）**且有触发器** | 平时不启动，由设备到达/网络可用等事件拉起——语义上仍是自启 |
//! | 命中 `diag::known` 白名单 | 见下 |
//!
//! **不收** `Start = 3` 且无触发器、以及 `Start = 4`（禁用）的服务——
//! 它们不是启动项。这一条很关键：本机 351 个服务里绝大多数属于这一类，
//! 全收进来会把真正的自启项淹掉。
//!
//! ## 为什么白名单项即使"手动/禁用"也要收
//!
//! 这是一个反直觉但必须的设计。本机实测 `ClickToRunSvc` 是 `DEMAND_START`
//! （被优化软件改成了手动）。如果只按 `Start = 2` 过滤，这个服务**根本不会
//! 出现在结果里**，那么"它的启动方式被改坏了"这条诊断就永远无法产生——
//! 用户最该看到的那条信息，被过滤器挡在了门外。
//!
//! 所以：过滤器负责"什么东西值得看"，而白名单是"我们提前知道值得看的"。
//!
//! ## 只读
//!
//! 本模块只调用 `OpenSCManagerW`（`SC_MANAGER_ENUMERATE_SERVICE`）、
//! `QueryServiceConfigW`、`QueryServiceConfig2W`——全是查询。
//! 没有任何 `ChangeServiceConfig` 的调用路径。

use crate::diag::known;
use crate::model::{BootPhase, DiagnosticInfo, NameSource, SourceKind, StartupItem};
use crate::scanners::builder::ItemBuilder;
use crate::util::cmdline;

/// 为什么收这个服务。带原因是为了让诊断能说清依据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CollectReason {
    /// `Start = 2`：开机自动启动
    AutoStart,
    /// `Start = 3` 且配了触发器：由系统事件拉起
    Triggered,
}

/// 一个服务的完整读取结果。
#[derive(Debug, Clone)]
struct ServiceRecord {
    name: String,
    display_name: String,
    /// `dwStartType`：2=自动 3=手动 4=禁用
    start_type: u32,
    /// `DelayedAutostart`：只在自动启动时有意义
    delayed: bool,
    image_path: String,
    /// 配了几个触发器（0 表示没有）
    trigger_count: u32,
    /// 触发器里有没有"系统状态变化"这一类（开机/登录类多半属于它）
    has_system_state_trigger: bool,
}

impl ServiceRecord {
    fn is_auto(&self) -> bool {
        self.start_type == 2
    }

    fn is_disabled(&self) -> bool {
        self.start_type == 4
    }

    /// 当前状态是否**不是**出厂预期的"自动"。
    ///
    /// 延迟启动也算自动——`wscsvc` 这类服务的出厂默认就是"自动（延迟）"，
    /// 把它判成"没自动"会造出一批假警报。
    fn not_auto(&self) -> bool {
        !self.is_auto()
    }

    /// 启动时机的人话描述。
    fn timing_phrase(&self) -> String {
        if self.is_disabled() {
            return "已禁用，不会启动".to_string();
        }
        if self.is_auto() {
            return if self.delayed {
                "开机时自动启动，但会等关键服务先起来之后再启动".to_string()
            } else {
                "开机时由系统自动启动".to_string()
            };
        }
        if self.trigger_count > 0 {
            return "平时不启动，由系统事件（如设备接入、网络可用）触发".to_string();
        }
        "需要时手动启动".to_string()
    }
}

/// 服务是否需要收进结果。
fn collect_reason(rec: &ServiceRecord) -> Option<CollectReason> {
    if rec.is_auto() {
        return Some(CollectReason::AutoStart);
    }
    // 触发器驱动的按需启动，语义上仍是"自启"
    if !rec.is_disabled() && rec.trigger_count > 0 {
        return Some(CollectReason::Triggered);
    }
    // 白名单：即使当前是手动/禁用也要收，否则诊断无法产生
    if known::lookup_auto_service(&rec.name).is_some() {
        return Some(CollectReason::Triggered);
    }
    None
}

/// 从一条服务记录构造 `StartupItem`。返回 `None` 表示这个服务不该收。
fn build_item(rec: &ServiceRecord) -> Option<StartupItem> {
    let reason = collect_reason(rec)?;
    let known_entry = known::lookup_auto_service(&rec.name);

    // ImagePath 的展开：服务路径可能是 `\SystemRoot\...` 这样的
    // "相对 SystemRoot" 写法（从注册表直接读时会见到），
    // 也可能带参数。先补前缀，再拆命令与参数。
    let expanded = expand_image_path(&rec.image_path);
    let (target, args) = cmdline::split_command(&expanded);

    let mut builder = ItemBuilder::new(SourceKind::Service, &rec.name)
        .location(format!(r"服务（{}）", scope_phrase(rec)))
        .command(rec.image_path.clone())
        .target(target)
        .args(args)
        .enabled(!rec.is_disabled())
        .boot_phase(service_boot_phase(rec));

    // 显示名：服务的 DisplayName 通常比 exe 的版本信息更贴近语境
    // （"Microsoft Office Click-to-Run Service" 好过 "OfficeClickToRun"）。
    // 但有些服务的 DisplayName 是占位符或等于服务名，builder 会过滤空值。
    if !rec.display_name.trim().is_empty() {
        builder = builder.display_name(rec.display_name.clone(), NameSource::ServiceDisplayName);
    }

    // ── 诊断：状态与预期不符 ──
    if let Some(entry) = known_entry {
        if rec.not_auto() {
            let actual = match rec.start_type {
                3 => "手动",
                4 => "禁用",
                _ => "非自动",
            };
            builder = builder.diagnostic(DiagnosticInfo {
                code: crate::diag::codes::MANUAL_BUT_SHOULD_AUTO.to_string(),
                severity: entry.severity,
                message: format!(
                    "{} 的启动类型被设成了「{}」，而它出厂默认是「{}」。{}",
                    entry.purpose, actual, entry.default_start, entry.consequence
                ),
                evidence: Some(format!(
                    "服务 {} 当前 Start={}，预期为自动",
                    rec.name,
                    rec.start_type
                )),
            });
        }
    }

    let mut item = builder
        .raw(serde_json::json!({
            "serviceName": rec.name,
            "startType": rec.start_type,
            "delayedAutostart": rec.delayed,
            "imagePath": rec.image_path,
            "triggerCount": rec.trigger_count,
            "hasSystemStateTrigger": rec.has_system_state_trigger,
            "collectReason": format!("{reason:?}"),
        }))
        .build();

    // 一句话说明：为什么它进了这份清单
    item.summary = Some(match reason {
        CollectReason::AutoStart => rec.timing_phrase(),
        CollectReason::Triggered if rec.trigger_count > 0 => rec.timing_phrase(),
        // 白名单项走到这里说明它当前不是自动——summary 要如实说"它现在不会自己启动"
        CollectReason::Triggered => format!("{}。当前状态：{}", rec.timing_phrase(), status_phrase(rec)),
    });

    Some(item)
}

fn status_phrase(rec: &ServiceRecord) -> &'static str {
    match rec.start_type {
        2 => "自动",
        3 => "手动",
        4 => "已禁用",
        _ => "未知",
    }
}

fn scope_phrase(_rec: &ServiceRecord) -> &'static str {
    // 服务一律是机器级：服务在登录前就可能启动，不存在"只给当前用户"的服务。
    "全机器"
}

/// 服务落在哪个开机阶段。
///
/// 判据是 Windows 的实际行为，不是猜测：
/// - **自动启动的服务**由 SCM 在会话管理器（SMSS）阶段拉起，
///   所以落在 `Smss`。
/// - **延迟启动**的服务被刻意排到关键服务之后，实际发生在登录前后，落 `Logon`。
/// - **触发器驱动**的按需启动，发生在用户会话里，落 `Logon`。
///
/// 服务级 Event 103 数据会覆盖这个估算（见 `diag::timeline`），
/// 那时 `timing.confidence` 会从 `Estimated` 变成 `Measured`。
fn service_boot_phase(rec: &ServiceRecord) -> BootPhase {
    if rec.is_auto() && !rec.delayed {
        BootPhase::Smss
    } else {
        BootPhase::Logon
    }
}

/// 展开服务 ImagePath 里可能出现的相对写法。
///
/// 注册表里的服务路径常见三种形态，都必须处理：
/// 1. `"C:\Program Files\App\svc.exe" -arg` —— 标准写法
/// 2. `\SystemRoot\System32\drivers\x.sys` —— 相对 `%SystemRoot%`（驱动居多）
/// 3. `\??\C:\...` —— NT 对象管理器前缀（本机实测存在）
///
/// 不处理第 2、3 种的后果是路径判存在性全部失败，
/// 于是这些服务会被集体标成"程序已被卸载"——一次几十条误报。
fn expand_image_path(raw: &str) -> String {
    let trimmed = raw.trim();
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());

    let lower = trimmed.to_ascii_lowercase();

    if let Some(rest) = lower.strip_prefix(r"\systemroot") {
        // 用原文的对应片段，保留大小写（Windows 路径本身不区分大小写，
        // 但展示给用户时保持原样更易于与注册表对照）
        return format!("{root}{}", &trimmed[trimmed.len() - rest.len()..]);
    }

    if let Some(rest) = trimmed.strip_prefix(r"\??\") {
        return rest.to_string();
    }
    if let Some(rest) = trimmed.strip_prefix(r"\\?\") {
        return rest.to_string();
    }

    trimmed.to_string()
}

/// 扫描所有符合条件的服务。
pub fn collect() -> Result<Vec<StartupItem>, String> {
    #[cfg(windows)]
    {
        let records = windows_impl::enumerate_services()?;
        let total = records.len();
        let items: Vec<StartupItem> = records.iter().filter_map(build_item).collect();

        log::info!(
            "服务扫描：读取 {total} 个，其中 {} 个属于自启（其余是手动/禁用且无触发器）",
            items.len()
        );

        Ok(items)
    }

    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::ServiceRecord;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::ERROR_MORE_DATA;
    use windows::Win32::System::Services::*;

    /// `SC_HANDLE` 的 RAII 包装。服务句柄泄漏不会立刻出问题，
    /// 但扫描一次几百个句柄，不关就是实打实的资源泄漏。
    struct ScHandle(SC_HANDLE);

    impl Drop for ScHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseServiceHandle(self.0);
            }
        }
    }

    /// `PWSTR` → `String`。空指针按空串处理。
    ///
    /// 这里刻意不调用 `PWSTR::to_string()`：那个方法依赖系统提供的
    /// 长度语义，而 SCM 返回的字符串一定有 NUL 终止，自己数长度更直接。
    unsafe fn pwstr_to_string(p: PWSTR) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        // 防御上限：正常路径不会超过 32K 字符，超过说明指针有问题，
        // 与其一路读下去撞访问违例，不如提前停下返回已读到的部分。
        while len < 32768 && *p.0.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p.0, len))
    }

    /// 分配一块**满足指针对齐**的缓冲区，并以 `&mut [u8]` 视图交给 API。
    ///
    /// ⚠️ 这里有个不能省的细节：`EnumServicesStatusExW` 与
    /// `QueryServiceConfig2W` 在 `windows` crate 里的签名收 `&mut [u8]`，
    /// 但结构体（`ENUM_SERVICE_STATUS_PROCESSW` / `SERVICE_TRIGGER_INFO`）
    /// 都**含指针**，要求 8 字节对齐。直接用 `Vec<u8>` 是 UB——
    /// `Vec<u8>` 的对齐只有 1，而这个错误在 x86-64 上多数时候"看起来能跑"，
    /// 只在特定分配位置才崩，是最难查的那种。
    ///
    /// 用 `Vec<u64>` 打底就天然满足 8 字节对齐。
    fn aligned_buffer(byte_len: usize) -> Vec<u64> {
        vec![0u64; byte_len.div_ceil(8)]
    }

    fn as_bytes_mut(buf: &mut [u64], byte_len: usize) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, byte_len) }
    }

    /// 枚举全部 Win32 服务并逐个读取配置。
    pub fn enumerate_services() -> Result<Vec<ServiceRecord>, String> {
        unsafe {
            let scm = OpenSCManagerW(None, None, SC_MANAGER_ENUMERATE_SERVICE)
                .map_err(|e| format!("无法连接服务管理器：{e}"))?;
            let scm = ScHandle(scm);

            let mut records = Vec::new();
            let mut resume: u32 = 0;

            loop {
                // 先要一块足够大的缓冲。第一次按经验给 64KB，
                // 不够就按 API 回报的需求重新分配。
                let mut need: u32 = 0;
                let mut returned: u32 = 0;
                let mut buf = aligned_buffer(64 * 1024);
                let buf_len = buf.len() * 8;

                let res = EnumServicesStatusExW(
                    scm.0,
                    SC_ENUM_PROCESS_INFO,
                    SERVICE_WIN32,
                    // 运行中 + 已停止都要：一个自动启动但当前已停止的服务
                    // 依然是启动项，漏掉它会低估开机负担。
                    ENUM_SERVICE_STATE(SERVICE_ACTIVE.0 | SERVICE_INACTIVE.0),
                    Some(as_bytes_mut(&mut buf, buf_len)),
                    &mut need,
                    &mut returned,
                    Some(&mut resume),
                    None,
                );

                match res {
                    Ok(()) => {}
                    Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => {
                        // 缓冲区不够：按 API 要求的字节数重来一次。
                        // 这里**不是**用 resume 继续，而是重新全量枚举，
                        // resume 语义在这里容易出错（会从中间接续导致丢服务）。
                        let mut big = aligned_buffer(need as usize);
                        let big_len = big.len() * 8;
                        EnumServicesStatusExW(
                            scm.0,
                            SC_ENUM_PROCESS_INFO,
                            SERVICE_WIN32,
                            ENUM_SERVICE_STATE(SERVICE_ACTIVE.0 | SERVICE_INACTIVE.0),
                            Some(as_bytes_mut(&mut big, big_len)),
                            &mut need,
                            &mut returned,
                            Some(&mut resume),
                            None,
                        )
                        .map_err(|e| format!("枚举服务失败：{e}"))?;

                        collect_from_buffer(&big, returned, &mut records);
                        break;
                    }
                    Err(e) => return Err(format!("枚举服务失败：{e}")),
                }

                if returned == 0 {
                    break;
                }

                collect_from_buffer(&buf, returned, &mut records);

                // SCM 缓冲一次放不下时返回 ERROR_MORE_DATA，正常走 Ok 就说明拿全了。
                // 但个别系统上会返回 Ok 且 need > 现有缓冲，所以补一道判断。
                if (need as usize) <= buf_len {
                    break;
                }
                resume = 0;
            }

            // 逐个读详细配置。读不到的服务跳过而不是整体失败——
            // 个别服务的 ACL 受限是常态。
            let mut out = Vec::with_capacity(records.len());
            for (name, display) in records {
                if let Some(rec) = read_config(scm.0, &name, &display) {
                    out.push(rec);
                }
            }

            Ok(out)
        }
    }

    /// 把 `EnumServicesStatusExW` 填好的缓冲解析成 `(服务名, 显示名)` 列表。
    fn collect_from_buffer(buf: &[u64], count: u32, out: &mut Vec<(String, String)>) {
        unsafe {
            let base = buf.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW;
            let stride = std::mem::size_of::<ENUM_SERVICE_STATUS_PROCESSW>();

            // 缓冲是按 8 字节对齐分配的，可能比实际数据长；
            // 能装下多少个结构体要按实际字节数算，不能按 count 之外的假设。
            let capacity = (buf.len() * 8) / stride;
            let n = (count as usize).min(capacity);

            for i in 0..n {
                let rec = &*base.add(i);
                let name = pwstr_to_string(rec.lpServiceName);
                if name.is_empty() {
                    continue;
                }
                let display = pwstr_to_string(rec.lpDisplayName);
                out.push((name, display));
            }
        }
    }

    /// 读单个服务的启动类型、路径与触发器。
    fn read_config(scm: SC_HANDLE, name: &str, display: &str) -> Option<ServiceRecord> {
        unsafe {
            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

            let handle = OpenServiceW(
                scm,
                windows::core::PCWSTR(wide.as_ptr()),
                SERVICE_QUERY_CONFIG,
            )
            .ok()?;
            let handle = ScHandle(handle);

            let (start_type, image_path, cfg_display) = query_config(handle.0)?;

            Some(ServiceRecord {
                name: name.to_string(),
                display_name: if cfg_display.trim().is_empty() {
                    display.to_string()
                } else {
                    cfg_display
                },
                start_type,
                delayed: query_delayed(handle.0),
                image_path,
                trigger_count: 0,
                has_system_state_trigger: false,
            })
            .map(|mut rec| {
                let (count, state) = query_triggers(handle.0);
                rec.trigger_count = count;
                rec.has_system_state_trigger = state;
                rec
            })
        }
    }

    /// `QueryServiceConfigW` → `(启动类型, 二进制路径, 显示名)`。
    ///
    /// 这个 API 的标准用法是"先问大小、再取数据"，
    /// 而它在问大小时**必然返回错误**（`ERROR_INSUFFICIENT_BUFFER`）——
    /// 所以第一次调用要忽略返回码，只看 `need`。
    /// 把这个错误当成失败会让所有服务都读不到配置。
    unsafe fn query_config(handle: SC_HANDLE) -> Option<(u32, String, String)> {
        let mut need: u32 = 0;
        let _ = QueryServiceConfigW(handle, None, 0, &mut need);
        if need == 0 {
            return None;
        }

        let mut buf = aligned_buffer(need as usize);
        let cfg = buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW;

        QueryServiceConfigW(handle, Some(cfg), need, &mut need).ok()?;

        let cfg = &*cfg;
        let image = pwstr_to_string(cfg.lpBinaryPathName);
        let display = pwstr_to_string(cfg.lpDisplayName);

        // `dwStartType` 是 newtype，取内层数值
        Some((cfg.dwStartType.0, image, display))
    }

    /// 是否延迟启动。读不到时按"不延迟"处理。
    ///
    /// 默认值的选择有讲究：`SERVICE_DELAYED_AUTO_START_INFO` 这个配置项
    /// **不存在**就等于"不延迟"（Windows 只在服务被设为延迟时才写这个键）。
    /// 反过来默认成"延迟"会把大量普通自动服务说成延迟启动。
    unsafe fn query_delayed(handle: SC_HANDLE) -> bool {
        let mut need: u32 = 0;
        let _ = QueryServiceConfig2W(
            handle,
            SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
            None,
            &mut need,
        );
        if need == 0 {
            return false;
        }

        let mut buf = aligned_buffer(need as usize);
        if QueryServiceConfig2W(
            handle,
            SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
            Some(as_bytes_mut(&mut buf, need as usize)),
            &mut need,
        )
        .is_err()
        {
            return false;
        }

        let info = &*(buf.as_ptr() as *const SERVICE_DELAYED_AUTO_START_INFO);
        info.fDelayedAutostart.as_bool()
    }

    /// `(触发器数量, 是否含系统状态变化触发器)`。
    ///
    /// 只读 `cTriggers` 与各触发器的 `dwTriggerType`，**不深挖 subtype GUID**。
    ///
    /// 这是一条刻意的防线：`SERVICE_TRIGGER_TYPE_CUSTOM_SYSTEM_STATE_CHANGE`
    /// 只说明"由系统状态变化触发"，具体是开机、登录还是从睡眠恢复，
    /// 取决于 `pTriggerSubtype` 指向的 GUID。那些 GUID 我没有权威出处，
    /// 凭记忆写下一串 16 进制数，错了也不会有人发现——而诊断会因此开始说谎。
    /// 所以这里只陈述"由系统事件触发"这个能被证实的事实。
    unsafe fn query_triggers(handle: SC_HANDLE) -> (u32, bool) {
        let mut need: u32 = 0;
        let _ = QueryServiceConfig2W(handle, SERVICE_CONFIG_TRIGGER_INFO, None, &mut need);
        if need == 0 {
            return (0, false);
        }

        let mut buf = aligned_buffer(need as usize);
        if QueryServiceConfig2W(
            handle,
            SERVICE_CONFIG_TRIGGER_INFO,
            Some(as_bytes_mut(&mut buf, need as usize)),
            &mut need,
        )
        .is_err()
        {
            return (0, false);
        }

        let info = &*(buf.as_ptr() as *const SERVICE_TRIGGER_INFO);
        if info.cTriggers == 0 || info.pTriggers.is_null() {
            return (0, false);
        }

        // 触发器数据与头部在**同一块缓冲**内，指针有效；
        // 但仍要防一手越界：cTriggers 若被写坏成巨大的值，
        // 下面的循环会一路读出去。
        let cap = (buf.len() * 8) / std::mem::size_of::<SERVICE_TRIGGER>();
        let n = (info.cTriggers as usize).min(cap.max(1));

        let mut has_state = false;
        for i in 0..n {
            let t = &*info.pTriggers.add(i);
            // ⚠️ `dwTriggerType` 是 newtype，而这个常量在 windows crate 里
            // 是裸 `u32`——所以取内层数值再比，两边类型才对得上。
            if t.dwTriggerType.0 == SERVICE_TRIGGER_TYPE_CUSTOM_SYSTEM_STATE_CHANGE {
                has_state = true;
            }
        }

        (info.cTriggers, has_state)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn aligned_buffer_is_eight_byte_aligned() {
            let buf = aligned_buffer(13);
            assert_eq!(buf.as_ptr() as usize % 8, 0, "缓冲必须满足指针对齐");
            assert!(buf.len() * 8 >= 13, "缓冲不能小于请求的字节数");
        }

        #[test]
        fn expand_handles_nt_prefixes() {
            // 这两种前缀本机都实测出现过，漏处理会导致路径判存在性全部失败
            assert_eq!(
                super::super::expand_image_path(r"\??\C:\Windows\a.exe"),
                r"C:\Windows\a.exe"
            );
            assert_eq!(
                super::super::expand_image_path(r"\\?\C:\Windows\a.exe"),
                r"C:\Windows\a.exe"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(name: &str, start_type: u32, delayed: bool, triggers: u32) -> ServiceRecord {
        ServiceRecord {
            name: name.into(),
            display_name: name.into(),
            start_type,
            delayed,
            image_path: r"C:\Windows\System32\svchost.exe".into(),
            trigger_count: triggers,
            has_system_state_trigger: false,
        }
    }

    #[test]
    fn auto_start_services_are_collected() {
        assert_eq!(
            collect_reason(&rec("Foo", 2, false, 0)),
            Some(CollectReason::AutoStart)
        );
        // 延迟启动也是自动
        assert_eq!(
            collect_reason(&rec("Bar", 2, true, 0)),
            Some(CollectReason::AutoStart)
        );
    }

    #[test]
    fn manual_service_without_trigger_is_not_an_autostart_item() {
        // 本机 351 个服务里绝大多数属于这一类。收进来会把真正的自启项淹掉。
        assert_eq!(collect_reason(&rec("BITS", 3, false, 0)), None);
    }

    #[test]
    fn manual_service_with_trigger_counts_as_autostart() {
        // 触发器驱动的按需启动，语义上仍是"自己会起来"
        assert_eq!(
            collect_reason(&rec("Foo", 3, false, 1)),
            Some(CollectReason::Triggered)
        );
    }

    #[test]
    fn disabled_known_service_is_collected_so_diagnosis_can_fire() {
        // 这条是整个白名单机制存在的理由：
        // Start=4（禁用）的服务如果被过滤掉，"它本该自动"这条诊断
        // 就永远不可能产生——而它恰恰是用户最该看到的信息。
        let r = rec("WinDefend", 4, false, 0);
        assert!(
            collect_reason(&r).is_some(),
            "白名单里的服务即使被禁用也必须收进来"
        );
    }

    #[test]
    fn disabled_unknown_service_is_skipped() {
        assert_eq!(collect_reason(&rec("SomeRandomSvc", 4, false, 0)), None);
    }

    #[test]
    fn known_service_changed_to_manual_produces_diagnostic() {
        let r = rec("ClickToRunSvc", 3, false, 0);
        let item = build_item(&r).expect("应被收进来");
        let d = item
            .diagnostics
            .iter()
            .find(|d| d.code == crate::diag::codes::MANUAL_BUT_SHOULD_AUTO)
            .expect("应产生 MANUAL_BUT_SHOULD_AUTO 诊断");

        // 证据里要有实际读到的数值，否则用户无法复核
        assert!(d.evidence.as_deref().unwrap_or("").contains("Start=3"));
        // 结论要说人话，不能只是代码
        assert!(d.message.contains("手动"));
        assert!(d.message.contains("自动"));
    }

    #[test]
    fn known_service_already_auto_produces_no_diagnostic() {
        // 正常机器上白名单必须是静默的，否则就成了新的噪音源
        let r = rec("WinDefend", 2, false, 0);
        let item = build_item(&r).unwrap();
        assert!(item.diagnostics.is_empty());
    }

    #[test]
    fn delayed_auto_service_lands_in_logon_phase() {
        // 延迟启动被刻意排到关键服务之后，实际发生在登录前后
        assert_eq!(service_boot_phase(&rec("A", 2, true, 0)), BootPhase::Logon);
        assert_eq!(service_boot_phase(&rec("B", 2, false, 0)), BootPhase::Smss);
        assert_eq!(service_boot_phase(&rec("C", 3, false, 1)), BootPhase::Logon);
    }

    #[test]
    fn image_path_expansion_handles_systemroot_prefix() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let out = expand_image_path(r"\SystemRoot\System32\drivers\foo.sys");
        assert_eq!(out, format!(r"{root}\System32\drivers\foo.sys"));

        // 标准绝对路径不该被动过
        assert_eq!(
            expand_image_path(r"C:\Windows\System32\svchost.exe -k netsvcs"),
            r"C:\Windows\System32\svchost.exe -k netsvcs"
        );
    }

    /// 真机烟雾测试：确认能读到本机的服务，且过滤后的规模合理。
    #[test]
    fn real_machine_scan_yields_plausible_set() {
        let items = collect().expect("服务扫描不应失败");

        println!("服务扫描：{} 项属于自启", items.len());

        // 几十到几百之间都正常。为 0 说明过滤条件写错了；
        // 超过 500 说明把非自启服务也收进来了。
        assert!(!items.is_empty(), "任何正常 Windows 都应扫到自动启动的服务");
        assert!(items.len() < 500, "规模异常，过滤条件可能失效");

        // 自动启动的服务必须带路径（否则后面的有效性检测没得判）
        let with_path = items.iter().filter(|i| !i.resolved_path.is_empty()).count();
        assert!(
            with_path * 2 > items.len(),
            "带路径的项不足一半，ImagePath 解析可能坏了"
        );

        for i in items.iter().take(8) {
            println!(
                "  {:<28} auto={:<5} phase={:?} target={}",
                i.name, i.enabled, i.boot_phase, i.resolved_path
            );
        }
    }
}
