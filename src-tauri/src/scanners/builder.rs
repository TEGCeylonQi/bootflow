//! `StartupItem` 构造器。
//!
//! 为什么需要它：`StartupItem` 有三十多个字段，而四个扫描器要构造的
//! 是**同一种东西**——差异只在"从哪读到的"和"原始记录长什么样"。
//! 如果每个扫描器各自 `StartupItem { ... }` 一遍，会出现两种退化：
//!
//! 1. **漏字段**：将来加一个字段，四个地方都要改，漏一个就是一处静默的默认值。
//! 2. **不一致**：比如一个扫描器记得调 `sign::inspect()`，另一个忘了，
//!    结果同样的程序在不同分组下风险评级不同。
//!
//! 所以这里把「哪些字段是扫描器提供的」和「哪些是统一派生出来的」分开：
//! 前者由 builder 方法显式给，后者在 `build()` 里统一算。
//!
//! **签名信息在 build() 里统一读取**，这是关键——`derive_kind()` 依赖
//! `is_os_component`，四个扫描器必须用同一份判定，否则「系统组件」
//! 分组会随来源不同而漂移。

use serde_json::json;

use crate::model::{
    derive_kind, BootPhase, DesiredState, DiagnosticInfo, ItemKind, ItemTiming, NameSource,
    RiskLevel, Scope, SourceKind, StartupItem, ValidityStatus,
};
use crate::util::{approved, sign};

pub struct ItemBuilder {
    source: SourceKind,
    name: String,
    scope: Scope,
    location: String,
    command: String,
    resolved_path: String,
    args: Vec<String>,
    enabled: bool,
    boot_phase: BootPhase,
    /// 显式指定的友好名。服务用 DisplayName、计划任务用 Description——
    /// 它们比 exe 的版本信息更贴近语境，因此优先级更高。
    display_name: Option<(String, NameSource)>,
    validity: ValidityStatus,
    validity_detail: Option<String>,
    diagnostics: Vec<DiagnosticInfo>,
    /// 强制覆盖"是否 Windows 自带组件"，见 `os_component()`。
    force_os_component: Option<bool>,
    raw: serde_json::Value,
}

impl ItemBuilder {
    pub fn new(source: SourceKind, name: impl Into<String>) -> Self {
        let scope = match source {
            SourceKind::StartupFolderMachine
            | SourceKind::RunMachine
            | SourceKind::RunMachine32
            | SourceKind::RunOnceMachine
            | SourceKind::RunOnceMachine32
            | SourceKind::Service => Scope::Machine,
            _ => Scope::User,
        };

        Self {
            source,
            name: name.into(),
            scope,
            location: String::new(),
            command: String::new(),
            resolved_path: String::new(),
            args: Vec::new(),
            // 服务与计划任务各有自己的启用状态，不由 StartupApproved 决定；
            // 默认 true，由对应的扫描器按实际读到的值覆盖。
            enabled: true,
            boot_phase: BootPhase::Unknown,
            display_name: None,
            validity: ValidityStatus::Ok,
            validity_detail: None,
            diagnostics: Vec::new(),
            force_os_component: None,
            raw: serde_json::Value::Null,
        }
    }

    pub fn scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    pub fn location(mut self, location: impl Into<String>) -> Self {
        self.location = location.into();
        self
    }

    /// 原始命令行（用于展示与诊断）。
    pub fn command(mut self, command: impl Into<String>) -> Self {
        self.command = command.into();
        self
    }

    /// 展开环境变量后的可执行文件路径。
    pub fn target(mut self, path: impl Into<String>) -> Self {
        self.resolved_path = path.into();
        self
    }

    pub fn args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn boot_phase(mut self, phase: BootPhase) -> Self {
        self.boot_phase = phase;
        self
    }

    /// 显式指定友好名（服务的 DisplayName、计划任务的 Description）。
    ///
    /// 计划任务扫描器（T6）在目标是通用宿主程序（`cmd.exe` / `rundll32.exe`）
    /// 时用它——那些 exe 的版本信息说明不了任务要干什么。
    /// 其余情况**不要**设：让 `build()` 走 exe 版本信息，同一个程序在
    /// Run 键与计划任务里才会显示成同一个名字。
    pub fn display_name(mut self, name: impl Into<String>, from: NameSource) -> Self {
        let name = name.into();
        if !name.trim().is_empty() {
            self.display_name = Some((name, from));
        }
        self
    }

    /// 扫描器已经能给出更具体的有效性结论时使用
    /// （例如它知道这是"已被停用但仍留在注册表里"）。T7 的服务扫描器会用。
    #[allow(dead_code)]
    pub fn validity(mut self, status: ValidityStatus, detail: impl Into<String>) -> Self {
        self.validity = status;
        self.validity_detail = Some(detail.into());
        self
    }

    /// 扫描器自己发现的问题（T6 用它说明"这个任务还会一并启动别的程序"、
    /// "它是被哪一级开关关掉的"）。
    pub fn diagnostic(mut self, d: DiagnosticInfo) -> Self {
        self.diagnostics.push(d);
        self
    }

    /// 强制指定"这是 Windows 自带组件"。
    ///
    /// 为什么需要这个口子：`derive_kind()` 靠 `signer.is_os_component` 判
    /// 「系统组件」，而那个字段的来源是**目标文件在不在系统目录**。
    /// 计划任务里有一大批动作是 COM 处理器、**根本没有目标文件**——
    /// 本机实测 55 项里有 23 项是这种。它们全都是 `\Microsoft\Windows\` 下
    /// Windows 自带的维护任务，却会因为"没有路径 → 不是系统组件"
    /// 被归到计划任务那一堆里，和 Clash Verge、PowerToys 混在一起。
    /// 用户看不出区别，只会觉得这份清单又长又乱。
    ///
    /// 只允许**置为 true**：判断"这确实是 Windows 自己的"有明确依据
    /// （任务路径前缀），而反过来断言"这绝不是系统组件"没有，
    /// 硬关会把 System32 里的真系统组件误判成第三方。
    pub fn os_component(mut self) -> Self {
        self.force_os_component = Some(true);
        self
    }

    /// 原始记录，留给后续的写操作做回滚审计。
    pub fn raw(mut self, value: serde_json::Value) -> Self {
        self.raw = value;
        self
    }

    pub fn build(self) -> StartupItem {
        // ── 统一读取文件元信息 ──
        // 空路径（系统注入项、无路径的 COM 触发项）直接跳过，
        // 避免拿空字符串去调 Shell / 验签。
        let meta = if self.resolved_path.trim().is_empty() {
            sign::unknown()
        } else {
            sign::inspect(&self.resolved_path)
        };

        // 友好名优先级：扫描器显式给的 > exe 版本信息 > 无（前端回退到 name）
        let (display_name, name_from) = match self.display_name {
            Some((n, f)) => (Some(n), Some(f)),
            None => (meta.display_name.clone(), meta.name_from),
        };

        // 扫描器有更可靠的"这是 Windows 自带组件"证据时以它为准。
        // 只覆盖 is_os_component 一个字段：发布者、签名有效性仍以实际文件为准。
        let mut signer = meta.signer;
        if self.force_os_component == Some(true) {
            signer.is_os_component = true;
        }

        let kind: ItemKind = derive_kind(self.source, &signer);

        let raw = if self.raw.is_null() {
            json!({ "source": format!("{:?}", self.source), "name": self.name })
        } else {
            self.raw
        };

        StartupItem {
            // 这两个由 pipeline 统一派生，扫描器留空即可
            id: String::new(),
            identity_key: String::new(),

            source: self.source,
            name: self.name,
            kind,
            display_name,
            name_from,
            summary: None,

            command: self.command,
            resolved_path: self.resolved_path,
            args: self.args,
            location: self.location,
            scope: self.scope,
            enabled: self.enabled,

            signer,

            // 图标由 `get_icons` command 按需批量取，扫描阶段不做——
            // 几十上百个 Shell 调用会把首扫拖慢好几秒
            icon_data: None,

            // 风险评级归 T8 的诊断引擎统一计算；在此之前一律 Safe，
            // 空着比编一个看起来专业的分值诚实
            risk: RiskLevel::Safe,
            risk_reasons: Vec::new(),
            diagnostics: self.diagnostics,

            boot_phase: self.boot_phase,
            timing: ItemTiming::default(),

            validity: self.validity,
            validity_detail: self.validity_detail,
            recommendation: None,
            duplicate_of: None,

            raw,

            // 【预留】v1.0 恒为空
            desired: DesiredState::default(),
            snapshot_ref: None,
        }
    }
}

/// 这个目录项是否应当被跳过。
///
/// `desktop.ini` 是文件夹自定义图标用的，`Thumbs.db` 是资源管理器的缩略图缓存，
/// 两者都会出现在启动文件夹里，且都不是启动项。以 `.` 开头的同理。
pub fn is_ignored_file_name(file_name: &str) -> bool {
    file_name.starts_with('.')
        || file_name.eq_ignore_ascii_case("desktop.ini")
        || file_name.eq_ignore_ascii_case("thumbs.db")
}

/// 从文件名取一个可读的名称（去掉扩展名）。
pub fn displayable_name(file_name: &str) -> String {
    match file_name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => file_name.to_string(),
    }
}

/// 便捷封装：按来源去 `StartupApproved` 查启用状态。
pub fn approved_enabled(source: SourceKind, name: &str) -> bool {
    approved::is_enabled(source, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignored_files_are_skipped() {
        assert!(is_ignored_file_name("desktop.ini"));
        assert!(is_ignored_file_name("Desktop.INI"));
        assert!(is_ignored_file_name("Thumbs.db"));
        assert!(is_ignored_file_name(".hidden"));
        // 带空格与括号的普通快捷方式不能被误判
        assert!(!is_ignored_file_name("Corsair iCUE.lnk"));
    }

    #[test]
    fn displayable_name_strips_extension() {
        assert_eq!(displayable_name("Steam.lnk"), "Steam");
        assert_eq!(displayable_name("noext"), "noext");
        // 以点开头的文件名没有"主干"可言，应原样保留而不是削成空串
        assert_eq!(displayable_name(".gitkeep"), ".gitkeep");
        // 多点文件名只削最后一段
        assert_eq!(displayable_name("a.b.exe"), "a.b");
    }

    #[test]
    fn builder_defaults_are_sane() {
        let it = ItemBuilder::new(SourceKind::RunUser, "X")
            .target(r"D:\__bootflow_missing__\x.exe")
            .build();

        assert!(!it.signer.is_signed);
        assert_eq!(it.risk, RiskLevel::Safe);
        assert!(it.icon_data.is_none());
        assert!(it.raw.is_object(), "raw 不应为 null");
        assert_eq!(it.scope, Scope::User);
    }

    #[test]
    fn explicit_display_name_wins_over_version_info() {
        let it = ItemBuilder::new(SourceKind::Service, "LocalSend Helper")
            .target(r"D:\__bootflow_missing__\x.exe")
            .display_name("LocalSend 助手服务", NameSource::ServiceDisplayName)
            .build();

        assert_eq!(it.display_name.as_deref(), Some("LocalSend 助手服务"));
        assert_eq!(it.name_from, Some(NameSource::ServiceDisplayName));
    }

    #[test]
    fn machine_sources_default_to_machine_scope() {
        let a = ItemBuilder::new(SourceKind::StartupFolderMachine, "a").build();
        let b = ItemBuilder::new(SourceKind::StartupFolderUser, "b").build();
        assert_eq!(a.scope, Scope::Machine);
        assert_eq!(b.scope, Scope::User);
    }
}
