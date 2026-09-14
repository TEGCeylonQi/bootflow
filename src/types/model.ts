/**
 * 与 Rust 侧 `src-tauri/src/model.rs` 一一对应的类型定义。
 * 约定：字段名一律 camelCase，Rust 侧靠 `#[serde(rename_all = "camelCase")]` 对齐。
 * 改动此文件时，务必同步修改 Rust 侧结构体。
 */

/** 启动项来源。前 8 项对应首版支持的核心四类（启动文件夹 / Run / RunOnce / 计划任务 / 服务） */
export type SourceKind =
  | 'StartupFolderUser'
  | 'StartupFolderMachine'
  | 'RunUser'
  | 'RunMachine'
  | 'RunMachine32'
  | 'RunOnceUser'
  | 'RunOnceMachine'
  | 'RunOnceMachine32'
  | 'ScheduledTask'
  | 'Service'
  | 'SystemHook'

/**
 * 「这是什么」——面向用户的类型维度，与 SourceKind（从哪注册的）**正交**。
 *
 * 一个程序可以「注册在 Run 键里」（source = RunUser）同时是「应用程序」（kind = app）。
 * 界面上优先展示 kind，因为用户理解「这是个服务」比理解「这在 HKLM\WOW6432Node 里」容易得多。
 */
export type ItemKind =
  /** 有界面的软件 */
  | 'app'
  /** 无界面、后台常驻，多由系统在开机早期拉起 */
  | 'service'
  /** 由任务计划程序按时间或事件调起 */
  | 'task'
  /** 不以独立进程运行，而是注入到其它进程中（AppInit_DLLs / IFEO / BHO） */
  | 'hook'
  /** Windows 自带组件，属禁改区 */
  | 'system'

/**
 * 友好名称的识别来源。属性面板会如实交代「这个名字是从哪读到的」，
 * 避免用户以为我们凭空编了个名字。
 */
export type NameSource =
  /** 动作目标 exe 的版本信息 FileDescription（最可靠） */
  | 'fileDescription'
  /** 版本信息 ProductName */
  | 'productName'
  /** 服务的 DisplayName */
  | 'serviceDisplayName'
  /** 计划任务的 Description */
  | 'taskDescription'
  /** 回退：文件名去扩展名 */
  | 'fileName'
  /** 回退：注册表值名 */
  | 'registryValueName'

/** 三层风险区 + 中间档。Locked = 禁改区，任何版本都不给写入口 */
export type RiskLevel = 'Locked' | 'High' | 'Medium' | 'Safe'

/**
 * 耗时数据的可信度——这是「诚实原则」的代码化。
 * measured = 来自系统事件日志的硬证据
 * estimated = 按启动相位推算，必须在界面上标注为估算
 * none = 信息不足，不参与甘特图绘制
 */
export type Confidence = 'measured' | 'estimated' | 'none'

/** 开机阶段，用于甘特图背景相位带 */
export type BootPhase =
  | 'kernel'
  | 'driver'
  | 'devices'
  | 'smss'
  | 'userAuth'
  | 'userInit'
  | 'shell'
  | 'logon'
  | 'unknown'

/**
 * ⭐ 启动项的有效性 —— 「无效启动项检测」的结论载体。
 *
 * 机器用久了，注册表里会积累大量"尸体"：程序早卸载了，Run 键还留着。
 * 它们不会拖慢开机（找不到文件直接跳过），但会污染判断——
 * 用户看到 16 个启动项以为全是负担，实际一半是空的。
 */
export type ValidityStatus =
  /** 目标存在且看起来可执行 */
  | 'ok'
  /** 目标文件不存在 —— 程序已被卸载，注册项是残留 */
  | 'missingTarget'
  /** 目标存在但不是有效的可执行文件 */
  | 'notExecutable'
  /** 目标位于当前不可达的位置（网络共享断开 / 可移动介质已拔出） */
  | 'unreachable'
  /** 同一程序存在多个自启入口，此项属冗余 */
  | 'duplicate'
  /** 已被停用但仍留在注册表中 */
  | 'disabledRemnant'
  /** 信息不足，无法判定 */
  | 'unknown'

/**
 * 建议动作。首版**只呈现建议，不提供执行入口** —— 执行归 v1.5 的写操作层。
 */
export type RecommendationAction =
  /** 保持现状（通常不展示，避免噪音） */
  | 'keep'
  /** 建议停用：可逆，保留实体，仅写 StartupApproved 缓存键 */
  | 'disable'
  /** 建议清理：目标已不存在，注册项纯属残留 */
  | 'remove'
  /** 情况复杂，建议人工确认 */
  | 'review'

/**
 * 处置建议。
 *
 * ⚠️ 强制约束：`reason` 必须是**人话**。用户读完应该能自己做决定，
 * 而不是被迫相信软件。出现 `MANUAL_BUT_SHOULD_AUTO` 这类码就是设计失败。
 */
export interface Recommendation {
  action: RecommendationAction
  /** 面向用户的一句话理由 */
  reason: string
  /** 建议本身的把握程度。低把握的建议措辞更保守。 */
  confidence: Confidence
}

export interface SignerInfo {
  isSigned: boolean
  /** 证书 CN，如 "Microsoft Corporation" */
  publisher?: string
  /** 发布者命中微软白名单 */
  isMicrosoft: boolean
  /** WinVerifyTrust 校验结果，取不到证书时为 undefined */
  certValid?: boolean
  /** 位于 System32 / drivers 等系统路径 */
  isOsComponent: boolean
}

export interface ItemTiming {
  confidence: Confidence
  /** 相对开机起点的估算启动时刻，毫秒 */
  startEstimateMs?: number
  /** 该时长耗时，毫秒 */
  durationMs?: number
  /** 数据来源事件 ID，如 103 */
  sourceEventId?: number
}

export interface DiagnosticInfo {
  /** 诊断码，如 SLOW_START / MANUAL_BUT_SHOULD_AUTO / UNSIGNED / GLOBAL_HOOK */
  code: string
  severity: RiskLevel
  /** 面向用户的中文结论 */
  message: string
  /** 证据，如 "EventID=103, 12.4s" */
  evidence?: string
}

/**
 * 【预留】编排目标态。v1.0 恒为默认值，由 v1.5 的写操作与 v2.0 的编排填充。
 * 现在就把位置留好，避免后续改数据结构造成迁移成本。
 */
export interface DesiredState {
  enabled?: boolean
  delaySec?: number
  /** 对应 IFEO CpuPriorityClass：1=Idle 2=Normal 3=High（4=RealTime 永久禁用） */
  priorityClass?: number
  ioPriority?: number
  /**
   * 在启动序列中的位次（同一个启动阶段内从 0 开始）。
   *
   * 与 `delaySec` 是两件事：延迟改的是"等多少秒"，顺序改的是"排在谁前面"。
   * 前者适合"这个不急"，后者适合"这个必须在那个之前"。
   */
  order?: number
  /** 就绪探针配置 */
  probe?: string
  /** DAG 依赖的上游项 ID 列表 */
  dependsOn?: string[]
}

export interface StartupItem {
  /** 稳定 UUID，由 source + identityKey 派生 */
  id: string
  source: SourceKind
  /** 归一化后的去重键：小写 exe 路径（去引号）+ 归一化参数 */
  identityKey: string
  /**
   * 原始名称：注册表值名 / 任务名 / 文件名 / 服务名。
   * **不要直接展示给用户**——它可能是 `localsend_app` 这类程序内部名，
   * 也可能是注册表里的 `AcmeHelper` 这类机器名。
   * 界面上一律走 `displayNameOf()`（见 src/lib/item.ts）。
   */
  name: string
  /**
   * 面向用户的类型。后端未提供时由前端 `resolveKind()` 兜底派生。
   * 【Rust 侧需同步】判定链：SystemHook > isOsComponent > Service > ScheduledTask > app
   */
  kind: ItemKind
  /**
   * 统一识别出的友好名称。优先取自动作目标 exe 的版本信息资源，
   * 回退顺序：FileDescription → ProductName → 服务 DisplayName → 任务描述 → 文件名。
   * 【Rust 侧需同步】GetFileVersionInfoW + VerQueryValueW 读
   * `\StringFileInfo\<lang><codepage>\FileDescription`
   */
  displayName?: string
  /** 友好名称的识别来源，属性面板会如实展示 */
  nameFrom?: NameSource
  /** 一句话人话说明。缺省时由前端 `summarize()` 按 kind + 发布者 + 启动时机派生 */
  summary?: string
  /** 原始命令行 / 快捷方式目标 */
  command: string
  /** 展开环境变量后的可执行文件路径 */
  resolvedPath: string
  args: string[]
  /** 注册表键路径 / 文件夹路径 / 任务全名 */
  location: string
  scope: 'user' | 'machine'
  /** 当前是否生效 */
  enabled: boolean
  signer: SignerInfo
  /** base64 PNG，由后端批量提取 */
  iconData?: string
  risk: RiskLevel
  riskReasons: string[]
  diagnostics: DiagnosticInfo[]
  bootPhase: BootPhase
  timing: ItemTiming
  /** ⭐ 有效性判定（无效启动项检测的结论） */
  validity: ValidityStatus
  /** 有效性判定的补充说明，如「目标文件不存在，程序可能已被卸载：D:\OldApp\a.exe」 */
  validityDetail?: string
  /** ⭐ 处置建议。v1.0 仅呈现，不提供执行入口。无建议时为空。 */
  recommendation?: Recommendation
  /** 若判定为重复项，指向「代表项」的 id */
  duplicateOf?: string
  /** 原始数据，留作后续回滚审计 */
  raw: unknown
  /** 【预留】编排目标态，v1.0 恒为空 */
  desired: DesiredState
  /** 【预留】关联快照 ID，v1.5 回滚使用 */
  snapshotRef?: string
}

export interface PhaseSpan {
  name: BootPhase
  startMs: number
  endMs: number
}

export interface SlowService {
  /**
   * 来自事件字段 `Name`。
   *
   * ⚠️ 对 Event 103 而言这是**服务名**（`windefend` / `eventlog`），不是显示名。
   * 匹配启动项时必须按服务名比，界面展示则优先用 `friendlyName`。
   */
  name: string
  /** 事件字段 `FriendlyName`，系统常常不给，所以可选 */
  friendlyName?: string
  /** 事件字段 `TotalTime`：这项加起来的全部耗时 */
  durationMs: number
  /**
   * 事件字段 `DegradationTime`：其中"多花的"时间。
   *
   * 它比 `durationMs` 更接近用户关心的"拖慢了多少"——总时间里有一部分
   * 是它正常启动本来就需要花的。**界面上以此为主指标**。
   */
  degradationMs: number
  /** 数据来源事件 ID：101=应用 102=驱动 103=服务 */
  eventId: number
}

/**
 * 开机耗时时间轴。
 *
 * ⚠️ **`phases` 为空不等于"开机耗时为 0"**。读取需要管理员权限
 * （该 channel 的 ACL 里没有普通用户的 ACE），所以 `unavailableReason` /
 * `needsElevation` 必须一路传到界面上——否则只能显示一张空图，
 * 用户会以为自己的开机不需要时间，而真实原因是"我们没被允许看"。
 *
 * 这是「诚实原则」在数据结构上的落点：**读不到就是读不到，不许用图表掩盖**。
 */
export interface BootTimeline {
  totalBootMs?: number
  /** 各相位耗时，按时间先后排列 */
  phases: PhaseSpan[]
  /** 被系统记录为"慢"的服务/应用/驱动 */
  slowServices: SlowService[]
  /** 本次读数对应哪一次开机（事件里的 `BootStartTime`），用于判断新鲜度 */
  bootStartedAt?: string
  /** 为什么拿不到数据。`undefined` 表示读取正常。 */
  unavailableReason?: string
  /** 是否因为权限不足。界面据此决定要不要给「以管理员身份重试」的入口 */
  needsElevation: boolean
}

export interface OsInfo {
  major: number
  minor: number
  build: number
  sku: string
}

export interface ScanResult {
  items: StartupItem[]
  os: OsInfo
  /** 是否以管理员身份运行 */
  elevated: boolean
  /** ISO8601 */
  scannedAt: string
  bootTimeline: BootTimeline
  /** 各来源可容忍的部分失败，不阻断整体扫描 */
  errors: string[]
}
