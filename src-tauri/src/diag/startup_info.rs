//! WDI `StartupInfo` —— **Windows 自己为每一项记的开销数据**。
//!
//! ## 这是「为什么拆不到单项」这个问题真正的答案
//!
//! 前几轮的结论是"Windows 没有单项耗时计时器"，那只说对了一半。
//! 准确的表述是：**没有单项的墙钟耗时，但有一项一项实测出来的资源开销。**
//!
//! Windows 诊断基础架构（WDI）每次登录都会跑一段引导跟踪，然后把结果落成
//! 本机 XML：
//!
//! ```text
//! %WINDIR%\System32\WDI\LogFiles\StartupInfo\<用户SID>_StartupInfo<1-5>.xml
//! ```
//!
//! 每个 `<Process>` 节点就是一个进程，带：
//!
//! | 字段 | 含义 |
//! |---|---|
//! | `Name` / `PID` | 映像完整路径、进程号 |
//! | `StartedInTraceSec` | 它在跟踪窗口里的第几秒被拉起 |
//! | `CpuUsage`（`Units="us"`） | 窗口内消耗的 CPU 时间，**微秒** |
//! | `DiskUsage`（`Units="bytes"`） | 窗口内读写磁盘的字节数 |
//! | `CommandLine` / `ParentPID` / `ParentName` | 命令行与父子关系 |
//!
//! **任务管理器「启动应用」页里的「启动影响」列，读的就是这份文件**
//! （微软公开口径：High = CPU > 1 秒或磁盘 > 3 MB；Medium = 300ms–1s 或
//! 300KB–3MB；Low = 更低）。这意味着我们的数字**用户能自己打开任务管理器
//! 逐条对照**——这是整个项目里最容易被验证、也最该被验证的一份数据。
//!
//! ## 它**不是**什么（界面文案必须跟着这几条说）
//!
//! * **CPU 时间不是墙钟耗时。** 多线程程序的 CPU 时间会跨核累加，
//!   所以一项的 CPU 时间可以超过窗口本身的长度。它是"占用了多少算力"，
//!   不是"它让开机慢了几秒"。微软自己也不管它叫耗时，叫 **impact（影响）**。
//! * **只有约 90 秒的登录窗口。** 开机早期启动的服务、驱动根本不在里面；
//!   有些版本的文件名/字段也不同（`CpuUsage` 大小写、单位属性都可能变），
//!   所以解析全程容错，缺字段不崩、不猜。
//! * **按用户 SID 分文件。** 别的账户的登录记录不能算在当前用户头上，
//!   所以优先取当前用户的 SID；只能退到别人的记录时，界面必须说明。
//! * **需要管理员权限**（该目录普通用户连列目录都被拒）。
//!   读不到就是读不到——这一条通路失败不影响其余三条。
//!
//! ## 与进程快照的分工
//!
//! | 通路 | 量 | 需要提权 | 覆盖范围 |
//! |---|---|---|---|
//! | `proc_snapshot` | 进程创建时刻（相对内核启动） | 否 | 所有进程 |
//! | `startup_info` | CPU 时间 / 磁盘字节（限登录窗口） | 是 | 登录期拉起的进程 |
//!
//! 两者都有时，时间轴用快照定位、"谁重"用这份数据——它们回答的不是同一个问题。

use serde::{Deserialize, Serialize};

use crate::model::ImpactLevel;

/// 一条 WDI 记录（对应一个进程实例）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupInfoRecord {
    /// 可执行文件名小写（如 `chrome.exe`）。归因时按它对应启动项。
    pub exe: String,
    /// 映像完整路径（保留原样，供证据展示与人工复核）。
    pub image: String,
    /// 进程号。同一个 exe 多次出现时靠它区分。
    pub pid: u32,
    /// 它在跟踪窗口里的第几秒被拉起（Windows 记的）。
    pub started_in_trace_ms: Option<u64>,
    /// 窗口内消耗的 CPU 时间（毫秒）。原值单位是微秒。
    pub cpu_ms: u64,
    /// 窗口内磁盘读写的字节数。
    pub disk_bytes: u64,
}

/// 一次 WDI 读取的结果。
#[derive(Debug, Clone, Default)]
pub struct StartupInfoReport {
    /// 跟踪窗口长度（毫秒）。`IntervalEndMs - IntervalStartMs`。
    pub window_ms: Option<u64>,
    /// 文件里一共多少条进程记录。
    pub records: Vec<StartupInfoRecord>,
    /// 数据取自哪个用户（SID 字符串）。
    pub source_sid: Option<String>,
    /// 上面那个 SID 是否就是当前登录用户。
    pub is_current_user: bool,
    /// 实际读的文件名。界面把它写进证据区——用户能自己去打开这个文件核对，
    /// 这是这份数据最该被验证、也最经得起验证的地方。
    pub source_file: Option<String>,
    /// 不可用时的原因（人话）。可用时为空。
    pub unavailable_reason: Option<String>,
}

impl StartupInfoReport {
    /// 构造一个"不可用"的结果。
    ///
    /// `pub` 是刻意的：调用方（以及测试）需要能构造出"读不到"这一态，
    /// 而不是只能靠 `Default::default()`——后者**看起来是可用**的
    /// （`unavailable_reason` 为空、`is_available()` 为真），
    /// 拿它当"读不到"会让界面把"没读到"说成"读到了但是空的"。
    pub fn unavailable(why: impl Into<String>) -> Self {
        Self {
            unavailable_reason: Some(why.into()),
            ..Default::default()
        }
    }

    /// 是否有可用的记录。
    pub fn is_available(&self) -> bool {
        self.unavailable_reason.is_none() && !self.records.is_empty()
    }
}

/// 读取并解析当前用户的 WDI `StartupInfo`。**只读，失败退化为"不可用"。**
pub fn capture() -> StartupInfoReport {
    #[cfg(windows)]
    {
        windows_impl::capture()
    }
    #[cfg(not(windows))]
    {
        StartupInfoReport::unavailable("此功能依赖 Windows 诊断基础架构（WDI）。".to_string())
    }
}

/// 解析 `StartupInfo` XML，取出全部进程记录。**纯函数，供离线单测直接喂 fixture。**
///
/// 容错规则：
/// * 缺 `CPUUsage` / `DiskUsage` 的子节点按 0 处理（不是"没有这一项"，是"这项没测到"）。
/// * `Name` 或 `PID` 解析不出来的节点整条跳过——没有身份就没法归因，
///   硬塞一条空记录只会在界面上多出一行问号。
/// * 大小写不敏感地找标签：不同 Windows 版本里出现过 `CpuUsage` 与 `CPUUsage`。
pub fn parse_records(xml: &str) -> Vec<StartupInfoRecord> {
    let mut out = Vec::new();
    let mut cursor = 0usize;

    while let Some(rel) = xml[cursor..].find("<Process") {
        let start = cursor + rel;

        let Some(gt_rel) = xml[start..].find('>') else {
            break;
        };
        let head = &xml[start..start + gt_rel];
        let body_start = start + gt_rel + 1;

        // 自闭合的 <Process .../> 没有内容，跳过（防御性，实际不会出现）
        let self_closing = head.trim_end().ends_with('/');

        // 节点体到 </Process> 为止。找不到闭合标签就说明文件被截断，
        // 此时**不解析这半条**——半个进程记录比没有更糟。
        let body_end = if self_closing {
            body_start
        } else {
            let Some(close_rel) = xml[body_start..].find("</Process>") else {
                break;
            };
            body_start + close_rel
        };
        let body = &xml[body_start..body_end];
        cursor = body_end;

        let Some(image) = attr_of(head, "Name").filter(|s| !s.trim().is_empty()) else {
            continue;
        };
        let Some(exe) = exe_of(image) else { continue };
        let Some(pid) = attr_of(head, "PID").and_then(|s| s.trim().parse::<u32>().ok()) else {
            continue;
        };

        // 三种标签写法的都认：CpuUsage / CPUUsage / CpuTime
        let cpu_us = child_num(body, &["CpuUsage", "CPUUsage", "CpuTime"]).unwrap_or(0);
        let disk_bytes = child_num(body, &["DiskUsage"]).unwrap_or(0);
        let started_in_trace_ms = attr_of(head, "StartedInTraceSec")
            .or_else(|| child_text(body, &["StartedInTraceSec"]))
            .and_then(parse_secs_to_ms);

        out.push(StartupInfoRecord {
            exe,
            image: unescape(image),
            pid,
            started_in_trace_ms,
            cpu_ms: cpu_us / 1000,
            disk_bytes,
        });
    }

    out
}

/// 读根节点的 `IntervalStartMs` / `IntervalEndMs`，差就是跟踪窗口长度。
pub fn parse_window_ms(xml: &str) -> Option<u64> {
    let head = root_head(xml)?;
    let start = attr_of(head, "IntervalStartMs")?.trim().parse::<i64>().ok()?;
    let end = attr_of(head, "IntervalEndMs")?.trim().parse::<i64>().ok()?;
    let diff = end.checked_sub(start)?;
    (diff > 0).then_some(diff as u64)
}

/// 把磁盘字节与 CPU 毫秒按**微软公开的阈值**分成三档。
///
/// 阈值照抄任务管理器（High = CPU > 1s **或** 磁盘 > 3MB；
/// Medium = CPU ≥ 300ms **或** 磁盘 ≥ 300KB；否则 Low）。
/// 之所以照抄而不是自己拍一个：用户能拿任务管理器对照，
/// 两边的档位不一致会立刻显得这个软件不可信。
pub fn level_of(cpu_ms: u64, disk_bytes: u64) -> ImpactLevel {
    const MIB: u64 = 1024 * 1024;
    const KIB: u64 = 1024;

    if cpu_ms > 1000 || disk_bytes > 3 * MIB {
        ImpactLevel::High
    } else if cpu_ms >= 300 || disk_bytes >= 300 * KIB {
        ImpactLevel::Medium
    } else {
        ImpactLevel::Low
    }
}

/// 取可执行文件名小写。`Name` 可能是完整路径，也可能只有文件名。
fn exe_of(image: &str) -> Option<String> {
    let base = image.rsplit(['\\', '/']).next()?.trim();
    if base.is_empty() {
        return None;
    }
    Some(base.to_ascii_lowercase())
}

/// 取根标签的头部（`<StartupData ...` 直到 `>`），供属性查找。
fn root_head(xml: &str) -> Option<&str> {
    let start = xml.find('<')?;
    // 跳过 XML 声明 `<?xml ... ?>`
    let mut i = start;
    loop {
        let rest = &xml[i..];
        if rest.starts_with("<?") || rest.starts_with("<!") {
            i += rest.find("?>").or_else(|| rest.find('>'))? + 2;
            continue;
        }
        let end = rest.find('>')?;
        return Some(&rest[..end]);
    }
}

/// 从标签头部取 `attr="value"` / `attr='value'`。兼容单双引号。
fn attr_of<'a>(head: &'a str, attr: &str) -> Option<&'a str> {
    let needle = format!("{attr}=");
    let mut from = 0usize;
    loop {
        let rel = head[from..].find(&needle)?;
        let eq = from + rel + needle.len();

        // ⚠️ 必须要求属性名在词边界上：找 "PID=" 时不能命中 "ParentPID="。
        let before = head[..eq - needle.len()].chars().last();
        if before.map_or(true, |c| c.is_ascii_whitespace() || c == '<') {
            let quote = head.as_bytes().get(eq).copied()? as char;
            if quote == '"' || quote == '\'' {
                let val_start = eq + 1;
                let val_end = head[val_start..].find(quote)? + val_start;
                return Some(&head[val_start..val_end]);
            }
        }
        from = eq;
    }
}

/// 取子节点文本，标签名大小写不敏感、允许开标签带属性。
fn child_text<'a>(body: &'a str, tags: &[&str]) -> Option<&'a str> {
    for tag in tags {
        let lower = body.to_ascii_lowercase();
        let open = format!("<{}", tag.to_ascii_lowercase());
        let mut from = 0usize;
        while let Some(rel) = lower[from..].find(&open) {
            let start = from + rel;
            let after = &body[start + open.len()..];
            // 必须有 '>' 或空白，避免 `<CpuUsageX>` 被当成 `<CpuUsage>`
            let first = after.chars().next()?;
            if first != '>' && !first.is_whitespace() {
                from = start + open.len();
                continue;
            }
            let gt = after.find('>')?;
            let value_start = start + open.len() + gt + 1;
            let close = format!("</{}>", tag.to_ascii_lowercase());
            let close_rel = lower[value_start..].find(&close)?;
            return Some(&body[value_start..value_start + close_rel]);
        }
    }
    None
}

/// 取子节点里的数字（剥掉 CDATA 包裹与空白）。
fn child_num(body: &str, tags: &[&str]) -> Option<u64> {
    let raw = child_text(body, tags)?;
    let cleaned = raw.trim().trim_start_matches("<![CDATA[").trim_end_matches("]]>").trim();
    let digits: String = cleaned
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

/// `"48.500"` → `48500`。容忍小数点与前后空白；负数/非法值返回 `None`。
fn parse_secs_to_ms(raw: &str) -> Option<u64> {
    let v: f64 = raw.trim().parse().ok()?;
    if !v.is_finite() || v < 0.0 {
        return None;
    }
    Some((v * 1000.0).round() as u64)
}

/// 还原 XML 实体（路径里出现 `&amp;` 是常事）。
fn unescape(s: &str) -> String {
    crate::diag::boot_log::unescape_xml(s)
}

/// 文件是 **UTF-16 LE**（带 BOM），不是 UTF-8。
///
/// 这条踩中过一次就再也不会忘：按 UTF-8 读会得到一串问号与 NUL 字节，
/// `find("<Process")` 永远找不到，表现为"文件存在但一条记录都没解析出来"。
/// 所以这里按 BOM 与 NUL 分布自动判定，UTF-16 走 `from_utf16_lossy`。
pub fn decode_bytes(bytes: &[u8]) -> String {
    // UTF-16 BOM：FF FE（LE）/ FE FF（BE）
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return from_utf16_lossy(&bytes[2..]);
    }
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let swapped: Vec<u8> = bytes[2..]
            .chunks_exact(2)
            .flat_map(|p| [p[1], p[0]])
            .collect();
        return from_utf16_lossy(&swapped);
    }

    // 没有 BOM 时看 NUL 字节的分布：UTF-16 的 ASCII 区间每隔一个字节就是 0。
    if bytes.len() >= 4 {
        let sample = &bytes[..bytes.len().min(64)];
        let nuls_at_odd = sample.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
        let odd_slots = sample.len() / 2;
        if odd_slots > 0 && nuls_at_odd * 2 >= odd_slots {
            return from_utf16_lossy(bytes);
        }
    }

    String::from_utf8_lossy(bytes).into_owned()
}

fn from_utf16_lossy(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(windows)]
mod windows_impl {
    //! Windows 侧：定位文件 + 读字节。全部只读，每一步失败都退化为"不可用"。

    use super::{StartupInfoReport, decode_bytes, parse_records, parse_window_ms};
    use std::path::{Path, PathBuf};

    /// `%WINDIR%\System32\WDI\LogFiles\StartupInfo`
    ///
    /// 用 `GetWindowsDirectoryW` 而不是环境变量：环境变量可被改，
    /// 而这份文件的位置是系统固定的。
    ///
    /// ⚠️ 这里**故意不判断目录是否存在**。实测踩过一次：`WDI` 整棵树对普通用户
    /// 是拒读的，`Path::is_dir()` 在父目录不可穿越时返回 `false`——
    /// 于是"没有权限"被报成了"这台机器上没有这个目录"。
    /// 两种情况的处置完全不同（一个让用户提权，一个让他等系统生成），
    /// 所以判断必须交给 `read_dir` 的真实错误码。
    fn log_dir() -> Option<PathBuf> {
        let windir = windows_dir()?;
        Some(
            windir
                .join("System32")
                .join("WDI")
                .join("LogFiles")
                .join("StartupInfo"),
        )
    }

    fn windows_dir() -> Option<PathBuf> {
        use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
        // `GetWindowsDirectoryW` 返回的是"写进缓冲区的字符数"，
        // 并且**不保证带结尾的 NUL**，所以按返回长度截取而不是找 NUL。
        let mut buf = [0u16; 512];
        let n = unsafe { GetWindowsDirectoryW(Some(&mut buf)) };
        if n == 0 || n as usize >= buf.len() {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..n as usize])))
    }

    /// 把 `read_dir` 的错误翻译成人话。**必须区分"没权限"与"不存在"。**
    ///
    /// 这不是措辞问题：前者要引导用户提权，后者要引导他等系统生成，
    /// 而错的那一句会把用户送到完全错误的方向上。
    pub(super) fn classify_access_error(e: &std::io::Error) -> String {
        match e.kind() {
            std::io::ErrorKind::PermissionDenied => {
                "读取这份数据需要管理员权限，当前以普通权限运行。以管理员身份重开本程序即可看到。"
                    .to_string()
            }
            std::io::ErrorKind::NotFound => {
                "系统里还没有这份记录（Windows 会在每次登录后生成一份，也可能被策略关掉了）。"
                    .to_string()
            }
            _ => format!("读取开机启动记录失败：{e}"),
        }
    }

    /// 文件名形如 `<SID>_StartupInfo<n>.xml`。取出 SID 部分。
    fn sid_of(file_name: &str) -> Option<String> {
        let (sid, _) = file_name.split_once("_StartupInfo")?;
        sid.starts_with("S-1-").then(|| sid.to_string())
    }

    /// 列目录。目录不在、或没权限，都在这里被如实翻译成一句话。
    /// **不要**在外面先判存在性——见 `log_dir` 的注释。
    fn list_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
        let rd = std::fs::read_dir(dir).map_err(|e| classify_access_error(&e))?;
        Ok(rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let n = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                n.contains("_StartupInfo") && n.to_ascii_lowercase().ends_with(".xml")
            })
            .collect())
    }

    /// 当前用户 SID。
    ///
    /// 走注册表 `ProfileList` 而不是 `GetTokenInformation(TokenUser)`：
    /// 后者要再开一个 windows feature，而 `ProfileList` 的键名就是 SID、
    /// `ProfileImagePath` 就是用户目录，用现成的 winreg 就能拿到，
    /// 而且**和登录会话的对应关系更直接**（我们关心的正是"谁登录了"）。
    fn current_user_sid() -> Option<String> {
        let profile = std::env::var("USERPROFILE").ok()?;
        let profile = profile.trim_end_matches(['\\', '/']).to_ascii_lowercase();
        if profile.is_empty() {
            return None;
        }

        let hklm = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE);
        let list = hklm
            .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList")
            .ok()?;

        for name in list.enum_keys().flatten() {
            if !name.starts_with("S-1-") {
                continue;
            }
            let Ok(sub) = list.open_subkey(&name) else { continue };
            let Ok(path): Result<String, _> = sub.get_value("ProfileImagePath") else {
                continue;
            };
            if path
                .trim_end_matches(['\\', '/'])
                .to_ascii_lowercase()
                == profile
            {
                return Some(name);
            }
        }
        None
    }

    /// 按修改时间取最新。
    ///
    /// ⚠️ Windows 为每个用户保留**最近 5 份**（`_StartupInfo1..5.xml`）。
    /// 只按"第一个匹配到的"取会拿到最旧的一次登录——数字看着合理、日期是错的，
    /// 而错的那一份恰好是"上次开机"的。所以必须比时间，不能比文件名。
    fn newest(files: &[PathBuf]) -> Option<&PathBuf> {
        files.iter().max_by_key(|p| {
            p.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
    }

    pub fn capture() -> StartupInfoReport {
        let Some(dir) = log_dir() else {
            return StartupInfoReport::unavailable(
                "拿不到 Windows 目录路径，无法定位开机启动记录。".to_string(),
            );
        };

        let files = match list_files(&dir) {
            Ok(f) => f,
            Err(why) => return StartupInfoReport::unavailable(why),
        };
        if files.is_empty() {
            return StartupInfoReport::unavailable(
                "目录在，但里面还没有开机启动记录文件——WDI 会在每次登录后生成一份。".to_string(),
            );
        }

        // 先按当前用户的 SID 挑，挑不到才退到"最新的那个用户"。
        // 退让时必须让界面知道这是别人的登录，否则数字会被当成自己的。
        let own_sid = current_user_sid();
        let (file, sid, is_current_user) = own_sid
            .as_deref()
            .and_then(|sid| {
                let mine: Vec<PathBuf> = files
                    .iter()
                    .filter(|p| p.file_name().and_then(|s| s.to_str()).and_then(sid_of).as_deref() == Some(sid))
                    .cloned()
                    .collect();
                newest(&mine).cloned().map(|f| (f, Some(sid.to_string()), true))
            })
            .or_else(|| {
                newest(&files).cloned().map(|f| {
                    let sid = f.file_name().and_then(|s| s.to_str()).and_then(sid_of);
                    (f, sid, false)
                })
            })
            .unwrap();

        let bytes = match std::fs::read(&file) {
            Ok(b) => b,
            Err(e) => return StartupInfoReport::unavailable(classify_access_error(&e)),
        };

        let xml = decode_bytes(&bytes);
        let records = parse_records(&xml);

        if records.is_empty() {
            return StartupInfoReport {
                source_sid: sid,
                is_current_user,
                source_file: file.file_name().and_then(|s| s.to_str()).map(str::to_string),
                unavailable_reason: Some(
                    "文件读到了，但里面没有任何可解析的进程记录（格式与已知版本不同）。"
                        .to_string(),
                ),
                ..Default::default()
            };
        }

        StartupInfoReport {
            window_ms: parse_window_ms(&xml),
            records,
            source_sid: sid,
            is_current_user,
            source_file: file.file_name().and_then(|s| s.to_str()).map(str::to_string),
            unavailable_reason: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 取自公开文档与解析器实现里给出的样本结构（Windows 10/11 实测格式）。
    /// 单位属性、CDATA 命令行、父进程字段都在，用来锁住解析器对这些写法的容忍度。
    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<StartupData IntervalStartMs="1000" IntervalEndMs="91000">
  <Process Name="C:\Windows\System32\SecurityHealthSystray.exe" PID="6208" StartedInTraceSec="48.500">
    <StartTime>2020/09/11:18:12:48.6685573</StartTime>
    <CommandLine><![CDATA["C:\Windows\System32\SecurityHealthSystray.exe" ]]></CommandLine>
    <DiskUsage Units="bytes">325120</DiskUsage>
    <CpuUsage Units="us">32024</CpuUsage>
    <ParentPID>6016</ParentPID>
    <ParentName>explorer.exe</ParentName>
  </Process>
  <Process Name="C:\Program Files\Acme\acmesync.exe" PID="7001" StartedInTraceSec="12.400">
    <CommandLine><![CDATA["C:\Program Files\Acme\acmesync.exe" --silent]]></CommandLine>
    <DiskUsage Units="bytes">8388608</DiskUsage>
    <CPUUsage Units="us">1402345</CPUUsage>
    <ParentPID>6016</ParentPID>
  </Process>
  <Process Name="C:\Windows\explorer.exe" PID="6016" StartedInTraceSec="3.100">
    <DiskUsage Units="bytes">1024</DiskUsage>
    <CpuUsage Units="us">50000</CpuUsage>
  </Process>
</StartupData>"#;

    #[test]
    fn parses_image_pid_and_start_offset() {
        let r = parse_records(SAMPLE);

        assert_eq!(r.len(), 3, "三个 Process 都要解析出来");

        assert_eq!(r[0].exe, "securityhealthsystray.exe");
        assert_eq!(r[0].image, r"C:\Windows\System32\SecurityHealthSystray.exe");
        assert_eq!(r[0].pid, 6208);
        assert_eq!(r[0].started_in_trace_ms, Some(48_500));
    }

    #[test]
    fn cpu_microseconds_become_milliseconds() {
        // CpuUsage 的单位是**微秒**，直接当毫秒用会放大 1000 倍，
        // 于是每一项都变成"高影响"——这正是最容易犯又最难发现的错。
        let r = parse_records(SAMPLE);
        assert_eq!(r[0].cpu_ms, 32, "32024us = 32ms");
        assert_eq!(r[1].cpu_ms, 1402, "1402345us ≈ 1402ms");
    }

    #[test]
    fn tolerates_cpu_usage_spelled_both_ways() {
        // 样本里第一条是 CpuUsage，第二条是 CPUUsage —— 两种都要认。
        let r = parse_records(SAMPLE);
        assert!(r[0].cpu_ms > 0, "CpuUsage 写法没被认出来");
        assert!(r[1].cpu_ms > 0, "CPUUsage 写法没被认出来");
    }

    #[test]
    fn parses_disk_usage_ignoring_units_attribute() {
        // 开标签带 Units="bytes" 属性，所以不能只找 `<DiskUsage>`。
        let r = parse_records(SAMPLE);
        assert_eq!(r[0].disk_bytes, 325_120);
        assert_eq!(r[1].disk_bytes, 8_388_608);
    }

    #[test]
    fn cdata_command_line_does_not_break_the_next_field() {
        // 命令行是 CDATA，里面还带引号。若解析器在这里迷路，
        // 后面 DiskUsage/CpuUsage 会整体错位到下一个进程上。
        let r = parse_records(SAMPLE);
        assert_eq!(r[1].disk_bytes, 8_388_608);
        assert_eq!(r[1].pid, 7001);
    }

    #[test]
    fn crlf_and_whitespace_between_nodes_are_fine() {
        let messy = SAMPLE.replace(">\n", ">\r\n").replace("<Process", "\n  <Process");
        assert_eq!(parse_records(&messy).len(), 3);
    }

    #[test]
    fn missing_disk_usage_yields_zero_not_a_dropped_row() {
        // 缺字段是"这项没测到"，不是"没有这一项"。丢掉整条会让它
        // 直接从界面上消失，用户会以为这个启动项不存在于系统记录里。
        let xml = r#"<StartupData>
            <Process Name="C:\a\a.exe" PID="10" StartedInTraceSec="1.0">
                <CpuUsage Units="us">1000</CpuUsage>
            </Process>
        </StartupData>"#;

        let r = parse_records(xml);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].disk_bytes, 0);
        assert_eq!(r[0].cpu_ms, 1);
    }

    #[test]
    fn process_without_identity_is_skipped() {
        // 没有 Name / PID 就没法归因到任何启动项。塞一条空记录
        // 只会在界面上多出一行问号。
        let xml = r#"<StartupData>
            <Process StartedInTraceSec="1.0"><CpuUsage Units="us">999999</CpuUsage></Process>
            <Process Name="C:\a\a.exe" PID="10"></Process>
        </StartupData>"#;

        let r = parse_records(xml);
        assert_eq!(r.len(), 1, "只有带全身份的才留下");
        assert_eq!(r[0].exe, "a.exe");
    }

    #[test]
    fn truncated_last_process_is_not_half_parsed() {
        // 文件被截断（WDI 正在写）时，最后一条只有半边。
        // 半条记录的数字是错的，宁可不要。
        let xml = r#"<StartupData>
            <Process Name="C:\a\a.exe" PID="10" StartedInTraceSec="1.0">
                <CpuUsage Units="us">1000</CpuUsage>
            </Process>
            <Process Name="C:\b\b.exe" PID="11" StartedInTraceSec="2.0">
                <CpuUsage Units="us">2000</CpuUsage>
        </StartupData>"#;

        let r = parse_records(xml);
        assert_eq!(r.len(), 1, "截断的半条不能进结果");
        assert_eq!(r[0].exe, "a.exe");
    }

    /* ───────────────── 影响分档：阈值必须与任务管理器一致 ───────────────── */

    #[test]
    fn impact_levels_match_task_manager_thresholds() {
        // 这些边界值照抄微软公开口径。任何一条改了，用户拿任务管理器
        // 一对照就会发现两边不一致——那比不给这个数据还糟。
        assert_eq!(level_of(299, 300 * 1024 - 1), ImpactLevel::Low);
        assert_eq!(level_of(300, 0), ImpactLevel::Medium);
        assert_eq!(level_of(999, 0), ImpactLevel::Medium);
        assert_eq!(level_of(0, 300 * 1024), ImpactLevel::Medium);
        assert_eq!(level_of(0, 3 * 1024 * 1024), ImpactLevel::Medium);

        assert_eq!(level_of(1001, 0), ImpactLevel::High);
        assert_eq!(level_of(0, 3 * 1024 * 1024 + 1), ImpactLevel::High);
    }

    #[test]
    fn level_uses_the_worse_of_the_two_axes() {
        // CPU 很低但读盘很大 → 高。只看 CPU 会把"载入大量资源"的项漏掉，
        // 而开机瓶颈恰恰几乎总在磁盘上。
        assert_eq!(level_of(10, 50 * 1024 * 1024), ImpactLevel::High);
        assert_eq!(level_of(5000, 0), ImpactLevel::High);
    }

    /* ───────────────── 属性查找 ───────────────── */

    #[test]
    fn attr_lookup_respects_word_boundary() {
        // 找 `PID=` 不能命中 `ParentPID=`。命中会让每个进程都取到父进程的
        // 进程号——两条记录换成同一个 PID，归因直接错乱。
        let head = r#"<Process Name="a.exe" ParentPID="6016" StartedInTraceSec="1">"#;
        assert_eq!(attr_of(head, "PID"), None);
        assert_eq!(attr_of(head, "ParentPID"), Some("6016"));

        let head2 = r#"<Process Name="a.exe" PID="42">"#;
        assert_eq!(attr_of(head2, "PID"), Some("42"));
    }

    #[test]
    fn attr_lookup_accepts_single_quotes() {
        // Windows 自己的渲染器在别处一律用单引号，这里也一并认下，
        // 免得上游换了渲染方式就要改解析器。
        let head = "<Process Name='a.exe' PID='7'>";
        assert_eq!(attr_of(head, "Name"), Some("a.exe"));
        assert_eq!(attr_of(head, "PID"), Some("7"));
    }

    #[test]
    fn window_comes_from_root_interval_attributes() {
        // 窗口长度是"这批数字覆盖了多久"，界面要靠它说明口径。
        assert_eq!(parse_window_ms(SAMPLE), Some(90_000));
    }

    #[test]
    fn window_is_none_when_attributes_absent_or_inverted() {
        assert_eq!(parse_window_ms("<StartupData></StartupData>"), None);
        assert_eq!(
            parse_window_ms(r#"<StartupData IntervalStartMs="900" IntervalEndMs="100">"#),
            None,
            "起点晚于终点说明字段含义不是我们以为的那样，宁可不给"
        );
    }

    /* ───────────────── 编码：UTF-16 LE ───────────────── */

    fn utf16le_with_bom(s: &str) -> Vec<u8> {
        let mut out = vec![0xFF, 0xFE];
        for u in s.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out
    }

    #[test]
    fn decodes_utf16le_which_is_what_wdi_writes() {
        let bytes = utf16le_with_bom(SAMPLE);

        // 按 UTF-8 读会得到一串 NUL 与替换字符，`find("<Process")` 永远失配，
        // 表现为"文件存在但一条记录都没有"。这条断言把那个陷阱钉住。
        assert!(
            !String::from_utf8_lossy(&bytes).contains("<Process"),
            "UTF-8 读法本来就该失败——这正是需要 BOM 判定的原因"
        );

        let xml = decode_bytes(&bytes);
        assert!(xml.contains("<Process"));
        assert_eq!(parse_records(&xml).len(), 3);
    }

    #[test]
    fn decodes_utf16be_as_well() {
        let mut bytes = vec![0xFE, 0xFF];
        for u in SAMPLE.encode_utf16() {
            bytes.extend_from_slice(&u.to_be_bytes());
        }
        assert_eq!(parse_records(&decode_bytes(&bytes)).len(), 3);
    }

    #[test]
    fn decodes_plain_utf8_unchanged() {
        assert_eq!(decode_bytes(SAMPLE.as_bytes()), SAMPLE);
    }

    #[test]
    fn decodes_bomless_utf16_by_nul_distribution() {
        // 少数版本不写 BOM。ASCII 区间里每隔一个字节是 0，
        // 靠这个分布足以认出来，不必要求用户手动指定编码。
        let mut bytes = Vec::new();
        for u in SAMPLE.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(parse_records(&decode_bytes(&bytes)).len(), 3);
    }

    /* ───────────────── 「读不到」必须分清是哪种读不到 ───────────────── */

    #[test]
    #[cfg(windows)]
    fn permission_denied_is_not_reported_as_missing() {
        // 本机实测踩过这个坑：`WDI` 整棵树对普通用户拒读，
        // 而 `Path::is_dir()` 在父目录不可穿越时返回 false ——
        // 于是"没权限"被报成了"这台机器上没有这个目录"，
        // 把用户引向"等系统生成"，而不是"提权重开"。
        // 两种情况的处置完全不同，所以必须按错误码分。
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);

        let a = windows_impl::classify_access_error(&denied);
        let b = windows_impl::classify_access_error(&missing);

        assert!(a.contains("管理员权限"), "无权限要引导提权：{a}");
        assert!(!b.contains("管理员权限"), "不存在不该说成没权限：{b}");
        assert_ne!(a, b, "两种情况必须给出不同的话");
    }

    /* ───────────────── 真机探针 ───────────────── */    /// 真机探针：打印本机 WDI 记录里最重的若干项。
    ///
    /// 跑法：`cargo test --lib probe_real_startup_info -- --ignored --nocapture`
    ///
    /// 需要管理员权限——该目录对普通用户连列目录都被拒。
    #[test]
    #[ignore]
    fn probe_real_startup_info() {
        let rep = capture();
        println!("=== WDI StartupInfo 真机探针 ===");
        println!("可用: {}", rep.is_available());
        println!("来源: {:?}（当前用户: {}）", rep.source_file, rep.is_current_user);
        println!("SID: {:?}", rep.source_sid);
        println!("窗口: {:?} ms", rep.window_ms);
        if let Some(why) = &rep.unavailable_reason {
            println!("不可用原因: {why}");
            return;
        }

        let mut rows = rep.records.clone();
        rows.sort_by_key(|r| std::cmp::Reverse(r.cpu_ms + r.disk_bytes / 1024));
        println!("--- 按影响排序前 15 ---");
        for r in rows.iter().take(15) {
            println!(
                "cpu {:>6}ms  disk {:>8.1}MB  @{:>7.1}s  {:?}  {}",
                r.cpu_ms,
                r.disk_bytes as f64 / 1_048_576.0,
                r.started_in_trace_ms.map(|v| v as f64 / 1000.0).unwrap_or(-1.0),
                level_of(r.cpu_ms, r.disk_bytes),
                r.image
            );
        }
    }
}