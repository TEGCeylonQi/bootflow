//! 开机耗时事件日志的读取。
//!
//! ## 数据来源
//!
//! `Microsoft-Windows-Diagnostics-Performance/Operational`，
//! 由 Windows 自己在上次开机时写入。事件 ID：
//!
//! | ID | 含义 | 级别 |
//! |---|---|---|
//! | 100 | 开机主事件，含各相位耗时 | Critical/Informational |
//! | 101 | 某个**应用程序**启动慢 | Warning |
//! | 102 | 某个**驱动**初始化慢 | Warning |
//! | 103 | 某个**服务**启动慢 | Warning |
//!
//! ⚠️ 字段名来自 `provider` 元数据与官方事件样本，**不是凭记忆写的**：
//! 事件 100 存在 version 1 / version 2 两个模板（本机 `wevtutil gp` 实测），
//! 所以解析一律按 `Data@Name` 取值，**不做位置假设**。
//! 缺字段就跳过那一段，不崩、不猜。
//!
//! ## ⚠️ 权限：这是本模块最需要如实交代的一件事
//!
//! 实测该 channel 的访问控制列表里**没有给普通用户任何权限**：
//!
//! ```text
//! channelAccess: O:BAG:SYD:(D;;0xf0007;;;AN)(D;;0xf0007;;;BG)
//!                     (A;;0xf0007;;;SY)(A;;0x7;;;BA)(A;;0x5;;;LS)
//!                     (A;;0x5;;;NS)(A;;0x1;;;SU)
//! ```
//!
//! 有权的是 `SY`(System) / `BA`(管理员) / `LS`(LocalService) /
//! `NS`(NetworkService) / `SU`(服务账户)。**普通登录用户不在其中**，
//! `EVT_VARIANT` 级别的 `wevtutil qe` 与 `Get-WinEvent` 一律返回"拒绝访问"。
//! 连 `.evtx` 文件本身也不可读（NTFS ACL 同样不给）。
//!
//! 这不是可以绕过的技术障碍，而是 Windows 的设计。所以本模块的职责是
//! **把"读不到"和"没有数据"区分清楚**，交给上层如实呈现：
//! 前者要引导用户提权，后者只是这台机器还没记录过开机事件。
//! 把两者混成一个空列表，用户会以为自己电脑的开机耗时是 0 秒。

#![allow(dead_code)]

use std::collections::BTreeMap;

/// 事件通道名。
const CHANNEL: &str = "Microsoft-Windows-Diagnostics-Performance/Operational";

/// 一次查询最多取多少条。开机事件不多（每次开机 1 条 Event 100），
/// 取 64 条足够覆盖最近几十次开机。
const MAX_EVENTS: usize = 64;

/// 读取结果。**三态是刻意的**——见模块头注释。
#[derive(Debug, Clone)]
pub enum BootLogOutcome {
    /// 成功读到原始事件
    Events(Vec<RawEvent>),
    /// 没有权限。需要提权后重试。
    AccessDenied,
    /// 日志本身不可用（未启用 / 被清理过 / 系统精简掉了）
    Unavailable(String),
}

/// 一条已解析的原始事件。
#[derive(Debug, Clone, Default)]
pub struct RawEvent {
    pub event_id: u32,
    /// `EventData` 里 `Data@Name` → 值。
    ///
    /// 用 `BTreeMap` 而不是 `HashMap`：导出报告和单测需要稳定顺序。
    pub data: BTreeMap<String, String>,
    /// `System/TimeCreated@SystemTime`
    pub time_created: Option<String>,
    /// `System/Computer`
    pub computer: Option<String>,
}

impl RawEvent {
    /// 取一个字符串字段。
    pub fn get(&self, key: &str) -> Option<&str> {
        self.data.get(key).map(|s| s.as_str()).filter(|s| !s.is_empty())
    }

    /// 取一个数值字段。解析失败按"没有"处理——事件字段偶尔会是空串。
    pub fn get_u64(&self, key: &str) -> Option<u64> {
        self.get(key)?.trim().parse().ok()
    }

    /// 取一个布尔字段。事件里的写法是 `true` / `false`。
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key)?.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        }
    }
}

/// 读取最近的开机性能事件。
pub fn read_boot_events() -> BootLogOutcome {
    #[cfg(windows)]
    {
        windows_impl::query()
    }

    #[cfg(not(windows))]
    {
        BootLogOutcome::Unavailable("仅支持 Windows".to_string())
    }
}

/* ───────────────────── XML 解析 ───────────────────── */

/// 从一条渲染后的 `<Event>` XML 里提取结构。
///
/// 手写而不是引 XML 库，理由是**解析目标极窄且格式固定**：
/// 只需要 `EventID`、`TimeCreated`、`Computer` 和一组
/// `<Data Name="X">v</Data>`。引一个通用 XML 库来干这件事，
/// 换来的是一个新的依赖面和一组自己的配置项；
/// 而这里真正需要的"容错"（缺字段不崩）手写反而更容易写对。
///
/// 仍然严格处理 XML 实体转义——路径里出现 `&amp;` 是常事
/// （`C:\Program Files\A & B\a.exe`），不还原就会把路径读错，
/// 而这个错误会一路传到"目标文件不存在"的误判上。
pub fn parse_event_xml(xml: &str) -> Option<RawEvent> {
    let event_id: u32 = extract_tag(xml, "EventID")?.trim().parse().ok()?;

    let time_created = extract_attr(xml, "TimeCreated", "SystemTime").map(str::to_string);
    let computer = extract_tag(xml, "Computer").map(str::to_string);

    Some(RawEvent {
        event_id,
        data: parse_data_entries(xml),
        time_created,
        computer,
    })
}

/// 取 `<tag>value</tag>` 里的 value。
fn extract_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");

    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

/// 取 `<tag attr="value"` 里的 value。
fn extract_attr<'a>(xml: &'a str, tag: &str, attr: &str) -> Option<&'a str> {
    let tag_pos = xml.find(&format!("<{tag}"))?;
    let rest = &xml[tag_pos..];

    // 只在标签内部找，避免蹭到后面元素的同名属性
    let tag_end = rest.find('>')?;
    let head = &rest[..tag_end];

    let needle = format!("{attr}=\"");
    let start = head.find(&needle)? + needle.len();
    let end = head[start..].find('"')? + start;

    Some(&head[start..end])
}

/// 提取全部 `<Data Name="X">v</Data>` / `<Data Name="X"/>`。
fn parse_data_entries(xml: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut cursor = 0usize;

    while let Some(rel) = xml[cursor..].find("<Data") {
        let start = cursor + rel;

        let Some(gt_rel) = xml[start..].find('>') else {
            break;
        };
        let head = xml[start..start + gt_rel].trim_end();
        cursor = start + gt_rel + 1;

        // `<Data Name="X"/>` —— 有键无值，事件里常见（字段被置空）
        let self_closing = head.ends_with('/');

        let name = match head.find("Name=\"") {
            Some(p) => {
                let s = p + 6;
                match head[s..].find('"') {
                    Some(e) => head[s..s + e].to_string(),
                    // 引号不闭合：整段当名字，总比丢掉这个字段强
                    None => head[s..].to_string(),
                }
            }
            // 没有 Name 属性的 Data（老模板里可能出现）按位置无名处理，
            // 直接跳过——按顺序猜名字比丢掉更危险。
            None => continue,
        };

        if name.is_empty() {
            continue;
        }

        if self_closing {
            out.insert(name, String::new());
            continue;
        }

        let Some(close_rel) = xml[cursor..].find("</Data>") else {
            break;
        };
        let raw_value = &xml[cursor..cursor + close_rel];
        cursor += close_rel + "</Data>".len();

        out.insert(name, unescape_xml(raw_value));
    }

    out
}

/// 还原 XML 实体。
fn unescape_xml(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }

    // ⚠️ `&amp;` 必须**最后**替换。反过来做的话，
    // `&amp;lt;` 会先被当成 `&lt;` 再变成 `<`，
    // 而它本意是字面量 "&lt;"。
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#13;", "\r")
        .replace("&#10;", "\n")
        .replace("&#9;", "\t")
        .replace("&amp;", "&")
}

#[cfg(windows)]
mod windows_impl {
    use super::{BootLogOutcome, RawEvent, CHANNEL, MAX_EVENTS};
    use windows::Win32::Foundation::ERROR_ACCESS_DENIED;
    use windows::Win32::System::EventLog::{
        EvtClose, EvtNext, EvtQuery, EvtRender, EvtQueryChannelPath, EvtQueryReverseDirection,
        EvtRenderEventXml, EVT_HANDLE,
    };

    /// 事件句柄 / 结果集句柄的 RAII 包装。
    ///
    /// 结果集里每一个 event 也是独立句柄，都要关。漏掉的话
    /// 一次扫描泄漏几十个句柄，反复刷新界面迟早耗尽。
    struct EvtGuard(EVT_HANDLE);

    impl Drop for EvtGuard {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe {
                    let _ = EvtClose(self.0);
                }
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn query() -> BootLogOutcome {
        let channel = wide(CHANNEL);
        // 只取这四类：100 是相位总账，101/102/103 是"谁拖慢了"。
        // 109/110 之类首版不解析，取回来也是丢。
        let xpath = wide(
            "*[System[(EventID=100 or EventID=101 or EventID=102 or EventID=103)]]",
        );

        unsafe {
            let flags = EvtQueryChannelPath.0 | EvtQueryReverseDirection.0;

            let result = match EvtQuery(
                EVT_HANDLE(0),
                windows::core::PCWSTR(channel.as_ptr()),
                windows::core::PCWSTR(xpath.as_ptr()),
                flags,
            ) {
                Ok(h) => EvtGuard(h),
                Err(e) => return classify_open_error(&e),
            };

            let mut out = Vec::new();
            let mut batch = vec![0isize; 16];

            loop {
                let mut returned: u32 = 0;

                // EvtNext 的 timeout：0 表示不等待，立即返回已有的。
                // 这里不需要等待，日志是已经落盘的。
                let next = EvtNext(result.0, &mut batch, 0, 0, &mut returned);

                if next.is_err() || returned == 0 {
                    break;
                }

                for &raw in batch.iter().take(returned as usize) {
                    let handle = EvtGuard(EVT_HANDLE(raw));
                    if let Some(xml) = render_xml(handle.0) {
                        if let Some(ev) = super::parse_event_xml(&xml) {
                            out.push(ev);
                        }
                    }
                    if out.len() >= MAX_EVENTS {
                        break;
                    }
                }

                if out.len() >= MAX_EVENTS || (returned as usize) < batch.len() {
                    break;
                }
            }

            if out.is_empty() {
                // 能打开查询但一条都没读到 —— 这台机器还没记录过开机性能事件
                // （新装的系统、日志被清理过、或"快速启动"从未触发过完整引导）。
                // 这**不是**权限问题，措辞必须区分开。
                return BootLogOutcome::Unavailable(
                    "这台电脑还没有记录过开机性能事件".to_string(),
                );
            }

            BootLogOutcome::Events(out)
        }
    }

    /// 把"打不开查询"的原因分成权限问题与其他问题。
    ///
    /// 这个区分直接决定界面上的动作：权限问题要给「以管理员身份重试」按钮，
    /// 其他问题给了按钮也没用。
    fn classify_open_error(e: &windows::core::Error) -> BootLogOutcome {
        if e.code() == ERROR_ACCESS_DENIED.to_hresult() {
            BootLogOutcome::AccessDenied
        } else {
            BootLogOutcome::Unavailable(format!("无法读取系统性能日志：{e}"))
        }
    }

    /// 把一个事件渲染成 XML 字符串。
    fn render_xml(handle: EVT_HANDLE) -> Option<String> {
        unsafe {
            let mut used: u32 = 0;
            let mut prop_count: u32 = 0;

            // 第一次问需要多大缓冲。必然返回失败，只看 used。
            let _ = EvtRender(
                EVT_HANDLE(0),
                handle,
                EvtRenderEventXml.0,
                0,
                None,
                &mut used,
                &mut prop_count,
            );

            if used == 0 {
                return None;
            }

            let mut buf = vec![0u8; used as usize];

            EvtRender(
                EVT_HANDLE(0),
                handle,
                EvtRenderEventXml.0,
                used,
                Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
                &mut used,
                &mut prop_count,
            )
            .ok()?;

            // XML 是 UTF-16LE，长度单位是字符，末尾带 NUL
            let units = used as usize / 2;
            let wide = std::slice::from_raw_parts(buf.as_ptr() as *const u16, units);

            Some(String::from_utf16_lossy(wide).trim_end_matches('\0').to_string())
        }
    }

    #[allow(unused)]
    fn _keep_imports(_: RawEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实事件样本（Event 103，字段取自公开的事件样本，
    /// 与 `provider` 元数据一致：Name / FriendlyName / Version /
    /// TotalTime / DegradationTime / Path / ProductName / CompanyName）。
    const EVENT_103: &str = r#"<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event"><System><Provider Name="Microsoft-Windows-Diagnostics-Performance" Guid="{CFC18EC0-96B1-4EBA-961B-622CAEE05B0A}" /><EventID>103</EventID><Version>1</Version><Level>3</Level><Task>4002</Task><Opcode>33</Opcode><TimeCreated SystemTime="2026-03-17T18:15:25.9842765Z" /><EventRecordID>61</EventRecordID><Channel>Microsoft-Windows-Diagnostics-Performance/Operational</Channel><Computer>JD-WIN11</Computer><Security UserID="S-1-5-19" /></System><EventData><Data Name="StartTime">2026-03-17T18:13:15.4645682Z</Data><Data Name="NameLength">10</Data><Data Name="Name">windefend</Data><Data Name="FriendlyNameLength">0</Data><Data Name="FriendlyName"></Data><Data Name="VersionLength">0</Data><Data Name="Version"></Data><Data Name="TotalTime">326</Data><Data Name="DegradationTime">234</Data><Data Name="Path">c:\programdata\microsoft\windows defender\platform\msmpeng.exe</Data><Data Name="ProductName"></Data><Data Name="CompanyName"></Data></EventData></Event>"#;

    /// Event 100 样本（version 2，含全部相位字段）。
    const EVENT_100: &str = r#"<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event"><System><Provider Name="Microsoft-Windows-Diagnostics-Performance" /><EventID>100</EventID><Version>2</Version><Level>1</Level><TimeCreated SystemTime="2015-06-01T06:05:39.839684000Z" /><Channel>Microsoft-Windows-Diagnostics-Performance/Operational</Channel><Computer>malcolm-PC</Computer></System><EventData><Data Name="BootTsVersion">2</Data><Data Name="BootStartTime">2015-06-01T06:02:36.734000400Z</Data><Data Name="BootEndTime">2015-06-01T06:05:31.883670200Z</Data><Data Name="BootTime">165331</Data><Data Name="MainPathBootTime">151680</Data><Data Name="BootKernelInitTime">38</Data><Data Name="BootDriverInitTime">9668</Data><Data Name="BootDevicesInitTime">3335</Data><Data Name="BootSmssInitTime">21943</Data><Data Name="BootCriticalServicesInitTime">4171</Data><Data Name="BootUserProfileProcessingTime">4098</Data><Data Name="BootMachineProfileProcessingTime">2947</Data><Data Name="BootExplorerInitTime">104515</Data><Data Name="BootNumStartupApps">9</Data><Data Name="BootPostBootTime">13651</Data><Data Name="BootIsRebootAfterInstall">false</Data><Data Name="BootIsDegradation">false</Data><Data Name="OSLoaderDuration">3923</Data><Data Name="BootPNPInitStartTimeMS">38</Data><Data Name="BootPNPInitDuration">5968</Data><Data Name="OtherKernelInitDuration">1139</Data><Data Name="SystemPNPInitStartTimeMS">6925</Data><Data Name="SystemPNPInitDuration">7035</Data><Data Name="SessionInitStartTimeMS">14143</Data><Data Name="Session0InitDuration">1272</Data><Data Name="Session1InitDuration">1693</Data><Data Name="SessionInitOtherDuration">18978</Data><Data Name="WinLogonStartTimeMS">36087</Data><Data Name="OtherLogonInitActivityDuration">4030</Data><Data Name="UserLogonWaitDuration">9820</Data></EventData></Event>"#;

    #[test]
    fn parses_event_103_fields() {
        let ev = parse_event_xml(EVENT_103).expect("应能解析");
        assert_eq!(ev.event_id, 103);
        assert_eq!(ev.get("Name"), Some("windefend"));
        assert_eq!(ev.get_u64("TotalTime"), Some(326));
        assert_eq!(ev.get_u64("DegradationTime"), Some(234));
        assert_eq!(ev.computer.as_deref(), Some("JD-WIN11"));
        assert_eq!(
            ev.time_created.as_deref(),
            Some("2026-03-17T18:15:25.9842765Z")
        );
    }

    #[test]
    fn empty_data_value_is_treated_as_absent() {
        // 真实事件里 FriendlyName 是空字符串。若按"存在"处理，
        // 界面会显示一个空的"友好名称"字段，或让下游拿空串去查表。
        let ev = parse_event_xml(EVENT_103).unwrap();
        assert!(ev.data.contains_key("FriendlyName"), "字段本身应被记录");
        assert_eq!(ev.get("FriendlyName"), None, "空值应视为没有");
    }

    #[test]
    fn parses_event_100_phase_fields() {
        let ev = parse_event_xml(EVENT_100).expect("应能解析");
        assert_eq!(ev.event_id, 100);
        assert_eq!(ev.get_u64("BootTime"), Some(165331));
        assert_eq!(ev.get_u64("MainPathBootTime"), Some(151680));
        assert_eq!(ev.get_u64("BootPostBootTime"), Some(13651));
        assert_eq!(ev.get_u64("WinLogonStartTimeMS"), Some(36087));
        assert_eq!(ev.get_bool("BootIsDegradation"), Some(false));
    }

    /// 官方的两条恒等式，用来锁住"我们没有读错字段"。
    ///
    /// 这两个关系在 Microsoft 的文档与事件样本里都能对上，
    /// 是校准解析器最便宜的抓手：字段读错一位，等式立刻不成立。
    #[test]
    fn event_100_satisfies_known_identities() {
        let ev = parse_event_xml(EVENT_100).unwrap();

        let boot = ev.get_u64("BootTime").unwrap();
        let main = ev.get_u64("MainPathBootTime").unwrap();
        let post = ev.get_u64("BootPostBootTime").unwrap();

        assert_eq!(
            boot,
            main + post,
            "BootTime 必须等于 MainPathBootTime + BootPostBootTime"
        );

        // 相位锚点必须单调递增，否则说明字段串位了
        let anchors = [
            ev.get_u64("BootPNPInitStartTimeMS").unwrap(),
            ev.get_u64("SystemPNPInitStartTimeMS").unwrap(),
            ev.get_u64("SessionInitStartTimeMS").unwrap(),
            ev.get_u64("WinLogonStartTimeMS").unwrap(),
        ];
        for w in anchors.windows(2) {
            assert!(w[0] < w[1], "相位锚点应单调递增，实际 {anchors:?}");
        }
    }

    #[test]
    fn xml_entities_are_unescaped() {
        // 路径里出现 & 是常事，不还原会把路径读错，
        // 而这个错误会一路传到"目标文件不存在"的误判上
        let xml = r#"<Event><System><EventID>103</EventID></System><EventData><Data Name="Path">C:\Program Files\A &amp; B\a.exe</Data><Data Name="X">&lt;raw&gt;</Data></EventData></Event>"#;
        let ev = parse_event_xml(xml).unwrap();

        assert_eq!(ev.get("Path"), Some(r"C:\Program Files\A & B\a.exe"));
        assert_eq!(ev.get("X"), Some("<raw>"));
    }

    #[test]
    fn amp_is_unescaped_last() {
        // `&amp;lt;` 本意是字面量 "&lt;"。若先把 `&amp;` 换成 `&`，
        // 就会再接一轮把 `&lt;` 变成 "<"，把字面量吃成标签。
        let xml = r#"<Event><System><EventID>1</EventID></System><EventData><Data Name="X">&amp;lt;</Data></EventData></Event>"#;
        let ev = parse_event_xml(xml).unwrap();
        assert_eq!(ev.get("X"), Some("&lt;"), "替换顺序错了会退化成 \"<\"");
    }

    #[test]
    fn self_closing_data_is_recorded_as_empty() {
        let xml = r#"<Event><System><EventID>100</EventID></System><EventData><Data Name="BootPrefetchBytes" /><Data Name="BootTime">100</Data></EventData></Event>"#;
        let ev = parse_event_xml(xml).unwrap();

        assert_eq!(ev.get_u64("BootTime"), Some(100), "自闭合字段不该吃掉后面的");
        assert_eq!(ev.get("BootPrefetchBytes"), None);
    }

    #[test]
    fn missing_fields_do_not_crash_and_are_reported_as_absent() {
        // Win10 / Win11 的事件 100 字段集不同（provider 里确实存在
        // version 1 与 version 2 两个模板）。缺字段必须只体现为"没有值"。
        let xml = r#"<Event><System><EventID>100</EventID></System><EventData><Data Name="BootTime">1000</Data></EventData></Event>"#;
        let ev = parse_event_xml(xml).unwrap();

        assert_eq!(ev.get_u64("BootTime"), Some(1000));
        assert_eq!(ev.get_u64("WinLogonStartTimeMS"), None);
        assert_eq!(ev.get("BootStartTime"), None);
    }

    #[test]
    fn no_event_id_means_not_an_event() {
        assert!(parse_event_xml("<Event><System></System></Event>").is_none());
        assert!(parse_event_xml("").is_none());
        assert!(parse_event_xml("not xml at all").is_none());
    }

    /// 真机烟雾测试：无论有没有权限，都必须**明确地**告诉我们属于哪种情况，
    /// 而不是返回一个含糊的空结果。
    #[test]
    fn real_machine_read_reports_a_definite_state() {
        match read_boot_events() {
            BootLogOutcome::Events(v) => {
                println!("读到 {} 条开机事件", v.len());
                assert!(!v.is_empty(), "Events 分支不该是空列表");
            }
            BootLogOutcome::AccessDenied => {
                // 本机（非提权）实测走这条分支
                println!("权限不足——这是预期结果：该 channel 没有给普通用户 ACE");
            }
            BootLogOutcome::Unavailable(why) => {
                println!("日志不可用：{why}");
            }
        }
    }
}
