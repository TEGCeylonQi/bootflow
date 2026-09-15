//! 注册表 `Run` / `RunOnce` 扫描器。
//!
//! 这是**数量最多、命名最乱、坑也最多**的一个来源。单独一个扫描器的理由
//! 不是"它位置不同"，而是它的每一处都和前一个来源不一样：
//!
//! | 坑 | 如果不管会怎样 | 这里怎么处理 |
//! |---|---|---|
//! | 值类型不只有 `REG_SZ` | `REG_EXPAND_SZ` 里的 `%USERPROFILE%` 不展开就找不到目标 | 按类型分别解码，字符串统一走 `expand_env` |
//! | 32 位程序写在 `WOW6432Node` 下 | 64 位进程读 `Software\...\Run` **根本读不到它们**，界面凭空少一批 | 显式用 `KEY_WOW64_32KEY` 再读一遍 |
//! | 值可能只是裸程序名 | `rundll32.exe xxx.dll,...` 按路径判存在性 → 全部误报"程序已卸载" | 按系统搜索顺序补全路径 |
//! | 值可能指向 `.lnk` | 只知道快捷方式文件名，拿不到目标与图标 | 复用 `util::link` |
//! | 从任务管理器禁用后值仍在 | 把已停用的当成在跑的，开机负担被高估 | 查 `StartupApproved` |
//!
//! ⚠️ 只读。本模块不出现任何写注册表的代码路径。
//!
//! **不扫描**的位置及理由（写在这里避免以后有人以为是漏了）：
//! - `RunOnceEx`：子键结构，值的语法是 `|` 分隔的多段命令，语义与 Run 完全不同，
//!   按 Run 的方式解析会得到一堆假项。等 v1.5 单独做。
//! - `Run` 键下的**子键**：确实可能被安装程序创建，但约定不统一
//!   （有的是目录式组织，有的干脆是误写），无法可靠解析，只记日志。

use winreg::enums::{
    RegType, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
};
use winreg::RegKey;

use crate::model::{BootPhase, Scope, SourceKind, StartupItem};
use crate::scanners::builder::{approved_enabled, ItemBuilder};
use crate::util::{cmdline, link};

// ─────────────────────────── 键位置表 ───────────────────────────

/// 注册表视图。**这是这个扫描器最容易出错的地方**，所以显式建模。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    /// 不指定视图（用当前进程的默认视图）。`HKCU\Software` 下的 Run 无重定向，
    /// 32/64 位进程读到的是同一个物理键，不需要读两遍。
    Native,
    /// 强制 64 位视图
    Wow64_64,
    /// 强制 32 位视图 —— 会被系统重定向到 `WOW6432Node`
    Wow64_32,
}

impl View {
    fn flags(self) -> u32 {
        match self {
            View::Native => KEY_READ,
            View::Wow64_64 => KEY_READ | KEY_WOW64_64KEY,
            View::Wow64_32 => KEY_READ | KEY_WOW64_32KEY,
        }
    }

    /// 写进 `raw`，属性面板的技术详情会展示
    fn label(self) -> &'static str {
        match self {
            View::Native => "native",
            View::Wow64_64 => "64",
            View::Wow64_32 => "32",
        }
    }
}

/// 一个要扫描的键位置。
struct KeySpec {
    /// 是否 HKLM（决定 hive 与默认 scope）
    machine: bool,
    /// 打开时用的键路径
    path: &'static str,
    source: SourceKind,
    view: View,
    /// 展示给用户的位置。32 位视图写成它**真实所在的物理路径**
    /// （`WOW6432Node`），比写"32 位视图"更容易对上用户自己打开注册表时看到的东西。
    display: &'static str,
    /// 是否来自组策略
    policy: bool,
}

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_ONCE: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
const POLICY_RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run";

/// 全部待扫描位置。
///
/// 顺序即用户看到的顺序：先是当前用户的，再是全机的；先是常驻的 Run，
/// 再是只跑一次的 RunOnce。32 位视图紧跟对应的 64 位视图，
/// 这样同类型的项在列表里是挨着的，不像"散落各处"。
const KEYS: &[KeySpec] = &[
    KeySpec {
        machine: false,
        path: RUN,
        source: SourceKind::RunUser,
        view: View::Native,
        display: r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
        policy: false,
    },
    KeySpec {
        machine: false,
        path: RUN_ONCE,
        source: SourceKind::RunOnceUser,
        view: View::Native,
        display: r"HKCU\Software\Microsoft\Windows\CurrentVersion\RunOnce",
        policy: false,
    },
    KeySpec {
        machine: true,
        path: RUN,
        source: SourceKind::RunMachine,
        view: View::Wow64_64,
        display: r"HKLM\Software\Microsoft\Windows\CurrentVersion\Run",
        policy: false,
    },
    KeySpec {
        machine: true,
        path: RUN,
        source: SourceKind::RunMachine32,
        view: View::Wow64_32,
        display: r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run",
        policy: false,
    },
    KeySpec {
        machine: true,
        path: RUN_ONCE,
        source: SourceKind::RunOnceMachine,
        view: View::Wow64_64,
        display: r"HKLM\Software\Microsoft\Windows\CurrentVersion\RunOnce",
        policy: false,
    },
    KeySpec {
        machine: true,
        path: RUN_ONCE,
        source: SourceKind::RunOnceMachine32,
        view: View::Wow64_32,
        display: r"HKLM\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\RunOnce",
        policy: false,
    },
    // 下面两处是**物理**的 Wow6432Node 路径。HKCU 的 Run 本来没有重定向，
    // 但少数 32 位安装程序会自己硬写这个路径——它们写进去的东西
    // 系统同样会执行，不扫就会漏。
    KeySpec {
        machine: false,
        path: r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run",
        source: SourceKind::RunUser,
        view: View::Native,
        display: r"HKCU\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run",
        policy: false,
    },
    KeySpec {
        machine: false,
        path: r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\RunOnce",
        source: SourceKind::RunOnceUser,
        view: View::Native,
        display: r"HKCU\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\RunOnce",
        policy: false,
    },
    // 组策略下发的位置。管理员推的策略项在这里，用户平时看不到，
    // 却实实在在会在登录时执行——正是"我的开机项列表里怎么多出来一个"的常见谜底。
    KeySpec {
        machine: false,
        path: POLICY_RUN,
        source: SourceKind::RunUser,
        view: View::Native,
        display: r"HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run",
        policy: true,
    },
    KeySpec {
        machine: true,
        path: POLICY_RUN,
        source: SourceKind::RunMachine,
        view: View::Wow64_64,
        display: r"HKLM\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run",
        policy: true,
    },
];

// ─────────────────────────── 值解码 ───────────────────────────

/// 解码结果。
enum Decoded {
    /// 一条可执行的命令行 + 值类型名（原样展示给用户）
    Command(String, &'static str),
    /// 类型不对，不可能是启动项
    NotACommand(&'static str),
}

/// 注册表字符串值的字节解码。
///
/// 三个必须处理的边界，都是真机上会遇到脏数据的地方：
/// 1. **不保证以 NUL 结尾**——按 NUL 结尾处理会把最后一个字符吃掉。
/// 2. **可能带多个尾随 NUL**——`REG_MULTI_SZ` 或程序写脏了都会这样。
/// 3. **字节数可能是奇数**——用 `from_utf16` 会直接报错。用 `chunks_exact(2)`
///    丢掉最后一个不完整字节，比整条命令解析失败要好。
fn utf16_lossy(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    String::from_utf16_lossy(&units)
}

/// 清理字符串值两端：去尾随 NUL、去首尾空白。
fn clean(s: &str) -> String {
    s.trim_end_matches('\0').trim().to_string()
}

/// 按值类型解码出一条命令行。
fn decode_command(bytes: &[u8], vtype: RegType) -> Decoded {
    match vtype {
        RegType::REG_SZ => Decoded::Command(clean(&utf16_lossy(bytes)), "REG_SZ"),

        // 与 REG_SZ 的区别是里面可能含 `%VAR%`。这里**不展开**——
        // 展开统一由 `cmdline::split_command` 做，那样只有一处实现。
        RegType::REG_EXPAND_SZ => {
            Decoded::Command(clean(&utf16_lossy(bytes)), "REG_EXPAND_SZ")
        }

        // 多字符串在 Run 键下极罕见，但确实存在（老安装程序写的）。
        // Run 的语义是"执行一条命令"，所以取第一条非空条目，
        // 同时把类型名原样带出去，让用户在技术详情里能看到这个异常。
        RegType::REG_MULTI_SZ => {
            let joined = utf16_lossy(bytes);
            match joined
                .split('\0')
                .map(clean)
                .find(|s| !s.is_empty())
            {
                Some(first) => Decoded::Command(first, "REG_MULTI_SZ"),
                None => Decoded::NotACommand("REG_MULTI_SZ（内容为空）"),
            }
        }

        RegType::REG_DWORD => Decoded::NotACommand("REG_DWORD"),
        RegType::REG_QWORD => Decoded::NotACommand("REG_QWORD"),
        RegType::REG_BINARY => Decoded::NotACommand("REG_BINARY"),
        RegType::REG_NONE => Decoded::NotACommand("REG_NONE"),
        _ => Decoded::NotACommand("未知类型"),
    }
}

// ─────────────────────────── 扫描入口 ───────────────────────────

/// 扫描所有 Run / RunOnce 位置。
///
/// 返回 `Err` 只在**一个键都读不到**时发生。单个键不存在（精简系统、
/// 组策略键从未被创建过）是常态，不算失败；单个键拒绝访问则记进错误列表
/// 照常返回其他位置的结果——**绝不能因为一处读不到就让整个界面空掉**。
pub fn collect() -> Result<Vec<StartupItem>, String> {
    let mut items = Vec::new();
    let mut errors = Vec::new();
    let mut readable = 0usize;

    for spec in KEYS {
        match scan_key(spec) {
            Ok(Some(mut v)) => {
                readable += 1;
                items.append(&mut v);
            }
            // 键不存在：正常情况
            Ok(None) => {}
            Err(e) => errors.push(format!("{}：{e}", spec.display)),
        }
    }

    if readable == 0 && !errors.is_empty() {
        return Err(errors.join("；"));
    }

    for e in &errors {
        log::warn!("Run 键部分失败：{e}");
    }

    log::info!(
        "Run 键：扫出 {} 项（{} 个位置可读，{} 个位置读取失败）",
        items.len(),
        readable,
        errors.len()
    );

    Ok(items)
}

/// 扫描单个键位置。
///
/// - `Ok(Some(items))` —— 键存在，读了（items 可能为空，说明键下没有值）
/// - `Ok(None)` —— 键不存在
/// - `Err(..)` —— 键存在但读不了（典型是拒绝访问）
fn scan_key(spec: &KeySpec) -> Result<Option<Vec<StartupItem>>, String> {
    let root = if spec.machine {
        RegKey::predef(HKEY_LOCAL_MACHINE)
    } else {
        RegKey::predef(HKEY_CURRENT_USER)
    };

    let key = match root.open_subkey_with_flags(spec.path, spec.view.flags()) {
        Ok(k) => k,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(describe_open_error(&e)),
    };

    // 子键不扫描，但值得记一笔：万一某个用户"明明有这一项却没显示出来"，
    // 日志里能查到线索，而不是让人怀疑扫描器坏了
    match key.enum_keys().count() {
        0 => {}
        n => log::info!(
            "{} 下存在 {} 个子键，本版本不扫描子键（结构约定不统一）",
            spec.display,
            n
        ),
    }

    let mut out = Vec::new();

    for entry in key.enum_values() {
        let (value_name, value) = match entry {
            Ok(pair) => pair,
            Err(e) => {
                // 枚举中途出错不应该让整个键放弃——已经读到的项照常返回
                log::warn!("{} 下有值读取失败：{e}", spec.display);
                continue;
            }
        };

        let (command, value_type) = match decode_command(&value.bytes, value.vtype) {
            Decoded::Command(c, t) => (c, t),
            Decoded::NotACommand(t) => {
                // 类型都不对的东西不可能是启动项。纳入结果只会让用户困惑，
                // "这里有个 REG_DWORD 的启动项"是没法解释的。
                log::info!("{} 下的「{value_name}」是 {t}，不是命令，跳过", spec.display);
                continue;
            }
        };

        if command.is_empty() {
            // 值被清空了。它启动不了任何东西，但**用户可能在别处看到过它**，
            // 与其让它神秘消失，不如收进来由有效性检测给出结论。
            log::info!("{} 下的「{value_name}」是空值", spec.display);
        }

        // RunOnce 里值内容为 `-` 是系统的约定：表示"取消这条挂起的记录"，
        // 不是要执行的命令。当成启动项收进去会得到一个永远"目标不存在"的假警报。
        if is_run_once(spec.source) && command.trim() == "-" {
            log::info!("{} 下的「{value_name}」是取消标记，跳过", spec.display);
            continue;
        }

        out.push(build_item(spec, &value_name, &command, value_type));
    }

    Ok(Some(out))
}

fn is_run_once(source: SourceKind) -> bool {
    matches!(
        source,
        SourceKind::RunOnceUser | SourceKind::RunOnceMachine | SourceKind::RunOnceMachine32
    )
}

/// 把"拒绝访问"翻译成用户能懂的话。
///
/// 裸的 `Access is denied. (os error 5)` 出现在界面上毫无意义——
/// 用户既不知道哪个键，也不知道该怎么办。
fn describe_open_error(e: &std::io::Error) -> String {
    const ERROR_ACCESS_DENIED: i32 = 5;
    match e.raw_os_error() {
        Some(ERROR_ACCESS_DENIED) => "拒绝访问（该位置需要管理员权限才能查看）".to_string(),
        _ => e.to_string(),
    }
}

/// 构造一个启动项。
fn build_item(spec: &KeySpec, value_name: &str, command: &str, value_type: &str) -> StartupItem {
    let (target_raw, args_raw) = cmdline::split_command(command);

    // ── 裸程序名补全 ──
    // `rundll32.exe xxx.dll,Entry` 这种值的"路径"只是程序名，
    // 直接判断存在性会得到"程序已被卸载"——而用户明明还装着 Office。
    // 按系统搜索顺序找一遍；找不到就保留原名，让有效性检测如实报出来。
    let target_resolved = if !target_raw.is_empty() && !cmdline::has_path_separator(&target_raw) {
        cmdline::resolve_via_search_path(&target_raw).unwrap_or(target_raw)
    } else {
        target_raw
    };

    // ── 指向 .lnk 的情况 ──
    // Run 值写成快捷方式路径确实存在（安装程序图省事）。不解析的话
    // 拿不到目标、图标与版本信息，界面上只会显示一个莫名的文件名。
    let (target, args) = if link::is_shortcut(&target_resolved) {
        match link::resolve(&target_resolved) {
            Some(sc) => {
                // 快捷方式自带参数在前，Run 值里手写的参数在后——
                // 后者通常是对整条命令的补充
                let mut merged = sc.args;
                merged.extend(args_raw);
                (sc.target, merged)
            }
            // 解析不了就保留原路径：至少让用户看到它指向哪个快捷方式，
            // 而不是凭空消失
            None => (target_resolved, args_raw),
        }
    } else {
        (target_resolved, args_raw)
    };

    // 查询启动状态**必须用原始值名**：`StartupApproved` 里存的键名与 Run 键下的
    // 值名逐字节相同。本机实测就有 `" QQPCTray"` 这样带前导空格的值名，
    // trim 过再去查会查不到记录，于是"已被用户禁用"会被误报成"启用中"。
    let enabled = approved_enabled(spec.source, value_name);

    let raw = serde_json::json!({
        "kind": if spec.policy { "registryPolicyRun" } else { "registryRun" },
        "hive": if spec.machine { "HKLM" } else { "HKCU" },
        "keyPath": spec.path,
        "keyDisplay": spec.display,
        // 原样保留（含可能的空格），写操作与回滚审计要靠它精确定位
        "valueName": value_name,
        "valueType": value_type,
        "view": spec.view.label(),
        "runOnce": is_run_once(spec.source),
        "policy": spec.policy,
        "approvedEnabled": enabled,
    });

    // `name` 是给界面回退显示用的，去掉首尾空白；
    // 但 raw.valueName 保持原样（见上）。
    let display_key = value_name.trim();

    ItemBuilder::new(spec.source, display_key)
        .scope(if spec.machine {
            Scope::Machine
        } else {
            Scope::User
        })
        .location(spec.display)
        .command(command)
        .target(target)
        .args(args)
        .enabled(enabled)
        // Run 与 RunOnce 都在用户登录过程中被拉起：常驻项在 shell 初始化后，
        // 全机项在登录更早的阶段。两者对用户而言都是"登录时"，不细分。
        .boot_phase(BootPhase::Logon)
        .raw(raw)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd_of(bytes: &[u8], t: RegType) -> Option<String> {
        match decode_command(bytes, t) {
            Decoded::Command(c, _) => Some(c),
            Decoded::NotACommand(_) => None,
        }
    }

    /// 把 `&str` 编成不带结尾 NUL 的 UTF-16LE 字节。
    /// **刻意不带 NUL**：真实注册表里不保证有，代码必须能处理。
    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    #[test]
    fn decodes_sz_without_trailing_nul() {
        assert_eq!(
            cmd_of(&utf16(r"C:\App\a.exe"), RegType::REG_SZ).unwrap(),
            r"C:\App\a.exe",
            "值不一定以 NUL 结尾，不能靠 NUL 定界"
        );
    }

    #[test]
    fn strips_multiple_trailing_nuls() {
        let mut bytes = utf16("a.exe");
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(cmd_of(&bytes, RegType::REG_SZ).unwrap(), "a.exe");
    }

    #[test]
    fn odd_byte_length_does_not_panic() {
        // 最后一个字节不完整：丢掉它，返回前半段，而不是整条解析失败
        let mut bytes = utf16("a.exe");
        bytes.push(0x41);
        assert_eq!(cmd_of(&bytes, RegType::REG_SZ).unwrap(), "a.exe");
    }

    #[test]
    fn expand_sz_is_kept_raw_and_typed() {
        let bytes = utf16(r"%ProgramFiles%\App\a.exe");
        match decode_command(&bytes, RegType::REG_EXPAND_SZ) {
            Decoded::Command(c, t) => {
                assert_eq!(c, r"%ProgramFiles%\App\a.exe", "展开交给 cmdline 统一处理");
                assert_eq!(t, "REG_EXPAND_SZ");
            }
            _ => panic!("应解码为命令"),
        }
    }

    #[test]
    fn multi_sz_takes_first_non_empty_entry() {
        let mut bytes = utf16("\0first.exe\0second.exe");
        bytes.extend_from_slice(&[0, 0]);
        assert_eq!(cmd_of(&bytes, RegType::REG_MULTI_SZ).unwrap(), "first.exe");
    }

    #[test]
    fn non_string_types_are_rejected() {
        // 这几个类型不可能是启动项，纳入结果只会让用户困惑
        assert!(cmd_of(&[1, 0, 0, 0], RegType::REG_DWORD).is_none());
        assert!(cmd_of(&[0xFF; 8], RegType::REG_BINARY).is_none());
        assert!(cmd_of(&[], RegType::REG_NONE).is_none());
    }

    #[test]
    fn key_table_covers_all_six_run_sources() {
        // 少扫一个视图，界面上就会凭空少一批项，而用户无从察觉——
        // 所以这里把"六个来源一个都不能少"钉死
        for want in [
            SourceKind::RunUser,
            SourceKind::RunOnceUser,
            SourceKind::RunMachine,
            SourceKind::RunMachine32,
            SourceKind::RunOnceMachine,
            SourceKind::RunOnceMachine32,
        ] {
            assert!(
                KEYS.iter().any(|k| k.source == want),
                "键位置表缺少 {want:?} 的扫描位置"
            );
        }
    }

    #[test]
    fn key_table_has_no_duplicate_locations() {
        // 同一个物理键被扫两遍会产出两条一模一样的项（去重只能合掉重名的那些），
        // 表现为界面上凭空多出一倍的条数
        let mut seen = std::collections::HashSet::new();
        for k in KEYS {
            let id = format!("{}{}{}", k.machine, k.path, k.view.label());
            assert!(seen.insert(id), "位置表里有重复项：{}", k.display);
        }
    }

    #[test]
    fn every_spec_display_starts_with_its_hive() {
        // display 是给用户看的注册表路径，写错了用户照着打开注册表会找不到
        for k in KEYS {
            let want = if k.machine { "HKLM\\" } else { "HKCU\\" };
            assert!(
                k.display.starts_with(want),
                "{} 的前缀应是 {want}",
                k.display
            );
        }
    }

    #[test]
    fn collect_never_panics_and_items_look_sane() {
        match collect() {
            Ok(items) => {
                for it in items {
                    assert!(!it.name.is_empty(), "值名不应为空");
                    // 名字为空说明注册表里有空值名——Windows 不允许，
                    // 真出现了说明我们的枚举出了问题
                    assert!(
                        !it.location.is_empty(),
                        "「{}」缺少位置信息",
                        it.name
                    );
                    assert!(
                        it.location.starts_with("HKCU\\") || it.location.starts_with("HKLM\\"),
                        "「{}」的位置不像注册表路径：{}",
                        it.name,
                        it.location
                    );
                }
            }
            Err(e) => panic!("Run 键扫描不应整体失败：{e}"),
        }
    }

    #[test]
    fn bare_command_names_are_resolved_to_real_paths() {
        let items = collect().unwrap_or_default();

        for it in items {
            let raw_name = it
                .raw
                .get("valueName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // 只看那些真的解析出了目标的项——解析失败的会走有效性检测报出来
            if it.resolved_path.is_empty() || it.validity != crate::model::ValidityStatus::Ok {
                continue;
            }

            assert!(
                cmdline::has_path_separator(&it.resolved_path),
                "「{raw_name}」解析出的目标仍是裸程序名 `{}`，说明没有走搜索路径补全",
                it.resolved_path
            );
        }
    }

    #[test]
    fn value_name_whitespace_is_trimmed_for_display_but_kept_raw() {
        // 本机实测存在 `" QQPCTray"` 这种带前导空格的值名（腾讯电脑管家写的）。
        // 展示名要干净，但原始值名必须逐字节保留——它同时是
        // StartupApproved 的查询键和将来写操作的定位键。
        let spec = &KEYS[0];
        let it = build_item(spec, " QQPCTray", r"C:\Windows\notepad.exe", "REG_SZ");

        assert_eq!(it.name, "QQPCTray", "展示名不应带前导空格");
        assert_eq!(
            it.raw.get("valueName").and_then(|v| v.as_str()),
            Some(" QQPCTray"),
            "原始值名必须原样保留，否则查不到 StartupApproved 记录"
        );
    }

    #[test]
    fn run_once_sources_are_recognised() {
        assert!(is_run_once(SourceKind::RunOnceUser));
        assert!(is_run_once(SourceKind::RunOnceMachine));
        assert!(is_run_once(SourceKind::RunOnceMachine32));
        // Run 是常驻的，不能和 RunOnce 混为一谈——后者跑一次就没了
        assert!(!is_run_once(SourceKind::RunUser));
        assert!(!is_run_once(SourceKind::RunMachine));
        assert!(!is_run_once(SourceKind::RunMachine32));
    }

    /// 诊断用：把这台机器上 Run 键的实际内容打出来，并逐条交代解析链路。
    ///
    /// 不断言数量（那会绑死在具体机器上），但它是验证"值解码 → 命令行拆分 →
    /// 裸名补全 → 签名"整条链是否真的可用的最快方式：
    /// `cargo test report_registry_run -- --nocapture`
    #[test]
    fn report_registry_run_items() {
        let mut items = collect().unwrap_or_default();

        // 关键：这里跑**完整管线**而不只是 collect()。
        // 去重（哪些被判重复、代表项是谁）和建议（给什么动作、什么措辞）
        // 都发生在管线里，只跑 collect 看不到，也就验证不了。
        let stats = crate::pipeline::finalize(
            &mut items,
            &crate::model::BootTimeline::default(),
            &crate::diag::proc_snapshot::Snapshot::default(),
            // 这条测试只关心 Run 键的解析链，不引入 WDI 那条需要提权、
            // 且随机器变化的数据通路——否则它会变成一条看天吃饭的测试。
            &crate::diag::startup_info::StartupInfoReport::default(),
            &mut crate::model::ImpactOverview::default(),
        );

        println!(
            "\nRun / RunOnce 共 {} 项；失效 {}，重复 {}，给出建议 {}",
            stats.total,
            stats.dead + stats.not_executable,
            stats.duplicates,
            stats.advised
        );

        for it in &items {
            let view = it
                .raw
                .get("view")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let vtype = it
                .raw
                .get("valueType")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let policy = it
                .raw
                .get("policy")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let policy_tag = if policy { " · 组策略下发" } else { "" };

            println!(
                "\n  [{:?}] 值名 = {} ({vtype}){policy_tag}",
                it.source, it.name
            );
            println!("      注册表视图 = {view}");
            println!(
                "      显示名 = {}  启用 = {}",
                it.display_name.as_deref().unwrap_or("(无版本信息)"),
                it.enabled
            );
            println!("      命令   = {}", it.command);
            println!("      目标   = {}", it.resolved_path);
            if !it.args.is_empty() {
                println!("      参数   = {:?}", it.args);
            }
            println!(
                "      发布者 = {}  有效性 = {:?}",
                it.signer.publisher.as_deref().unwrap_or("-"),
                it.validity
            );

            if let Some(r) = &it.recommendation {
                println!("      建议   = {:?}（{:?}）", r.action, r.confidence);
                println!("      理由   = {}", r.reason);
            }
            if let Some(d) = &it.duplicate_of {
                // 只打前 8 位，完整 uuid 会把输出撑得很长
                println!("      重复于 = {}…", &d[..8.min(d.len())]);
            }
        }
    }
}
