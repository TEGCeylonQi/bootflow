//! 系统注入扫描器。
//!
//! 这一类和其他来源有本质区别：**它不是一次启动，而是一段被加载进别人的代码**。
//!
//! | 机制 | 位置 | 效果 |
//! |---|---|---|
//! | `AppInit_DLLs` | `NT\CurrentVersion\Windows` | 每个加载 user32.dll 的进程都会带上它 |
//! | `IFEO Debugger` | `Image File Execution Options\<exe>` | 某个程序被启动时，实际跑的是另一个程序 |
//! | `IFEO PerfOptions` | 同上 `\PerfOptions` | 某个程序被启动时，被赋予指定的 CPU/IO 优先级 |
//!
//! ## ⚠️ 两个必须真机验证才知道的坑
//!
//! **1. `AppInit_DLLs` 有值 ≠ 注入生效。**
//!
//! 它受一个独立的开关 `LoadAppInit_DLLs` 控制。本机实测：
//!
//! ```text
//! AppInit_DLLs       REG_SZ     C:\PROGRA~1\VIRTUA~2\VIRTUA~4.DLL
//! LoadAppInit_DLLs   REG_DWORD  0x0      ← 关闭
//! ```
//!
//! 这是 VB-Audio 虚拟声卡的 DLL，注入本身早就被关掉了。
//! 只看 `AppInit_DLLs` 非空就报"发现全局注入"，是一次彻头彻尾的误报——
//! 而且会让用户白白紧张一场（"有个来路不明的 DLL 注入我所有程序"）。
//!
//! **2. 这里的路径必须是 8.3 短名，而且这是设计使然。**
//!
//! `PROGRA~1` 不是"系统没写好"，而是**必须这样写**：
//! `AppInit_DLLs` 的值是**空格分隔**的列表，所以路径里不能有空格，
//! 安装程序只能用短名。这也意味着拆分它只需按空白切——不必担心
//! 某个路径内部的空格，因为那种路径根本不允许出现在这里。
//!
//! 展示时再展开成长名（`GetLongPathName` 语义），用户才看得懂。
//!
//! ## 只读
//!
//! 只 `open_subkey`，不出现任何 `set_value` / `create_subkey`。

use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY};
use winreg::RegKey;

use crate::model::{BootPhase, DiagnosticInfo, RiskLevel, SourceKind, StartupItem};
use crate::scanners::builder::ItemBuilder;

/// `AppInit_DLLs` 所在位置。32 位那份走注册表重定向，不写死 `WOW6432Node`。
const APPINIT_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows";

/// IFEO 所在位置。
const IFEO_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options";

/// 扫描全部系统注入项。
pub fn collect() -> Result<Vec<StartupItem>, String> {
    let mut out = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    match collect_appinit() {
        Ok(mut v) => out.append(&mut v),
        Err(e) => errors.push(format!("全局注入配置：{e}")),
    }

    match collect_ifeo() {
        Ok(mut v) => out.append(&mut v),
        Err(e) => errors.push(format!("程序启动改写：{e}")),
    }

    if out.is_empty() && !errors.is_empty() {
        return Err(errors.join("；"));
    }
    // 有结果时把部分失败记进日志即可——一个位置读不到不该让整类消失
    for e in &errors {
        log::warn!("系统注入扫描部分失败：{e}");
    }

    Ok(out)
}

/* ───────────────────── AppInit_DLLs ───────────────────── */

/// 「全局注入配置」的一次读取结果。
#[derive(Debug, Clone)]
struct AppInitEntry {
    dll: String,
    /// `LoadAppInit_DLLs`。`None` 表示注册表里没有这个值。
    load_enabled: Option<u32>,
    /// 这是 32 位视图那份吗
    is_32bit: bool,
}

impl AppInitEntry {
    /// 注入是否真的生效。
    ///
    /// 缺失 `LoadAppInit_DLLs` 值时必须按**关闭**处理：
    /// 该值不存在表示从未被显式开启过（Windows 8 起它的默认值就是 0，
    /// 且系统不再默认启用 AppInit）。反过来默认成"开启"，
    /// 会把所有残留配置都报成活跃注入。
    fn is_active(&self) -> bool {
        matches!(self.load_enabled, Some(v) if v != 0)
    }
}

fn collect_appinit() -> Result<Vec<StartupItem>, String> {
    let entries = read_appinit_entries()?;
    Ok(entries.iter().filter_map(build_appinit_item).collect())
}

/// 读两个视图的 `AppInit_DLLs`。
fn read_appinit_entries() -> Result<Vec<AppInitEntry>, String> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let mut out = Vec::new();
    let mut any_read = false;

    // 顺序：先 64 位，再 32 位。两个视图是不同的物理键
    // （32 位那份被重定向到 WOW6432Node），必须各读一次。
    for (flags, is_32bit) in [
        (KEY_READ, false),
        (KEY_READ | KEY_WOW64_32KEY, true),
    ] {
        let key = match hklm.open_subkey_with_flags(APPINIT_KEY, flags) {
            Ok(k) => k,
            // 键不存在是常态（绝大多数系统没配过），不是失败
            Err(_) => continue,
        };
        any_read = true;

        let raw: String = match key.get_value("AppInit_DLLs") {
            Ok(v) => v,
            Err(_) => continue,
        };

        let load_enabled: Option<u32> = key.get_value("LoadAppInit_DLLs").ok();

        // ⚠️ 这个值可能是**空格分隔**的多个 DLL。见模块头注释：
        // 路径里不允许有空格，所以按空白切是安全的。
        for dll in raw.split_whitespace() {
            let dll = dll.trim_matches(|c| c == ',' || c == ';');
            if dll.is_empty() {
                continue;
            }
            out.push(AppInitEntry {
                dll: dll.to_string(),
                load_enabled,
                is_32bit,
            });
        }
    }

    if !any_read {
        return Err("无法读取全局注入配置（键不存在或访问被拒绝）".to_string());
    }

    Ok(out)
}

fn build_appinit_item(entry: &AppInitEntry) -> Option<StartupItem> {
    let active = entry.is_active();

    // 短名 → 长名。用 `canonicalize` 而不是 GetLongPathName：
    // 它在路径不存在时返回 Err，我们正好需要"保留原值"这个行为
    // （注入的 DLL 被删掉时，如实显示注册表里的原样值）。
    let long = expand_short_path(&entry.dll);

    let file_name = long
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(&long)
        .to_string();

    let view = if entry.is_32bit { "32 位" } else { "64 位" };

    let mut builder = ItemBuilder::new(SourceKind::SystemHook, &file_name)
        .location(format!("系统全局注入 · AppInit_DLLs（{view}程序）"))
        .command(entry.dll.clone())
        .target(long)
        .enabled(active)
        .boot_phase(BootPhase::UserInit);

    // 这里**不设** display_name：让 builder 去读 DLL 的版本信息。
    // VB-Audio 的 DLL 有自己的 FileDescription，比文件名可读。

    if active {
        builder = builder.diagnostic(DiagnosticInfo {
            code: crate::diag::codes::GLOBAL_HOOK.to_string(),
            severity: RiskLevel::High,
            message: "这段代码不是一个独立程序，而是会被装进每一个带界面的程序里一起运行。\
                      它的影响范围是这台电脑上几乎所有软件——出问题时影响面也一样大。"
                .to_string(),
            evidence: Some(format!(
                "AppInit_DLLs = {}，LoadAppInit_DLLs = {}",
                entry.dll,
                entry.load_enabled.unwrap_or(0)
            )),
        });
    } else {
        // 本机实测就是这个状态：值还在，开关关着。
        // 报成"发现注入"是误报，会让用户白紧张；如实说明才对。
        builder = builder.diagnostic(DiagnosticInfo {
            code: crate::diag::codes::HOOK_NOT_ACTIVE.to_string(),
            severity: RiskLevel::Safe,
            message: "这里登记了一段要被装进其它程序的代码，但**注入开关是关闭的**，\
                      它现在不会生效。多半是某个程序卸载时留下没清干净的记录。"
                .to_string(),
            evidence: Some(format!(
                "AppInit_DLLs = {}，LoadAppInit_DLLs = {}（0 表示未启用）",
                entry.dll,
                entry.load_enabled.unwrap_or(0)
            )),
        });
    }

    let mut item = builder
        .raw(serde_json::json!({
            "kind": "appInitDlls",
            "appInitDlls": entry.dll,
            "loadAppInitDlls": entry.load_enabled,
            "view": view,
            "active": active,
        }))
        .build();

    item.summary = Some(if active {
        "它会被加载进几乎所有带界面的程序".to_string()
    } else {
        "这里有一段注入记录，但注入开关是关的，当前不生效".to_string()
    });

    // 注入项不以独立文件形式"启动"，所以不走目标存在性检查
    item.validity = crate::model::ValidityStatus::Unknown;
    item.validity_detail = None;

    Some(item)
}

/// 把 8.3 短名展开成长名。路径不存在时原样返回。
fn expand_short_path(path: &str) -> String {
    if !path.contains('~') {
        return path.to_string();
    }

    match std::fs::canonicalize(path) {
        Ok(p) => {
            let s = p.to_string_lossy().to_string();
            // canonicalize 在 Windows 上会加 `\\?\` 前缀（扩展长度路径语法），
            // 展示给用户要去掉——那不是路径的一部分。
            s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
        }
        Err(_) => path.to_string(),
    }
}

/* ───────────────────── IFEO ───────────────────── */

fn collect_ifeo() -> Result<Vec<StartupItem>, String> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let root = hklm
        .open_subkey_with_flags(IFEO_KEY, KEY_READ)
        .map_err(|e| format!("无法打开 Image File Execution Options：{e}"))?;

    let mut out = Vec::new();

    // ⚠️ 这里**只挑有两类值的子键**，不能见到子键就收。
    //
    // 本机实测有 65 个子键，绝大多数是 Windows 自己写的
    // （`MitigationOptions`、`MinimumStackCommitInBytes` 之类），
    // 它们既不改变程序行为也不值得用户知道。全部收进来会让
    // "程序启动改写"这个类别被 60 多条噪音填满，
    // 真正的那一条（如果存在）就永远找不到了。
    for name in root.enum_keys().flatten() {
        let Ok(sub) = root.open_subkey_with_flags(&name, KEY_READ) else {
            continue;
        };

        if let Ok(debugger) = sub.get_value::<String, _>("Debugger") {
            if !debugger.trim().is_empty() {
                out.push(build_ifeo_debugger_item(&name, &debugger));
            }
        }

        if let Ok(perf) = sub.open_subkey_with_flags("PerfOptions", KEY_READ) {
            if let Some(item) = build_ifeo_priority_item(&name, &perf) {
                out.push(item);
            }
        }
    }

    Ok(out)
}

/// `IFEO\...\<exe>\Debugger`：某个程序的启动被改写了。
fn build_ifeo_debugger_item(exe_name: &str, debugger: &str) -> StartupItem {
    let (target, args) = crate::util::cmdline::split_command(debugger);

    let mut item = ItemBuilder::new(SourceKind::SystemHook, exe_name)
        .location(format!("程序启动改写 · 当 {exe_name} 启动时"))
        .command(debugger)
        .target(target)
        .args(args)
        .enabled(true)
        .boot_phase(BootPhase::Unknown)
        .diagnostic(DiagnosticInfo {
            code: crate::diag::codes::IFEO_HIJACK.to_string(),
            severity: RiskLevel::High,
            message: format!(
                "有人改写了 {exe_name} 的启动方式：它被启动时，实际运行的是上面那个程序。\
                 调试工具与恶意软件都会用这个机制，所以值得你确认一下是不是自己装的。"
            ),
            evidence: Some(format!("Debugger = {debugger}")),
        })
        .raw(serde_json::json!({
            "kind": "ifeoDebugger",
            "targetExe": exe_name,
            "debugger": debugger,
        }))
        .build();

    item.summary = Some(format!("{exe_name} 的启动被指向了另一个程序"));
    item.validity = crate::model::ValidityStatus::Unknown;
    item
}

/// `IFEO\...\<exe>\PerfOptions`：某个程序被附加了优先级改写。
fn build_ifeo_priority_item(
    exe_name: &str,
    perf: &RegKey,
) -> Option<StartupItem> {
    let cpu: Option<u32> = perf.get_value("CpuPriorityClass").ok();
    let io: Option<u32> = perf.get_value("IoPriority").ok();
    let page: Option<u32> = perf.get_value("PagePriority").ok();

    if cpu.is_none() && io.is_none() && page.is_none() {
        return None;
    }

    // `CpuPriorityClass = 4` 是 RealTime。这是本项目的永久护栏之一：
    // 用户态程序占 Realtime 会抢在音频驱动之前拿到 CPU，
    // 直接后果是爆音与系统假死。所以它单独成一条、单独措辞。
    let is_realtime = cpu == Some(4);

    let cpu_phrase = match cpu {
        Some(1) => "最低（Idle）",
        Some(2) => "普通（Normal）",
        Some(3) => "高（High）",
        Some(4) => "实时（RealTime）",
        Some(_) => "未知档位",
        None => "未设置",
    };

    let (severity, message) = if is_realtime {
        (
            RiskLevel::High,
            format!(
                "{exe_name} 被设成了「实时」优先级。这一档会让它抢在所有程序——\
                 包括声卡驱动——之前占用处理器，典型后果是音频爆音、鼠标卡顿，\
                 严重时整个系统失去响应。这是所有优先级设置里唯一不建议使用的一档。"
            ),
        )
    } else {
        (
            RiskLevel::Medium,
            format!(
                "{exe_name} 每次启动时都会被自动设成「{cpu_phrase}」优先级。\
                 这通常是为了让它跑得更顺（或者更不打扰），但如果不是你特意设的，\
                 建议确认一下来源。"
            ),
        )
    };

    let mut item = ItemBuilder::new(SourceKind::SystemHook, exe_name)
        .location(format!("程序启动改写 · {exe_name} 的优先级"))
        .command(exe_name)
        .enabled(true)
        .boot_phase(BootPhase::Unknown)
        .diagnostic(DiagnosticInfo {
            code: crate::diag::codes::IFEO_PRIORITY.to_string(),
            severity,
            message,
            evidence: Some(format!(
                "CpuPriorityClass={}，IoPriority={}，PagePriority={}",
                cpu.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
                io.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
                page.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
            )),
        })
        .raw(serde_json::json!({
            "kind": "ifeoPerfOptions",
            "targetExe": exe_name,
            "cpuPriorityClass": cpu,
            "ioPriority": io,
            "pagePriority": page,
        }))
        .build();

    item.summary = Some(format!("启动 {exe_name} 时会被自动设为{cpu_phrase}优先级"));
    item.validity = crate::model::ValidityStatus::Unknown;
    Some(item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(load: Option<u32>) -> AppInitEntry {
        AppInitEntry {
            dll: r"C:\PROGRA~1\VIRTUA~2\VIRTUA~4.DLL".into(),
            load_enabled: load,
            is_32bit: false,
        }
    }

    /// 本机实测状态的回归锁：值在、开关关着。
    #[test]
    fn appinit_with_switch_off_is_not_active() {
        assert!(
            !entry(Some(0)).is_active(),
            "LoadAppInit_DLLs=0 时必须判为未生效——否则就是一次误报"
        );
    }

    #[test]
    fn missing_load_value_defaults_to_inactive() {
        // 值不存在 = 从未显式开启过。默认成"已开启"会把所有残留配置
        // 都报成活跃注入。
        assert!(!entry(None).is_active());
    }

    #[test]
    fn appinit_switch_on_is_active() {
        assert!(entry(Some(1)).is_active());
    }

    #[test]
    fn inactive_appinit_reports_safe_not_high() {
        let item = build_appinit_item(&entry(Some(0))).unwrap();

        let d = &item.diagnostics[0];
        assert_eq!(d.code, crate::diag::codes::HOOK_NOT_ACTIVE);
        assert_eq!(d.severity, RiskLevel::Safe, "未生效的注入不该报成高危");

        // 证据里必须带上开关的实际值，用户才能自己复核
        assert!(d.evidence.as_deref().unwrap().contains("LoadAppInit_DLLs = 0"));
        assert!(!item.enabled);
    }

    #[test]
    fn active_appinit_reports_high_with_human_wording() {
        let item = build_appinit_item(&entry(Some(1))).unwrap();

        let d = &item.diagnostics[0];
        assert_eq!(d.code, crate::diag::codes::GLOBAL_HOOK);
        assert_eq!(d.severity, RiskLevel::High);
        assert!(item.enabled);
        // 措辞不能吓人也不能淡化：说清"影响范围大"，不说"发现病毒"
        assert!(d.message.contains("影响范围"));
        for term in ["病毒", "恶意软件", "感染"] {
            assert!(!d.message.contains(term), "不该出现 {term} 这类断言");
        }
    }

    #[test]
    fn appinit_items_are_hooks_not_apps() {
        let item = build_appinit_item(&entry(Some(1))).unwrap();
        assert_eq!(item.source, SourceKind::SystemHook);
        assert_eq!(
            item.kind,
            crate::model::ItemKind::Hook,
            "注入项必须归类为「系统注入」，而不是普通应用"
        );
    }

    #[test]
    fn short_path_expansion_keeps_missing_paths_verbatim() {
        // 注入的 DLL 被删掉时，要如实显示注册表里的原样值，
        // 而不是显示一个空的或编出来的长路径
        let ghost = r"C:\__bootflow_not_here__\X~1.DLL";
        assert_eq!(expand_short_path(ghost), ghost);

        // 不含波浪号的路径原样返回，不做任何处理
        assert_eq!(expand_short_path(r"C:\Windows\a.dll"), r"C:\Windows\a.dll");
    }

    #[test]
    fn short_path_expansion_works_on_a_real_file() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let short = format!(r"{root}\SYSTEM~1\notepad.exe");
        let long = expand_short_path(&short);

        // 不是所有系统都启用了 8.3 名称生成，能展开就验证一下内容
        if long != short {
            assert!(!long.contains("~"), "展开后不该还留着短名：{long}");
        }
    }

    /// 真机烟雾测试。本机的实际状态是「有 AppInit 配置但开关关闭」，
    /// 这条测试用来确认我们**不会**把它报成活跃注入。
    #[test]
    fn real_machine_appinit_is_read_and_classified() {
        let entries = read_appinit_entries().expect("应能读取 AppInit 配置");

        for e in &entries {
            println!(
                "AppInit：{} view={} load={:?} active={}",
                e.dll,
                if e.is_32bit { "32" } else { "64" },
                e.load_enabled,
                e.is_active()
            );
        }

        // 读到了就必须至少产出一条项；没读到（干净系统）也是合法结果
        let items = collect_appinit().expect("AppInit 扫描不应失败");
        assert_eq!(items.len(), entries.len());
    }

    #[test]
    fn real_machine_ifeo_scan_does_not_fabricate_items() {
        let items = collect_ifeo().expect("IFEO 扫描不应失败");

        println!("IFEO 改写项：{} 条", items.len());

        // 本机实测 65 个子键里 0 个 Debugger、0 个 PerfOptions，
        // 所以这里必须是空的。若不为空，说明我们把系统自带的
        // 无害子键（MitigationOptions 之类）误收进来了。
        assert!(
            items.is_empty(),
            "本机没有 IFEO 改写，扫描结果却是 {} 条——过滤条件可能过宽",
            items.len()
        );
    }

    #[test]
    fn real_machine_scan_returns_without_panicking() {
        let items = collect().expect("系统注入扫描不应整体失败");
        for i in &items {
            assert!(!i.name.trim().is_empty(), "项必须有名字");
            assert_eq!(i.source, SourceKind::SystemHook);
        }
        println!("系统注入项合计 {} 条", items.len());
    }
}
