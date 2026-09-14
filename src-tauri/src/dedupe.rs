//! 跨来源去重。
//!
//! 为什么这件事重要：同一个程序经常有**多个自启入口**——安装程序往 HKLM Run
//! 写一个、又在全局启动文件夹放个快捷方式、再注册个计划任务。用户看到三条
//! 记录会以为要处理三件事，实际是一件事。
//!
//! 这里提供三级判定：
//! - `identity_key`：**精确身份** = 路径 + 参数。用于判断"是否同一个启动项"。
//! - `program_key`：**程序身份** = 仅路径。用于识别"同一程序的多个入口"。
//! - `ValidityStatus::{Duplicate, DisabledRemnant}`：同一程序的多个入口里，
//!   **仍在启用**的那条是"两个入口都会被拉起"（真实性能问题），
//!   **已经停用**的那条是"不再起作用的冗余记录"（纯残渣）。
//!   两者性质不同，不能合并成一个"重复"标签——理由见 `mark_duplicates`。
//!
//! 参数**不做大小写归一**是刻意的。归一小写能多合并一些情况，但也可能把
//! `-Server A` 和 `-server B` 这类语义不同的配置误判成同一条。对去重功能而言，
//! 误合并（把两件事说成一件）比漏合并（少提醒一次）的危害大得多。

use std::collections::HashMap;

use uuid::Uuid;

use crate::model::{StartupItem, ValidityStatus};

/// uuid v5 命名空间。**这是个固定值，不要改**——
/// 改了会导致所有启动项的 id 变化，进而让 v1.5 的快照比对全部失效。
const BOOTFLOW_NS: Uuid = Uuid::from_bytes([
    0x6b, 0x1f, 0x7c, 0x2a, 0x4d, 0x3e, 0x4a, 0x11, 0x9c, 0x8b, 0x2f, 0x1a, 0x3d, 0x5e, 0x7f, 0x90,
]);

/// 归一化路径：去引号、统一分隔符、转小写。
///
/// 转小写是安全的：NTFS 默认大小写不敏感，`C:\App\A.exe` 和 `c:\app\a.exe`
/// 在实践中就是同一个文件。
pub fn normalize_path(path: &str) -> String {
    path.trim()
        .trim_matches('"')
        .trim()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

/// 归一化参数：逐项 trim、丢弃空项、用单空格连接。**不改变大小写。**
pub fn normalize_args(args: &[String]) -> String {
    args.iter()
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 精确身份键 = 路径 + 参数
pub fn identity_key(resolved_path: &str, args: &[String]) -> String {
    let p = normalize_path(resolved_path);
    let a = normalize_args(args);
    if a.is_empty() {
        p
    } else {
        format!("{p}#{a}")
    }
}

/// 程序身份键 = 仅路径。用于识别"同一程序的多个自启入口"。
pub fn program_key(resolved_path: &str) -> String {
    normalize_path(resolved_path)
}

/// 来源内的**区分符** —— 回答"这一项凭什么和同一来源里的其他项区别开"。
///
/// 【为什么必须有这个东西】
/// `identity_key` 是**跨来源**去重用的键，它天然**不保证在同一个来源内唯一**。
/// 真机上典型的两类撞车：
///
/// - 一个 `svchost.exe -k netsvcs -p` 承载十几个服务，身份键完全一样
/// - 注册表里同一个程序的两条记录（如本地实测的两条 LocalSend），路径也一样
///
/// 只用「来源 + 身份键」派生 id，这些项会拿到**完全相同的 id**。后果不是"看着别扭"：
///
/// - React 列表 key 冲突 → 渲染错乱、勾选串味
/// - `selectedId` 指向歧义 → 点 A 弹出的是 B 的详情
/// - 编排草稿按 id 存 → 想改 A，实际改动记到了 B 头上
///
/// 本机实测：273 项里只有 157 个唯一 id，也就是**四成以上的项在互相冒名**。
/// 这个 bug 靠看界面几乎发现不了（列表看着挺正常），是"id 必须唯一"这条断言逼出来的。
///
/// 区分符取「注册位置 + 原始名称」：两者都是系统里的持久值，
/// 稳定性不受影响；组合起来能唯一确定一条注册记录。
///
/// 中间用单元分隔符 `\u{1f}` 而不是冒号或竖线：分隔符本身不能出现在
/// 被拼接的字符串里，否则 `a:b` + `c` 和 `a` + `b:c` 会拼出同一个种子。
/// 位置与名称里出现控制字符是不可能的，所以这个分隔符是安全的。
pub fn discriminator_of(it: &StartupItem) -> String {
    format!("{}\u{1f}{}", it.location, it.name)
}

/// 由「来源 + 身份键 + 来源内区分符」派生稳定 UUID。
///
/// 稳定性是硬要求：同一台机器上今天扫和明天扫，同一个启动项必须得到同一个 id，
/// 否则 v1.5 的快照比对会认为"全都变了"。
///
/// 唯一性同样是硬要求，而且它比稳定性更容易被忽略 —— 见 `discriminator_of`。
pub fn stable_id(source: &str, identity: &str, discriminator: &str) -> String {
    Uuid::new_v5(
        &BOOTFLOW_NS,
        format!("{source}\u{1f}{identity}\u{1f}{discriminator}").as_bytes(),
    )
    .to_string()
}

/// 标记重复项。返回被标记的数量。
///
/// 规则：
/// 1. 已被判定为「坏项」（目标缺失/不可执行）的不参与重复判定——
///    它的问题更严重，说"它和别的重复"会分散注意力。
/// 2. **只有自启入口类来源参与**（见 `SourceKind::is_autostart_entry`）。
///    服务与计划任务按 exe 路径去重会造成大规模误报。
/// 3. 按 `program_key`（仅路径，不含参数）分组，组内 >1 时保留一个"代表项"，
///    其余按状态分两类：
///    - **仍启用** → `Duplicate`（它还占着一个自启位，两个入口都会被拉起）
///    - **已停用** → `DisabledRemnant`（一条不再起作用的冗余记录）
/// 4. 代表项的挑选顺序：**先看是否启用**（启用的更该保留），再看来源优先级，
///    最后用 id 兜底保证稳定。
pub fn mark_duplicates(items: &mut [StartupItem]) -> usize {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();

    for (idx, it) in items.iter().enumerate() {
        if it.validity.is_broken() {
            continue;
        }
        // 服务/计划任务的"重复"是另一回事，不在这里判
        if !it.source.is_autostart_entry() {
            continue;
        }
        let key = program_key(&it.resolved_path);
        if key.is_empty() {
            continue;
        }
        groups.entry(key).or_default().push(idx);
    }

    let mut marked = 0usize;

    for (_key, idxs) in groups {
        if idxs.len() < 2 {
            continue;
        }

        // 挑选代表项：启用优先，其次来源优先级高者，最后按 id。
        //
        // id 这一层不是可有可无的：前两个条件相同时（两个项来源一样、
        // 都启用或都停用），谁被标成"多余"就完全由分组用的 HashMap
        // 迭代顺序决定。那意味着**同一次扫描跑两遍可能给出不同结论**，
        // 用户会看到"上次说这条多余，这次说那条多余"。
        let rep = idxs.iter().copied().min_by(|&a, &b| {
            let ia = &items[a];
            let ib = &items[b];
            (u8::from(!ia.enabled), ia.source.keep_rank())
                .cmp(&(u8::from(!ib.enabled), ib.source.keep_rank()))
                .then_with(|| ia.id.cmp(&ib.id))
        });

        let Some(rep_idx) = rep else { continue };
        let rep_id = items[rep_idx].id.clone();
        let rep_identity = items[rep_idx].identity_key.clone();
        let rep_name = items[rep_idx]
            .display_name
            .clone()
            .unwrap_or_else(|| items[rep_idx].name.clone());

        for i in idxs {
            if i == rep_idx {
                continue;
            }

            let same_identity = items[i].identity_key == rep_identity;
            let still_enabled = items[i].enabled;

            // ⭐ 关键区分：**已停用**的重复项和仍启用的重复项，性质完全不同。
            //
            // 用户主动停用某个启动项（比如不想让 Parsec 开机自启）是**正常配置**，
            // 我们绝不能碰。但"同一个程序已经有一个在用的入口，另一条却还停在那里"
            // 就纯属残渣了——它既没有被使用，也没有存在的理由。
            //
            // 混为一谈的后果很严重：要么把用户的正常配置说成"可以清理"，
            // 要么对着一条已停用的记录建议"停用它"（现在就是这么干的，属于废话）。
            items[i].validity = if still_enabled {
                ValidityStatus::Duplicate
            } else {
                ValidityStatus::DisabledRemnant
            };

            items[i].duplicate_of = Some(rep_id.clone());
            items[i].validity_detail = Some(if !still_enabled {
                format!("和「{rep_name}」指向同一个程序，而这一条已经停用了")
            } else if same_identity {
                format!("和「{rep_name}」完全重复：同一个程序、同样的启动参数")
            } else {
                format!("和「{rep_name}」指向同一个程序，但启动参数不同，两个入口都会被拉起")
            });

            marked += 1;
        }
    }

    marked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    /// 造一个够用的启动项。字段只填去重会读到的那些。
    fn make(id: &str, source: SourceKind, path: &str, args: &[&str], enabled: bool) -> StartupItem {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();

        StartupItem {
            id: id.to_string(),
            source,
            identity_key: identity_key(path, &args),
            name: "n".into(),
            kind: ItemKind::App,
            display_name: None,
            name_from: None,
            summary: None,
            command: path.into(),
            resolved_path: path.into(),
            args,
            location: "l".into(),
            scope: Scope::User,
            enabled,
            signer: SignerInfo::default(),
            icon_data: None,
            risk: RiskLevel::Safe,
            risk_reasons: vec![],
            diagnostics: vec![],
            boot_phase: BootPhase::Logon,
            timing: ItemTiming::default(),
            validity: ValidityStatus::Ok,
            validity_detail: None,
            recommendation: None,
            duplicate_of: None,
            raw: serde_json::Value::Null,
            desired: DesiredState::default(),
            snapshot_ref: None,
        }
    }

    #[test]
    fn path_normalization_is_case_and_quote_insensitive() {
        assert_eq!(normalize_path("\"C:\\App\\A.exe\""), "c:\\app\\a.exe");
        assert_eq!(normalize_path(" C:/App/A.exe "), "c:\\app\\a.exe");
    }

    #[test]
    fn identity_key_distinguishes_arguments() {
        let a = identity_key("C:\\App\\A.exe", &["--mode=x".into()]);
        let b = identity_key("C:\\App\\A.exe", &["--mode=y".into()]);
        let c = identity_key("C:\\App\\A.exe", &[]);
        assert_ne!(a, b, "参数不同应视为不同启动项");
        assert_ne!(a, c, "有无参数应视为不同启动项");
        assert_eq!(program_key("C:\\App\\A.exe"), program_key("\"c:/app/a.exe\""));
    }

    #[test]
    fn stable_id_is_deterministic() {
        let a = stable_id("RunUser", "c:\\app\\a.exe", "hkcu\\...\\run\u{1f}A");
        let b = stable_id("RunUser", "c:\\app\\a.exe", "hkcu\\...\\run\u{1f}A");
        assert_eq!(a, b, "同一台机器上重复扫描必须得到同一个 id");
        assert_ne!(
            a,
            stable_id("RunMachine", "c:\\app\\a.exe", "hkcu\\...\\run\u{1f}A"),
            "来源不同就是不同的项"
        );
        assert_ne!(
            a,
            stable_id("RunUser", "c:\\app\\a.exe", "hkcu\\...\\run\u{1f}B"),
            "来源内的注册位置/名称不同，就是两条不同的记录"
        );
    }

    /// 回归测试：**同一个命令行承载的多个服务必须拿到互不相同的 id**。
    ///
    /// 真机实测过这个 bug：一个 `svchost.exe -k netsvcs -p` 承载十几个服务，
    /// 身份键完全相同，于是 id 也完全相同。273 项里只有 157 个唯一 id。
    ///
    /// 危害是隐性的：列表看着正常，但点 A 弹出 B 的详情、勾选 A 会连带勾中 B。
    /// 靠看界面基本发现不了，只有"id 必须唯一"这条断言能逼出来。
    #[test]
    fn services_sharing_one_host_still_get_unique_ids() {
        let path = r"C:\Windows\System32\svchost.exe";
        let args = ["-k", "netsvcs", "-p"];

        let mut items: Vec<_> = ["Windefend", "EventLog", "Schedule", "Themes"]
            .iter()
            .map(|svc| {
                let mut it = make("", SourceKind::Service, path, &args, true);
                it.name = (*svc).to_string();
                it.location = format!("服务:\\{svc}");
                it
            })
            .collect();

        for it in items.iter_mut() {
            let did = stable_id(source_key(it.source), &it.identity_key, &discriminator_of(it));
            it.id = did;
        }

        let ids: std::collections::HashSet<_> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids.len(),
            items.len(),
            "同一宿主承载的服务拿到了相同 id —— 界面上的勾选与详情会互相串味"
        );
    }

    #[test]
    fn discriminator_survives_colon_in_location() {
        // 用普通分隔符拼接时，`a:b`+`c` 和 `a`+`b:c` 会拼出同一个种子。
        // 内容里的冒号在真机上到处都是（`服务:\WinDefend`），所以分隔符必须是
        // 内容里不可能出现的控制字符。
        let mut x = make("", SourceKind::Service, r"C:\a.exe", &[], true);
        x.location = "服务:A".into();
        x.name = "B".into();

        let mut y = make("", SourceKind::Service, r"C:\a.exe", &[], true);
        y.location = "服务".into();
        y.name = "A:B".into();

        assert_ne!(discriminator_of(&x), discriminator_of(&y));
    }

    /// `source_key` 是 pipeline 内部用的，这里只要一个稳定的字符串即可。
    fn source_key(s: SourceKind) -> &'static str {
        match s {
            SourceKind::Service => "Service",
            _ => "Other",
        }
    }

    /// 真实样本：本机 LocalSend 注册了两个入口，
    /// 一个是 `localsend_app`（用户已停用），一个是 `LocalSend --hidden`（在用）。
    #[test]
    fn disabled_duplicate_is_a_remnant_not_a_duplicate() {
        let path = r"E:\Tools\LocalSend\localsend_app.exe";
        let mut items = vec![
            make("keep", SourceKind::RunUser, path, &["--hidden"], true),
            make("dead", SourceKind::RunUser, path, &[], false),
        ];

        let n = mark_duplicates(&mut items);

        assert_eq!(n, 1);
        let disabled = items.iter().find(|i| i.id == "dead").unwrap();
        assert_eq!(
            disabled.validity,
            ValidityStatus::DisabledRemnant,
            "已停用的重复项要判成「已停用残留」——对一条已停用的记录建议停用是废话"
        );
        assert!(disabled.duplicate_of.is_some(), "仍要指向代表项");
        assert!(disabled
            .validity_detail
            .as_deref()
            .unwrap()
            .contains("已经停用"));

        let kept = items.iter().find(|i| i.id == "keep").unwrap();
        assert_eq!(kept.validity, ValidityStatus::Ok, "在用的那条不该被动");
    }

    #[test]
    fn both_enabled_duplicate_stays_duplicate() {
        let path = r"C:\App\a.exe";
        let mut items = vec![
            make("u", SourceKind::RunUser, path, &[], true),
            make("m", SourceKind::RunMachine, path, &[], true),
        ];

        mark_duplicates(&mut items);

        // 两个都启用时按来源优先级留全机的那个
        let machine = items.iter().find(|i| i.id == "m").unwrap();
        let user = items.iter().find(|i| i.id == "u").unwrap();

        assert_eq!(machine.validity, ValidityStatus::Ok, "全机项优先级更高");
        assert_eq!(
            user.validity,
            ValidityStatus::Duplicate,
            "两个入口都在生效，这才是真正意义上的「重复」"
        );
    }

    #[test]
    fn identical_entries_are_described_as_fully_duplicated() {
        let path = r"C:\App\a.exe";
        let mut items = vec![
            make("a", SourceKind::RunUser, path, &[], true),
            make("b", SourceKind::RunMachine, path, &[], true),
        ];

        mark_duplicates(&mut items);

        let dup = items.iter().find(|i| i.duplicate_of.is_some()).unwrap();
        assert!(
            dup.validity_detail.as_deref().unwrap().contains("完全重复"),
            "参数也一样时要说清是彻底重复，而不是「参数不同」"
        );
    }

    /// 这条是**防未来**的：T6/T7 把服务与计划任务接进来之后，
    /// 按 exe 路径分组会把 `svchost.exe` 的几十个服务全判成"重复入口"。
    #[test]
    fn services_are_never_deduplicated_by_path() {
        let mut items: Vec<_> = (0..5)
            .map(|i| {
                make(
                    &format!("s{i}"),
                    SourceKind::Service,
                    r"C:\Windows\System32\svchost.exe",
                    &[],
                    true,
                )
            })
            .collect();

        let n = mark_duplicates(&mut items);

        assert_eq!(n, 0, "一个 svchost.exe 承载几十个服务，按路径去重是灾难性的误报");
        assert!(items.iter().all(|i| i.validity == ValidityStatus::Ok));
    }

    #[test]
    fn scheduled_tasks_are_never_deduplicated_by_path() {
        // 同一个 backup.exe 可以既跑"每天 9 点"又跑"每周日 2 点"——
        // 它们不是重复，是两件不同的事
        let mut items = vec![
            make("t1", SourceKind::ScheduledTask, r"C:\App\backup.exe", &["daily"], true),
            make("t2", SourceKind::ScheduledTask, r"C:\App\backup.exe", &["weekly"], true),
        ];

        assert_eq!(mark_duplicates(&mut items), 0);
    }

    #[test]
    fn system_hooks_are_never_deduplicated_by_path() {
        let mut items = vec![
            make("h1", SourceKind::SystemHook, r"C:\a\inject.dll", &[], true),
            make("h2", SourceKind::SystemHook, r"C:\a\inject.dll", &[], true),
        ];
        assert_eq!(mark_duplicates(&mut items), 0);
    }

    #[test]
    fn flagged_duplicate_does_not_depend_on_enumeration_order() {
        // 两项来源相同、状态相同 —— 不靠 id 兜底时，谁被标"多余"
        // 完全由分组用的 HashMap 迭代顺序决定，同一次扫描跑两遍结论可能不同
        let path = r"C:\App\x.exe";
        let mut a = vec![
            make("id-a", SourceKind::RunUser, path, &[], true),
            make("id-b", SourceKind::RunUser, path, &[], true),
        ];
        let mut b = a.clone();
        b.reverse(); // 模拟枚举顺序变化

        mark_duplicates(&mut a);
        mark_duplicates(&mut b);

        let flagged = |v: &[StartupItem]| {
            v.iter()
                .find(|i| i.duplicate_of.is_some())
                .map(|i| i.id.clone())
                .unwrap()
        };

        assert_eq!(
            flagged(&a),
            flagged(&b),
            "被标重复的项必须与枚举顺序无关"
        );
    }

    #[test]
    fn broken_items_do_not_participate_in_dedup() {
        // 目标已失效的项，它的问题更严重；说"它和别的重复"会分散注意力
        let path = r"C:\App\gone.exe";
        let mut items = vec![
            make("a", SourceKind::RunUser, path, &[], true),
            make("b", SourceKind::RunMachine, path, &[], true),
        ];
        items[0].validity = ValidityStatus::MissingTarget;

        mark_duplicates(&mut items);

        assert_eq!(items[0].validity, ValidityStatus::MissingTarget, "坏项结论不该被覆盖");
        assert!(items.iter().all(|i| i.duplicate_of.is_none()));
    }
}
