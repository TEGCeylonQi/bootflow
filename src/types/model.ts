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
/**
 * 耗时数据的可信度 —— 「诚实原则」的代码化。
 *
 * 四档的区别**不是**"准不准"，而是"测到了什么"。Windows 只对少数被判慢的项
 * 记录耗时，所以"有耗时"和"有实测"不是一回事；把两者混成一个档次，
 * 就等于拿出现时刻冒充耗时。
 */
export type Confidence =
  /** 实测耗时：系统事件日志留下的这一项耗时硬证据（Event 101/102/103） */
  | 'measured'
  /** 实测出现时刻：内核记录了进程创建时刻，但**没有**耗时 */
  | 'observed'
  /** 估算：按启动相位推算，界面必须标注且不给时长 */
  | 'estimated'
  | 'none'

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
  /**
   * 相对开机起点的**估算**启动时刻，毫秒。
   *
   * ⚠️ 只有 `estimated` 才用它，值是"它所属的开机相位从第几毫秒开始"。
   * 同一相位里的所有项拿到的是**同一个**数字——这不是数据，是相位边界。
   */
  startEstimateMs?: number
  /**
   * 该时长耗时，毫秒。
   *
   * ⚠️ **只有 `measured` 才会填**。系统没记这项耗时就是没有，不换算、不补值。
   */
  durationMs?: number
  /** 数据来源事件 ID，如 103 */
  sourceEventId?: number
  /**
   * **实测**：进程创建时刻相对本次开机起点的毫秒数（内核记录）。
   *
   * 这是"拆到单项"真正落地的地方。但它**不是耗时**——
   * 界面措辞必须是「开机后第 N 秒出现」，不是「启动花了 N 秒」。
   */
  observedStartMs?: number
  /**
   * **实测**：快照那一刻，该进程**从启动至今**累计从磁盘读取的字节数。
   *
   * 必须与 `observedAtMs` 一起看：它不是"开机阶段读了这么多"。
   */
  readBytes?: number
  /** 快照拍摄于开机后多久（毫秒）。用来界定上面那个累计值有多"新"。 */
  observedAtMs?: number
  /**
   * **实测**：Windows 自己在登录窗口里量出来的资源消耗（WDI `StartupInfo`）。
   *
   * 与上面几项**并存**，不是替代关系。`undefined` = 这次没拿到它的记录。
   */
  impact?: ItemImpact
}

/**
 * 单项「启动影响」—— **任务管理器「启动影响」列读的就是这份数据**。
 *
 * ⚠️ 它量的是**资源占用**，不是"让开机慢了几秒"。多线程程序的 CPU 时间
 * 跨核累加，可以超过窗口本身的长度。所以这一档**从不与耗时混着算**，
 * 界面上也必须单独一栏、单独措辞。
 */
export interface ItemImpact {
  /** 窗口内消耗的 CPU 时间（毫秒）。⚠️ 跨核累加，不是墙钟耗时 */
  cpuMs: number
  /** 窗口内读写的磁盘字节数 */
  diskBytes: number
  /** 按微软阈值分出的档位 */
  level: ImpactLevel
  /** 它在跟踪窗口里的第几秒被拉起（Windows 记的） */
  startedInTraceMs?: number
  /** 对应上了几个进程实例。>1 时界面要说明这些数字是合计值 */
  processCount: number
}

/**
 * 清单排序方式。
 *
 * 刻意只有两种、而不是一个"综合分"：它们回答的是**两个不同的问题**——
 * `default` 是"谁先跑起来"（时序），`impact` 是"谁最费资源"（开销）。
 * 揉成一个分数就再也说不清某一项为什么排在前面。
 */
export type SortBy = 'default' | 'impact'

/**
 * 「启动影响」三档 —— 与任务管理器同一把尺子。
 *
 * 阈值不是我们定的，是微软公开口径（High = CPU > 1 秒或磁盘 > 3 MB；
 * Medium = CPU ≥ 300 ms 或磁盘 ≥ 300 KB）。用户能打开任务管理器逐条对照，
 * 两边档位不一致会立刻让整个软件不可信。
 */
export type ImpactLevel = 'low' | 'medium' | 'high'

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

/**
 * 一条「开机自记账」记录——**BootFlow 自己写的**，不依赖系统事件日志。
 *
 * 存在的理由：Windows 的 Event 100 只在「完整引导 + 系统认为开机偏慢」时才写，
 * 开了快速启动的机器可以几个月一条都没有（实测某台机器 403 天零记录）。
 * 自记账在每次登录时由 BootFlow 的自启条目静默跑一次，绕过这个限制。
 *
 * ⚠️ **能承诺与不能承诺的**：
 * - 能：从内核启动到登录自启那一刻的**总时长**（`GetTickCount64` 官方口径，实测）；
 * - 不能：拆到各相位 / 各启动项。那是用户态拿不到的，只有 Event 100/103 有。
 * 所以界面上它只画一根总长度条，绝不假造分段。
 */
export interface BootRecord {
  /** 近似开机起点（ISO8601，本地时区）。由「记录时刻 − 已开机时长」推得。 */
  bootStartedAt: string
  /** 记录落盘那一刻的墙钟时间（ISO8601） */
  recordedAt: string
  /**
   * 从内核启动到记录这一刻的毫秒数（`GetTickCount64` 官方口径）。
   *
   * **同一次开机只保留最早的一次观测**：开机自启跑的那次≈开机耗时；
   * 之后手动重跑会得到"运行时长"，那不是开机耗时，会被丢弃。
   */
  totalMs: number
  /** 数据来源，恒为 `marker`（自记账）。Event 100 走 timeline，不写这里。 */
  source: string
  /**
   * 开机起点是怎么来的——界面据此区分**实测**与**估算**。
   *
   * - `log`：系统日志里**本次开机**的那条事件给出起点（实测）。
   * - `tick`：系统日志读不到，退回 `记录时刻 − GetTickCount64` 推算（估算）。
   *
   * 为什么必须带上：`GetTickCount64` 在**快速启动下不会重置**，
   * 这条退路算出来可能是跨了好几次开关机的累计运行时长。
   * 不标出来就等于拿推算冒充实测。
   */
  basis: string
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
  /** 本次扫描时所做的**进程采样**概况 */
  observation: ItemObservation
  /** 本次扫描读到的**启动影响**数据概况 */
  impact: ImpactOverview
  /** 各来源可容忍的部分失败，不阻断整体扫描 */
  errors: string[]
}

/**
 * 一次 WDI `StartupInfo` 读取的概况。
 *
 * 与 `ItemObservation` 同样的理由单列出来：**"一项都没对上"和"没读到"
 * 必须能分清**。前者是数据在、但启动项列表里没有匹配的映像；
 * 后者是权限/版本/策略导致这份数据根本不存在——两者的处置完全不同。
 */
export interface ImpactOverview {
  /** 跟踪窗口长度（毫秒）。所有 CPU / 磁盘数字都只覆盖这么长一段时间 */
  windowMs?: number
  /** 文件里一共多少条进程记录 */
  recordCount: number
  /** 其中成功对应到启动项的个数 */
  matchedCount: number
  /** 数据取自哪个账户的登录会话（SID） */
  sourceSid?: string
  /**
   * 上面那个 SID 是不是当前登录用户。
   *
   * `false` 时界面**必须**说明"这是另一个账户那次登录的记录"——
   * 把别人的登录数据当成自己的，比不给数据更糟。
   */
  isCurrentUser: boolean
  /** 数据来自哪个文件。用户能自己去打开核对 */
  sourceFile?: string
  /** 不可用时的原因（人话） */
  unavailableReason?: string
}

/**
 * 一次开机进程采样的概况。
 *
 * 单列一个结构、而不是让界面从各项 `timing` 里自己汇总，是因为
 * **一项都没匹配上时也必须能说清"为什么"**：空数组既可能是"采样失败"，
 * 也可能是"采到了但没一项对得上"，两者处置完全不同。
 */
export interface ItemObservation {
  /** 采样拍摄于开机后多久（毫秒）。缺省 = 采样不可用 */
  capturedAtOffsetMs?: number
  /** 采样窗口内一共多少个进程 */
  processCount: number
  /** 其中成功对应到启动项的个数 */
  observedCount: number
  /** 不可用时的原因（人话） */
  unavailableReason?: string
}
