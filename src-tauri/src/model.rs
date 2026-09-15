//! 核心数据模型 —— 前后端的唯一契约。
//!
//! ⚠️ 改动此文件时，必须同步修改 `src/types/model.ts`。
//! 两侧靠 `#[serde(rename_all = "camelCase")]` 对齐字段名，
//! 枚举则各自用 `rename_all` 精确匹配 TS 联合类型的字面量。
//!
//! 三条设计原则在本文件中的体现：
//! 1. **可逆性** —— `desired` / `snapshot_ref` 是预留位，v1.0 恒空，
//!    但它们现在就在结构里，避免 v1.5 引入迁移成本。
//! 2. **诚实** —— `ItemTiming::confidence` 三档强制区分实测/估算。
//! 3. **面向用户** —— `kind` / `display_name` / `name_from` / `summary`
//!    四个字段是"去术语化"的载体，界面层靠它们才能说人话。

use serde::{Deserialize, Serialize};

/// 启动项来源 —— 回答"它注册在哪里"。
///
/// 这是**技术分类**，界面主视图已不再用它分组（用户看不懂），
/// 但属性面板的技术详情、报告导出、以及后续写操作的定位都需要它。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceKind {
    StartupFolderUser,
    StartupFolderMachine,
    RunUser,
    RunMachine,
    RunMachine32,
    RunOnceUser,
    RunOnceMachine,
    RunOnceMachine32,
    ScheduledTask,
    Service,
    SystemHook,
}

impl SourceKind {
    /// 去重时用于挑选"代表项"的优先级，数字越小越优先保留。
    ///
    /// 排序理由：全局启动文件夹 > 用户启动文件夹 > 服务 > 计划任务 > Run。
    /// 前两者是"用户或安装程序明确放置的快捷方式"，语义最清晰；
    /// Run 键排在后面是因为命名最随意（`localsend_app` 这种机器名多来自这里）。
    pub fn keep_rank(self) -> u8 {
        match self {
            SourceKind::StartupFolderMachine => 0,
            SourceKind::StartupFolderUser => 1,
            SourceKind::Service => 2,
            SourceKind::ScheduledTask => 3,
            SourceKind::RunMachine => 4,
            SourceKind::RunMachine32 => 5,
            SourceKind::RunUser => 6,
            SourceKind::RunOnceMachine => 7,
            SourceKind::RunOnceMachine32 => 8,
            SourceKind::RunOnceUser => 9,
            SourceKind::SystemHook => 10,
        }
    }

    /// 是否属于「自启入口」类来源。
    ///
    /// 这个区分直接决定**路径级去重能不能做**，加它是因为不做区分会在
    /// T6/T7 接入后造成大规模误报：
    ///
    /// - 自启入口（启动文件夹 / Run / RunOnce）的一条记录 = 某个程序的一次启动。
    ///   同一个程序有两条记录就是冗余，去重有意义。
    /// - **服务不适用**：一个 `svchost.exe` 承载几十个服务，
    ///   `rundll32.exe` 也常常是好几个服务的宿主。按 exe 路径去重会把
    ///   这些完全正常的配置全部说成"重复入口"。
    /// - **计划任务不适用**：同一个 `backup.exe` 可以有"每天 9 点"和
    ///   "每周日 2 点"两个任务，它们不是重复，是两件事。
    /// - **系统注入项不适用**：它不是一次启动，而是一段被加载进别人的代码。
    ///
    /// 服务与计划任务的重复判定需要各自的语义（服务名、触发器、任务路径），
    /// 留到各自的任务里单独设计，不共用这一套。
    pub fn is_autostart_entry(self) -> bool {
        !matches!(
            self,
            SourceKind::Service | SourceKind::ScheduledTask | SourceKind::SystemHook
        )
    }
}

/// 「这是什么」—— 面向用户的类型维度，与 `SourceKind` **正交**。
///
/// 同一个程序可以「注册在 Run 键里」（source）同时是「应用程序」（kind）。
/// 界面优先展示 kind，因为理解"这是个服务"比理解"这在 WOW6432Node 里"容易得多。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    /// 有界面的软件
    App,
    /// 无界面、后台常驻
    Service,
    /// 由任务计划程序按时间或事件调起
    Task,
    /// 不是独立进程，而是注入到别的进程里（AppInit_DLLs / IFEO / BHO）
    Hook,
    /// Windows 自带组件，属禁改区
    System,
}

/// 友好名称的识别来源。属性面板会如实交代"这个名字从哪读到的"，
/// 避免用户以为我们凭空编了个名字。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NameSource {
    /// 动作目标 exe 的版本信息 FileDescription（最可靠）
    FileDescription,
    /// 版本信息 ProductName
    ProductName,
    /// 服务的 DisplayName
    ServiceDisplayName,
    /// 计划任务的 Description
    TaskDescription,
    /// 回退：文件名去扩展名
    FileName,
    /// 回退：注册表值名
    RegistryValueName,
}

/// 三层风险区 + 中间档。`Locked` = 禁改区，任何版本都不给写入口。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    Locked,
    High,
    Medium,
    Safe,
}

/// 「启动影响」三档 —— **与任务管理器同一把尺子**。
///
/// 阈值不是我们定的，是微软公开的口径（High = CPU > 1 秒或磁盘 > 3 MB；
/// Medium = CPU ≥ 300 ms 或磁盘 ≥ 300 KB；否则 Low）。
/// 照抄的理由很实际：用户能打开任务管理器逐条对照，
/// 两边档位不一致会立刻让整个软件不可信。
///
/// ⚠️ 它量的是**资源占用**，不是"让开机慢了几秒"。多线程程序的 CPU
/// 时间跨核累加，可以超过窗口本身的长度——所以这一档从不与耗时混着算。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImpactLevel {
    Low,
    Medium,
    High,
}

/// 一项的「启动影响」：Windows 自己在登录窗口里实测出来的资源消耗。
///
/// 来源是 WDI 的 `StartupInfo` XML（见 `diag::startup_info`），
/// **任务管理器的「启动影响」列读的就是它**。
///
/// 与 `ItemTiming` 的关系：`ItemTiming` 回答"它在什么时候出现 / 花了多久"，
/// `ItemImpact` 回答"它有多重"。两条轴互不替代——一个程序可以在开机后
/// 第 12 秒才出现（时间轴上很靠后），同时是全场最重的那个。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemImpact {
    /// 窗口内消耗的 CPU 时间（毫秒）。⚠️ 跨核累加，不是墙钟耗时。
    pub cpu_ms: u64,
    /// 窗口内读写的磁盘字节数。
    pub disk_bytes: u64,
    /// 按微软阈值分出的档位。
    pub level: ImpactLevel,
    /// 它在跟踪窗口里的第几秒被拉起（Windows 记的）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_in_trace_ms: Option<u64>,
    /// 对应上了几个进程实例。>1 时界面要说明这些数字是合计值。
    pub process_count: usize,
}

/// 耗时数据的可信度 —— 「诚实原则」的代码化。
///
/// 四个档次的区别**不是**"准不准"，而是"测到了什么"：
/// Windows 只对少数被判慢的项记录耗时，所以"有耗时"和"有实测"不是一回事。
/// 把这两者混成一个"实测"，就等于拿出现时刻冒充耗时。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// **实测耗时**：系统事件日志里留下了这一项的耗时硬证据（Event 101/102/103）。
    /// 只有被判慢的项才有——这是 Windows 唯一会记单项耗时的地方。
    Measured,
    /// **实测出现时刻**：内核记录了它的进程创建时刻，我们知道它在开机后第几秒出现，
    /// 但**不知道它花了多久**（那取决于应用自己什么时候算"就绪"）。
    /// 数据同样是实测的，只是量到的不是耗时。
    Observed,
    /// **估算**：按启动相位推算，界面上必须标注为估算且不显示时长。
    Estimated,
    /// 信息不足，不参与甘特图绘制
    None,
}

/// 开机阶段，用于甘特图背景相位带与"启动时机"的口语化表达。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BootPhase {
    Kernel,
    Driver,
    Devices,
    Smss,
    UserAuth,
    UserInit,
    Shell,
    Logon,
    Unknown,
}

impl BootPhase {
    /// 甘特图相位带的绘制顺序。由 T8 的事件解析与前端甘特视图消费。
    #[allow(dead_code)]
    pub const ORDER: [BootPhase; 8] = [
        BootPhase::Kernel,
        BootPhase::Driver,
        BootPhase::Devices,
        BootPhase::Smss,
        BootPhase::UserAuth,
        BootPhase::UserInit,
        BootPhase::Shell,
        BootPhase::Logon,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    User,
    Machine,
}

/// ⭐ 启动项的有效性 —— 「无效启动项检测」的结论载体。
///
/// 这项能力的价值：机器上用久了的系统里，大量 Run 项指向早已卸载的程序，
/// 它们不会让开机变慢（找不到文件就跳过了），但它们**污染判断**——
/// 用户看到 16 个启动项以为都是负担，实际有一半是尸体。
/// 把它们识别出来，用户第一次就能看清"真正的负担有几个"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValidityStatus {
    /// 目标存在且看起来可执行
    Ok,
    /// 目标文件不存在 —— 程序已被卸载，注册项是残留
    MissingTarget,
    /// 目标存在但不是有效的可执行文件（扩展名不对，或是文件夹）
    NotExecutable,
    /// 目标位于当前不可达的位置（网络共享断开 / 可移动介质已拔出）
    Unreachable,
    /// 同一程序存在多个自启入口，此项属冗余
    Duplicate,
    /// 已被停用但仍留在注册表中
    DisabledRemnant,
    /// 信息不足，无法判定（例如 UWP 项、无路径的 COM 触发项）
    Unknown,
}

impl ValidityStatus {
    /// 是否属于"应该处理的问题"。`Duplicate` 与 `DisabledRemnant` 轻微，
    /// `Unreachable` 可能是临时的（笔记本没插扩展坞），也不宜激进。
    pub fn is_broken(self) -> bool {
        matches!(
            self,
            ValidityStatus::MissingTarget | ValidityStatus::NotExecutable
        )
    }
}

/// 建议动作。首版**只呈现建议，不提供执行入口** —— 执行归 v1.5 的写操作层。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecommendationAction {
    /// 保持现状
    Keep,
    /// 建议停用（可逆，保留实体，仅写 StartupApproved 缓存键）
    Disable,
    /// 建议清理（目标已不存在，注册项纯属残留）
    Remove,
    /// 信息不足或情况复杂，建议人工确认
    Review,
}

/// 「建议」的完整载体。
///
/// ⚠️ 强制约束：`reason` 必须是**人话**，不允许只给代码或技术断言。
/// 用户看完这句话应该能自己做决定，而不是被迫相信软件。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub action: RecommendationAction,
    /// 面向用户的一句话理由，例："这个程序已经不在电脑上了，启动项是卸载残留"
    pub reason: String,
    /// 建议本身的把握程度。低把握的建议仍会展示，但措辞更保守。
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignerInfo {
    pub is_signed: bool,
    /// 证书 CN，如 "Microsoft Corporation"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// 发布者命中微软白名单
    pub is_microsoft: bool,
    /// WinVerifyTrust 校验结果，取不到证书时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_valid: Option<bool>,
    /// 位于 System32 / drivers 等系统路径
    pub is_os_component: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemTiming {
    pub confidence: Confidence,
    /// 相对开机起点的**估算**启动时刻，毫秒。
    ///
    /// ⚠️ `Confidence::Estimated` 才用它，值是"它所属的开机相位是从第几毫秒开始的"。
    /// 同一个相位里的所有项拿到的是**同一个**数字——这不是数据，是相位边界。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_estimate_ms: Option<u64>,
    /// 该时长耗时，毫秒。
    ///
    /// ⚠️ **只有 `Measured` 才会填**。系统没记这项耗时就是没有，不换算、不补值。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// 数据来源事件 ID，如 103
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_event_id: Option<u32>,
    /// **实测**：进程创建时刻相对本次开机起点的毫秒数（内核记录）。
    ///
    /// 这是"拆到单项"真正落地的地方——每一项自己的真实时间位置。
    /// 但它**不是耗时**，界面措辞必须是"开机后第 N 秒出现"。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_start_ms: Option<u64>,
    /// **实测**：快照那一刻，该进程**从启动至今**累计从磁盘读取的字节数。
    ///
    /// 必须与 `observed_at_ms` 一起看：它不是"开机阶段读了这么多"。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_bytes: Option<u64>,
    /// 快照拍摄于开机后多久（毫秒）。用来界定上面那个累计值有多"新"。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
    /// **实测**：Windows 自己在登录窗口里量出来的资源消耗（WDI `StartupInfo`）。
    ///
    /// 与上面几项是**并存的**，不是替代关系：一个项完全可以既有出现时刻、
    /// 又有实测耗时、还有实测影响。`None` = 这次没拿到它的记录
    /// （没在登录窗口里跑、不在这份数据覆盖的启动机制内、或没提权读不到）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impact: Option<ItemImpact>,
}

impl Default for ItemTiming {
    fn default() -> Self {
        Self {
            confidence: Confidence::None,
            start_estimate_ms: None,
            duration_ms: None,
            source_event_id: None,
            observed_start_ms: None,
            read_bytes: None,
            observed_at_ms: None,
            impact: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticInfo {
    /// 诊断码，如 SLOW_START / MANUAL_BUT_SHOULD_AUTO / UNSIGNED / GLOBAL_HOOK
    pub code: String,
    pub severity: RiskLevel,
    /// 面向用户的中文结论
    pub message: String,
    /// 证据，如 "EventID=103, 12.4s"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// 【预留】编排目标态。v1.0 恒为默认值，由 v1.5 的写操作与 v2.0 的编排填充。
///
/// 现在就把位置留好，是因为**数据结构一旦发布就难改**——
/// 而 v2.0 的编排是这个产品的护城河，不能让它受制于 v1.0 的短视。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesiredState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delay_sec: Option<u64>,
    /// 对应 IFEO `CpuPriorityClass`：1=Idle 2=Normal 3=High
    /// ⚠️ 4=RealTime 永久禁用，会抢在音频驱动前拿 CPU
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_class: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_priority: Option<u8>,
    /// 在启动序列中的位次（同一启动阶段内从 0 开始）。
    ///
    /// 与 `delay_sec` 是两件事：延迟改的是"等多少秒"，顺序改的是"排在谁前面"。
    /// 前者适合"这个不急"，后者适合"这个必须在那个之前"。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
    /// 就绪探针配置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<String>,
    /// DAG 依赖的上游项 ID 列表
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupItem {
    /// 稳定 UUID，由 `source + identity_key` 派生（uuid v5），
    /// 保证同一台机器上反复扫描得到的 id 不变——这是快照比对的前提。
    pub id: String,
    pub source: SourceKind,
    /// 归一化后的去重键：小写 exe 路径（去引号）+ 归一化参数
    pub identity_key: String,
    /// 原始名称：注册表值名 / 任务名 / 文件名 / 服务名。
    /// **不要直接展示给用户** —— 它可能是 `localsend_app` 这类机器名。
    pub name: String,
    /// 面向用户的类型。判定链见 `derive_kind()`。
    pub kind: ItemKind,
    /// 统一识别出的友好名称。后端优先从 exe 版本信息读取。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// 友好名称的识别来源，属性面板会如实展示
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_from: Option<NameSource>,
    /// 一句话人话说明。缺省时前端按 `kind + 发布者 + 启动时机` 派生。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// 原始命令行 / 快捷方式目标
    pub command: String,
    /// 展开环境变量后的可执行文件路径
    pub resolved_path: String,
    pub args: Vec<String>,
    /// 注册表键路径 / 文件夹路径 / 任务全名
    pub location: String,
    pub scope: Scope,
    /// 当前是否生效
    pub enabled: bool,
    pub signer: SignerInfo,
    /// base64 PNG，由后端批量提取
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_data: Option<String>,
    pub risk: RiskLevel,
    pub risk_reasons: Vec<String>,
    pub diagnostics: Vec<DiagnosticInfo>,
    pub boot_phase: BootPhase,
    pub timing: ItemTiming,
    /// ⭐ 有效性判定（无效启动项检测）
    pub validity: ValidityStatus,
    /// 有效性判定的补充说明，例："目标路径不存在：D:\OldApp\a.exe"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validity_detail: Option<String>,
    /// ⭐ 处置建议。v1.0 仅呈现，不提供执行入口。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<Recommendation>,
    /// 若判定为重复项，指向"代表项"的 id
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
    /// 原始数据，留作后续回滚审计
    pub raw: serde_json::Value,
    /// 【预留】编排目标态，v1.0 恒为空
    pub desired: DesiredState,
    /// 【预留】关联快照 ID，v1.5 回滚使用
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_ref: Option<String>,
}

/// 类型判定链（顺序即优先级）：
/// `系统注入 > Windows 组件 > 服务 > 计划任务 > 应用程序`
///
/// 为什么 `is_os_component` 排在 Service 前面？因为"Windows Audio""RPC"
/// 这类东西形态上是服务，但对用户而言属于"系统自带、别碰"这一档，
/// 归到「系统组件」比混在第三方服务里更符合直觉。
pub fn derive_kind(source: SourceKind, signer: &SignerInfo) -> ItemKind {
    if source == SourceKind::SystemHook {
        return ItemKind::Hook;
    }
    if signer.is_os_component {
        return ItemKind::System;
    }
    match source {
        SourceKind::Service => ItemKind::Service,
        SourceKind::ScheduledTask => ItemKind::Task,
        _ => ItemKind::App,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseSpan {
    pub name: BootPhase,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlowService {
    /// 来自事件字段 `Name`。
    ///
    /// ⚠️ 对 Event 103 而言这是**服务名**（`windefend` / `eventlog`），
    /// 不是显示名——匹配启动项时必须按服务名比。
    pub name: String,
    /// 事件字段 `FriendlyName`，常为空
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
    /// 事件字段 `TotalTime`，单位毫秒
    pub duration_ms: u64,
    /// 事件字段 `DegradationTime`：其中"多花的"时间
    ///
    /// 它比 `TotalTime` 更接近"拖慢了多少"，因为总时间里有一部分是
    /// 正常启动本来就需要的。
    pub degradation_ms: u64,
    /// 数据来源事件 ID：101=应用 102=驱动 103=服务
    pub event_id: u32,
}

/// 开机耗时时间轴。
///
/// ⚠️ **`phases` 为空不等于"开机耗时为 0"**。读取需要管理员权限
/// （该 channel 的 ACL 里没有普通用户的 ACE），所以必须用
/// `unavailable_reason` / `needs_elevation` 把"读不到"讲清楚，
/// 否则界面只能显示一个空图，用户会以为自己的开机不需要时间。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootTimeline {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_boot_ms: Option<u64>,
    /// 各相位耗时，按时间先后排列
    pub phases: Vec<PhaseSpan>,
    /// 被系统记录为"慢"的服务/应用/驱动
    pub slow_services: Vec<SlowService>,
    /// 本次读数对应哪一次开机（事件里的 `BootStartTime`），用于判断新鲜度
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_started_at: Option<String>,
    /// 为什么拿不到数据。`None` 表示读取正常。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    /// 是否因为权限不足。界面上据此决定要不要给「以管理员身份重试」
    pub needs_elevation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsInfo {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    /// 如 "Windows 11 Pro 24H2"
    pub sku: String,
}

impl OsInfo {
    /// 事件日志字段在 Win10/11 上有差异，解析时用来分版本容错。由 T8 消费。
    #[allow(dead_code)]
    pub fn is_windows_11(&self) -> bool {
        self.build >= 22000
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub items: Vec<StartupItem>,
    pub os: OsInfo,
    /// 是否以管理员身份运行
    pub elevated: bool,
    /// ISO8601
    pub scanned_at: String,
    pub boot_timeline: BootTimeline,
    /// 本次扫描时做的**进程采样**概况。见 `ItemObservation`。
    pub observation: ItemObservation,
    /// 本次扫描读到的**启动影响**数据概况。见 `ImpactOverview`。
    pub impact: ImpactOverview,
    /// 各来源可容忍的部分失败，不阻断整体扫描
    pub errors: Vec<String>,
}

/// 一次 WDI `StartupInfo` 读取的概况。
///
/// 与 `ItemObservation` 同样的理由单列出来：**"一项都没对上"和"没读到"
/// 必须能分清**。前者是数据在、但我们的启动项列表里没有匹配的映像；
/// 后者是权限/版本/策略导致这份数据根本不存在——
/// 两者的处置完全不同，让界面去猜就成了编造。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpactOverview {
    /// 跟踪窗口长度（毫秒）。所有 CPU / 磁盘数字都只覆盖这么长一段时间。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_ms: Option<u64>,
    /// 文件里一共多少条进程记录。
    pub record_count: usize,
    /// 其中成功对应到启动项的个数。
    pub matched_count: usize,
    /// 数据取自哪个账户的登录会话（SID）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_sid: Option<String>,
    /// 上面那个 SID 是不是当前登录用户。
    ///
    /// `false` 时界面**必须**说明"这是另一个账户那次登录的记录"——
    /// 把别人的登录数据当成自己的，比不给数据更糟。
    pub is_current_user: bool,
    /// 数据来自哪个文件。用户能自己去打开核对，这是这份数据最经得起验证的地方。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    /// 不可用时的原因（人话）。可用时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

impl ImpactOverview {
    /// 从读取结果生成概况。`matched_count` 由调用方在归因后回填。
    pub fn from_report(rep: &crate::diag::startup_info::StartupInfoReport) -> Self {
        // ⚠️ `StartupInfoReport::default()` 是"可用但空"（没有原因、没有记录）。
        // 那种状态直接交给界面，界面就只能自己猜是"读到了空文件"还是"没读到"。
        // 在这里归一化成一个明确的原因，让界面永远不需要猜。
        let reason = rep.unavailable_reason.clone().or_else(|| {
            rep.records
                .is_empty()
                .then(|| "这次没有读到任何开机启动影响记录。".to_string())
        });

        Self {
            window_ms: rep.window_ms,
            record_count: rep.records.len(),
            matched_count: 0,
            source_sid: rep.source_sid.clone(),
            is_current_user: rep.is_current_user,
            source_file: rep.source_file.clone(),
            unavailable_reason: reason,
        }
    }
}

/// 一次开机进程采样的概况。
///
/// 为什么单列成一个结构、而不是让前端从各项的 `timing` 里自己汇总：
/// **一项都没匹配上时也必须能说清"为什么"**。空数组既可能是"采样失败"，
/// 也可能是"采到了但没一项对得上"，两者的处置完全不同——
/// 让界面去猜就成了编造，而这里恰好有一个确定的答案。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemObservation {
    /// 采样拍摄于开机后多久（毫秒）。`None` = 连开机起点都没拿到，采样不可用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_at_offset_ms: Option<u64>,
    /// 采样窗口内一共多少个进程。
    pub process_count: usize,
    /// 其中成功对应到启动项的个数。
    pub observed_count: usize,
    /// 不可用时的原因（人话）。可用时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}
