//! 处置建议引擎。
//!
//! ⚠️ 三条铁律：
//!
//! 1. **只给建议，不给行动**。v1.0 不提供任何执行入口。用户看到"建议停用"，
//!    但按钮不存在——执行能力要到 v1.5 有了快照与回滚才开放。
//! 2. **必须说人话**。`reason` 里出现 `MANUAL_BUT_SHOULD_AUTO` 这种码就是失败。
//!    用户读完这句话应该能自己判断该不该听我们的。
//! 3. **不确定就说不确定**。`confidence` 低时措辞保守，用"可能""建议确认"，
//!    不要用祈使句命令用户做事。
//!
//! 返回 `None` 表示"没有意见"——界面就不显示任何建议标记。
//! 大多数启动项应该是 `None`，界面才干净。

use crate::model::{
    Confidence, ItemKind, Recommendation, RecommendationAction, RiskLevel, StartupItem,
    ValidityStatus,
};

/// 诊断码。
///
/// 定义在 `diag::codes`（诊断的产地），这里只做转发。
/// 建议引擎是诊断的**消费方**——它读诊断来决定说什么，
/// 但不该拥有诊断码的定义权。把码留在"谁发现的"那一层，
/// 才能保证一个码只描述一个事实。
pub use crate::diag::codes;

/// 对单个启动项给出处置建议。
pub fn advise(item: &StartupItem) -> Option<Recommendation> {
    // ── 第一优先级：目标已经不存在。
    //    这是唯一敢说"高置信"的场景——文件都没了，留着注册项没有任何意义。
    if item.validity == ValidityStatus::MissingTarget {
        return Some(Recommendation {
            action: RecommendationAction::Remove,
            reason: "这个程序已经不在电脑上了，它的启动记录是卸载时留下的残留。\
                     清理它不会影响任何正在使用的东西（它本来也启动不了）。"
                .to_string(),
            confidence: Confidence::Measured,
        });
    }

    if item.validity == ValidityStatus::NotExecutable {
        return Some(Recommendation {
            action: RecommendationAction::Remove,
            reason: "这个启动项目标指向的不是一个可运行的程序，系统实际上无法执行它。\
                     它可能来自一次失败的安装或错误的配置。"
                .to_string(),
            confidence: Confidence::Measured,
        });
    }

    if item.validity == ValidityStatus::Unreachable {
        return Some(Recommendation {
            action: RecommendationAction::Review,
            reason: "它指向的位置当前访问不到。如果你现在没连接那个网络位置，\
                     这属于正常现象，不用处理；如果这个位置已经不用了，可以考虑清理。"
                .to_string(),
            confidence: Confidence::Estimated,
        });
    }

    // ── 第二优先级：同一个程序的冗余入口。这里要分两档，不能合并：
    //
    //    · **已停用的那条** → 建议清理。用户已经不用它了，记录留着没有任何作用，
    //      删掉也不影响那个在用的入口。这是唯一敢在"重复"场景说 Remove 的情况。
    //    · **仍启用的那条** → 只建议停用，不说删除。哪一条该留取决于用户自己的
    //      使用习惯（也许他就是要两个入口），把选择权留给他，而且停用可逆。
    if item.validity == ValidityStatus::DisabledRemnant {
        return Some(Recommendation {
            action: RecommendationAction::Remove,
            reason: "这个程序已经有一个在用的启动入口，而这一条是重复的、\
                     而且已经被停用了。它现在不起任何作用，清理掉不会影响\
                     那个还在生效的入口。"
                .to_string(),
            confidence: Confidence::Measured,
        });
    }

    if item.validity == ValidityStatus::Duplicate {
        return Some(Recommendation {
            action: RecommendationAction::Disable,
            reason: "这个程序在系统里注册了不止一个启动入口，它们都会被拉起。\
                     保留一个就够——多出来的入口不会让它启动得更快，\
                     反而容易在你想关掉它的时候漏掉一个。".to_string(),
            confidence: Confidence::Measured,
        });
    }

    // ── 分界线：以下建议都建立在"它现在会运行"这个前提上 ──
    //
    // 已经停用的项不会运行。对它说"它会在开机早期运行、影响面大"、
    // "系统记录过它启动很慢"、"它的启动方式被改成了手动"，全都是错的——
    // 用户会立刻发现这些话跟眼前这条"已停用"的记录对不上，
    // 然后开始怀疑这个工具的其他判断。
    //
    // 放在这个位置而不是最前面，是因为上面几条（目标没了 / 重复 / 够不着）
    // 与"是否运行"无关，对已停用的项同样成立，也依然值得说。
    if !item.enabled {
        return None;
    }

    // ── 第三优先级：全局注入。这是风险最高的一类，但它常常是用户
    //    自己装的（比如字体渲染工具），所以措辞是"了解一下"而不是"干掉它"。
    if item.kind == ItemKind::Hook {
        return Some(Recommendation {
            action: RecommendationAction::Review,
            reason: "它不是一个独立运行的程序，而是会被装进其它程序里一起运行。\
                     因此它的影响范围远超它自己——出问题时影响面也更大。\
                     如果这个工具是你自己装的、且在用，可以保留。".to_string(),
            confidence: Confidence::Estimated,
        });
    }

    // ── 第四：缺失数字签名。仅在"位置或形态让它更重要"时才提，
    //    否则会淹没在大量未签名的小工具里，变成噪音。
    if !item.signer.is_signed && needs_attention_when_unsigned(item) {
        return Some(Recommendation {
            action: RecommendationAction::Review,
            reason: "这个项没有数字签名，无法核验它由谁发布。\
                     它又位于所有用户共享的位置（或会在开机早期运行），\
                     因此影响面比普通启动项大。建议你确认一下是否认识它。"
                .to_string(),
            confidence: Confidence::Estimated,
        });
    }

    // ── 第五：命中系统记录的慢启动证据。这是 Event 103 的硬数据，
    //    所以比其他推测更值得用户花时间看一眼。
    if let Some(d) = item
        .diagnostics
        .iter()
        .find(|d| d.code == codes::SLOW_START)
    {
        let ev = d
            .evidence
            .as_deref()
            .map(|e| format!("（{e}）"))
            .unwrap_or_default();
        return Some(Recommendation {
            action: RecommendationAction::Review,
            reason: format!(
                "系统自己记录了它上一次开机启动很慢{ev}。\
                 这是 Windows 的实测数据，不是我们的推测。\
                 可以考虑把它改成更晚启动，或者确认一下它是否真的需要开机就运行。"
            ),
            confidence: Confidence::Measured,
        });
    }

    // ── 第六：状态与预期不符（典型是 Office 的启动服务被优化软件改成了手动）。
    if item
        .diagnostics
        .iter()
        .any(|d| d.code == codes::MANUAL_BUT_SHOULD_AUTO)
    {
        return Some(Recommendation {
            action: RecommendationAction::Review,
            reason: "它的启动方式被改成了「手动」，但按它的用途应该由系统自动拉起。\
                     这通常是系统优化工具留下的结果，会导致依赖它的程序\
                     第一次打开时卡住不动。".to_string(),
            confidence: Confidence::Measured,
        });
    }

    // 其余一律不给建议。界面保持干净，用户才愿意看真正重要的那几条。
    None
}

/// 未签名是否值得提。
///
/// 判据是"影响面"：只影响当前用户的普通启动项，未签名很常见（小工具、自编译程序），
/// 提了也是噪音；而全局位置或开机早期运行的东西，一旦有问题影响面大得多。
fn needs_attention_when_unsigned(item: &StartupItem) -> bool {
    use crate::model::Scope;

    // ⚠️ 禁改区的项直接放过。理由必须写在这里，否则以后一定会被人当成漏判补上。
    //
    // 注意这条规则**不是因为签名判断不准**——那件事已经解决了：
    // `util::sign::verify_signature` 现在会在内嵌验签失败后走 catalog 验签
    // （`CryptCATAdmin*` + `WTD_CHOICE_CATALOG`），系统目录里那些
    // 不内嵌签名的文件（本机实测 ctfmon.exe / rundll32.exe）现在能被正确
    // 识别为已签名。
    //
    // 保留豁免的原因是**建议本身没有意义**：禁改区意味着界面上根本没有
    // 修改入口。对一个用户永远无法处置的项说"建议你确认一下它的来源"，
    // 除了制造焦虑什么也做不到——而这类工具最不该做的就是这件事。
    //
    // 判据用 `risk == Locked` 而不是 `kind == System`：
    // 后者只覆盖"目标在系统目录"这一种情况，而禁改区还包括
    // 那些路径可能在别处、但语义上属于系统命脉的核心服务（见 `diag::risk`）。
    if item.risk == RiskLevel::Locked {
        return false;
    }

    if item.scope == Scope::Machine {
        return true;
    }

    matches!(
        item.boot_phase,
        crate::model::BootPhase::Kernel
            | crate::model::BootPhase::Driver
            | crate::model::BootPhase::Devices
            | crate::model::BootPhase::Smss
    ) || item.risk == RiskLevel::High
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn base_item() -> StartupItem {
        StartupItem {
            id: "t".into(),
            source: SourceKind::RunUser,
            identity_key: "k".into(),
            name: "test".into(),
            kind: ItemKind::App,
            display_name: None,
            name_from: None,
            summary: None,
            command: "x".into(),
            resolved_path: "x".into(),
            args: vec![],
            location: "l".into(),
            scope: Scope::User,
            enabled: true,
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
    fn dead_target_recommends_cleanup_with_human_reason() {
        let mut it = base_item();
        it.validity = ValidityStatus::MissingTarget;
        let r = advise(&it).expect("应当给出建议");
        assert_eq!(r.action, RecommendationAction::Remove);
        // 理由必须是人话，不能出现诊断码
        assert!(!r.reason.contains("DEAD_TARGET"));
        assert!(r.reason.contains("残留"));
    }

    #[test]
    fn healthy_signed_item_gets_no_noise() {
        let it = base_item();
        assert!(advise(&it).is_none(), "正常项不应产生任何建议噪音");
    }

    #[test]
    fn duplicate_is_advised_to_disable_not_delete() {
        let mut it = base_item();
        it.validity = ValidityStatus::Duplicate;
        let r = advise(&it).unwrap();
        assert_eq!(
            r.action,
            RecommendationAction::Disable,
            "仍在生效的重复项建议停用而非删除——停用可逆"
        );
        assert!(
            r.reason.contains("都会被拉起"),
            "要说清后果是两个入口都会启动，而不只是「记录冗余」"
        );
    }

    #[test]
    fn disabled_remnant_is_advised_to_remove() {
        // 已停用的重复项和仍在生效的重复项，处置方式必须不同：
        // 对一条**已经停用**的记录建议"停用它"，用户只会觉得这软件没看懂状况。
        let mut it = base_item();
        it.validity = ValidityStatus::DisabledRemnant;
        it.enabled = false;

        let r = advise(&it).expect("应给出建议");
        assert_eq!(
            r.action,
            RecommendationAction::Remove,
            "已经停用的冗余记录，建议的是清理而不是停用"
        );
        assert!(
            r.reason.contains("停用"),
            "理由里要交代它已经被停用过这件事"
        );
        assert!(
            r.reason.contains("不会影响"),
            "要让用户放心：清理它不会动到那个在用的入口"
        );
    }

    #[test]
    fn unreachable_is_review_not_remove() {
        // 拔掉的移动硬盘、没连的 VPN —— 不能建议清理
        let mut it = base_item();
        it.validity = ValidityStatus::Unreachable;
        let r = advise(&it).expect("应给出建议");
        assert_eq!(
            r.action,
            RecommendationAction::Review,
            "位置暂时够不着，不等于该删"
        );
    }

    #[test]
    fn unsigned_on_machine_scope_is_flagged() {
        let mut it = base_item();
        it.scope = Scope::Machine;
        it.signer.is_signed = false;
        let r = advise(&it).expect("全局位置的未签名项应提示");
        assert_eq!(r.action, RecommendationAction::Review);
    }

    #[test]
    fn locked_items_never_get_advice_about_signature() {
        // 禁改区的项在界面上没有任何修改入口。对用户永远无法处置的东西
        // 说"建议你确认一下它的来源"，只会制造焦虑。
        let mut it = base_item();
        it.kind = ItemKind::System;
        it.risk = RiskLevel::Locked;
        it.scope = Scope::Machine;
        it.signer.is_signed = false;
        it.signer.is_os_component = true;

        assert!(
            advise(&it).is_none(),
            "禁改区的项不该收到「未签名，请确认」这类建议"
        );
    }

    #[test]
    fn locked_exemption_does_not_leak_to_manageable_items() {
        // 反面：豁免必须只覆盖禁改区。一个**可以**处置的全局未签名项，
        // 仍然应该收到提示——否则这条豁免就吃掉了整个未签名检测。
        let mut it = base_item();
        it.scope = Scope::Machine;
        it.signer.is_signed = false;
        it.risk = RiskLevel::Safe; // 不是禁改区

        assert!(
            advise(&it).is_some(),
            "可处置的全局未签名项仍应被提示，豁免不能扩大化"
        );
    }

    #[test]
    fn disabled_item_gets_no_advice_about_how_it_runs() {
        // 一个已经停用的项不会再运行。对它说"启动很慢""没有签名""影响面大"
        // 全部与事实矛盾——用户看到的就是一条"已停用"的记录。
        let mut it = base_item();
        it.enabled = false;
        it.scope = Scope::Machine;
        it.signer.is_signed = false;
        it.diagnostics = vec![DiagnosticInfo {
            code: codes::SLOW_START.to_string(),
            severity: RiskLevel::Medium,
            message: "x".into(),
            evidence: None,
        }];

        assert!(
            advise(&it).is_none(),
            "已停用的项不该收到任何与「它运行起来怎样」相关的建议"
        );
    }

    #[test]
    fn disabled_item_still_gets_cleanup_advice_when_target_is_gone() {
        // 但"目标已经不存在"与是否运行无关，对停用项依然成立
        let mut it = base_item();
        it.enabled = false;
        it.validity = ValidityStatus::MissingTarget;
        let r = advise(&it).expect("目标丢失的停用项仍应给出清理建议");
        assert_eq!(r.action, RecommendationAction::Remove);
    }
}
