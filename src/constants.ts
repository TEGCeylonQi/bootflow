import type {
  BootPhase,
  ImpactLevel,
  ItemKind,
  RecommendationAction,
  RiskLevel,
  SourceKind,
  ValidityStatus,
} from './types/model'

/**
 * 【技术分类 · 界面已不再用它分组】
 *
 * 下面三个常量按「注册位置」归类，但用户看不懂「注册表 Run」意味着什么，
 * 所以左侧清单已改用 ItemKind（见文件下方的 KIND_META / KIND_ORDER）分组。
 * 保留它们是因为报告导出、以及后续可能出现的「高级视图」仍需按注册位置归类。
 */
export type SourceGroup = '启动文件夹' | '注册表 Run' | '计划任务' | '服务' | '系统钩子'

export const SOURCE_LABEL: Record<SourceKind, string> = {
  StartupFolderUser: '启动文件夹',
  StartupFolderMachine: '启动文件夹（全局）',
  RunUser: 'Run · HKCU',
  RunMachine: 'Run · HKLM',
  RunMachine32: 'Run · HKLM 32位',
  RunOnceUser: 'RunOnce · HKCU',
  RunOnceMachine: 'RunOnce · HKLM',
  RunOnceMachine32: 'RunOnce · HKLM 32位',
  ScheduledTask: '计划任务',
  Service: '服务',
  SystemHook: '系统钩子',
}

export const SOURCE_GROUP: Record<SourceKind, SourceGroup> = {
  StartupFolderUser: '启动文件夹',
  StartupFolderMachine: '启动文件夹',
  RunUser: '注册表 Run',
  RunMachine: '注册表 Run',
  RunMachine32: '注册表 Run',
  RunOnceUser: '注册表 Run',
  RunOnceMachine: '注册表 Run',
  RunOnceMachine32: '注册表 Run',
  ScheduledTask: '计划任务',
  Service: '服务',
  SystemHook: '系统钩子',
}

/** 分组展示顺序 */
export const GROUP_ORDER: SourceGroup[] = ['服务', '计划任务', '启动文件夹', '注册表 Run', '系统钩子']

export interface RiskMeta {
  label: string
  color: string
  /** 一句话解释这个等级意味着什么 */
  desc: string
}

export const RISK_META: Record<RiskLevel, RiskMeta> = {
  Locked: {
    label: '禁改区',
    color: '#f85149',
    desc: '系统关键组件，任何版本都不提供修改入口',
  },
  High: {
    label: '高危',
    color: '#d29922',
    desc: '未签名、全局注入、被异常修改，或命中慢启动记录',
  },
  Medium: {
    label: '注意',
    color: '#58a6ff',
    desc: '信息不完整或存在轻微可疑特征，建议关注',
  },
  Safe: {
    label: '安全',
    color: '#3fb950',
    desc: '用户级位置且签名有效，风险可控',
  },
}

export const RISK_ORDER: RiskLevel[] = ['Locked', 'High', 'Medium', 'Safe']

export const PHASE_LABEL: Record<BootPhase, string> = {
  kernel: '内核初始化',
  driver: '驱动加载',
  devices: '设备初始化',
  smss: '会话管理',
  userAuth: '用户认证',
  userInit: '用户会话',
  shell: '外壳启动',
  logon: '登录后',
  unknown: '未归类',
}

/** 甘特图相位背景带的顺序 */
export const PHASE_ORDER: BootPhase[] = [
  'kernel',
  'driver',
  'devices',
  'smss',
  'userAuth',
  'userInit',
  'shell',
  'logon',
]

export const CONFIDENCE_LABEL: Record<string, string> = {
  measured: '实测耗时',
  /**
   * 「实测出现时刻」——它和 `measured` 都是硬数据，量到的却不是同一个东西：
   * 这一档只知道"什么时候出现"，**不知道花了多久**。所以标签里必须带"时刻"，
   * 只写"实测"会让人把它读成耗时。
   */
  observed: '实测出现时刻',
  estimated: '估算',
  none: '未知',
}

/**
 * 「启动影响」三档的用户措辞 —— **与任务管理器用同一套词**。
 *
 * 刻意不写成"严重/中等/轻微"之类：任务管理器里写的是"高/中/低"，
 * 换一套词就等于逼用户在两处之间做翻译，而"能逐条对照"正是这份数据的价值。
 */
export const IMPACT_LABEL: Record<ImpactLevel, string> = {
  low: '低',
  medium: '中',
  high: '高',
}

/** 三档的状态色。复用既有的 ok / warn / danger 三色，不新造颜色。 */
export const IMPACT_COLOR: Record<ImpactLevel, string> = {
  low: '#3fb950',
  medium: '#d29922',
  high: '#f85149',
}

/** 甘特图与图例共用的相位配色：蓝 → 橙 表达时间推进 */
export const PHASE_COLOR: Record<BootPhase, string> = {
  kernel: '#1f6feb',
  driver: '#388bfd',
  devices: '#58a6ff',
  smss: '#79c0ff',
  userAuth: '#a5d6ff',
  userInit: '#d29922',
  shell: '#f0883e',
  logon: '#db6d28',
  unknown: '#484f58',
}

/* ─────────────────────────────────────────────────────────────
   以下为「面向用户」的展示层常量。
   与上面 SOURCE_* 的区别：SOURCE_* 是技术分类（注册在哪），
   下面的 KIND_* 才是用户能看懂的「这是什么」。
   ───────────────────────────────────────────────────────────── */

export interface KindMeta {
  /** 完整名称，用于分组标题与徽章悬浮说明 */
  label: string
  /** 两字简称，用于小尺寸徽章 */
  short: string
  color: string
  /** 一句话解释这类东西是什么、为什么在开机时运行 */
  desc: string
}

export const KIND_META: Record<ItemKind, KindMeta> = {
  app: {
    label: '应用程序',
    short: '应用',
    color: '#58a6ff',
    desc: '有界面的软件。通常在你登录后自行启动，是否保留由你决定。',
  },
  service: {
    label: '后台服务',
    short: '服务',
    color: '#bc8cff',
    desc: '没有界面，在后台持续运行。多数由系统在开机阶段拉起，用于支撑某个功能。',
  },
  task: {
    label: '计划任务',
    short: '任务',
    color: '#d29922',
    desc: '由 Windows 任务计划程序按时间或事件触发。注意：多数计划任务并不在开机时运行。',
  },
  hook: {
    label: '系统注入',
    short: '注入',
    color: '#f85149',
    desc: '不是一个独立程序，而是被塞进其它程序里一起运行。会影响所有相关程序，风险最高。',
  },
  system: {
    label: '系统组件',
    short: '系统',
    color: '#6e7681',
    desc: 'Windows 自带组件。本工具不提供修改入口——动了它们通常会弄坏系统。',
  },
}

/** 分组与筛选的展示顺序：用户关心的在前，系统自带的沉底 */
export const KIND_ORDER: ItemKind[] = ['app', 'service', 'task', 'hook', 'system']

/** 启动方式的「人话版」——技术路径只在详情里出现 */
export const SOURCE_PLAIN: Record<SourceKind, string> = {
  StartupFolderUser: '启动文件夹（当前用户）',
  StartupFolderMachine: '启动文件夹（所有用户）',
  RunUser: '注册表启动项（当前用户）',
  RunMachine: '注册表启动项（所有用户）',
  RunMachine32: '注册表启动项（所有用户 · 32 位）',
  RunOnceUser: '一次性启动项（当前用户）',
  RunOnceMachine: '一次性启动项（所有用户）',
  RunOnceMachine32: '一次性启动项（所有用户 · 32 位）',
  ScheduledTask: '计划任务',
  Service: '系统服务',
  SystemHook: '系统注入配置',
}

/** 把开机阶段翻译成人能记住的时间点，替代「userInit」「smss」这类术语 */
export const PHASE_SLOGAN: Record<BootPhase, string> = {
  kernel: '系统内核阶段',
  driver: '驱动加载阶段',
  devices: '设备初始化阶段',
  smss: '系统会话阶段',
  userAuth: '登录认证阶段',
  userInit: '登录过程中',
  shell: '桌面加载时',
  logon: '登录后启动',
  unknown: '不在开机流程中',
}

export const NAME_SOURCE_LABEL: Record<string, string> = {
  fileDescription: '程序自身声明的名称',
  productName: '程序产品名',
  serviceDisplayName: '服务注册的显示名',
  taskDescription: '任务自带的描述',
  fileName: '文件名（未读到程序信息）',
  registryValueName: '注册表项名（未读到程序信息）',
}

/* ─────────────────────────────────────────────────────────────
   启动项有效性（无效启动项检测的结论）
   ───────────────────────────────────────────────────────────── */

export interface ValidityMeta {
  label: string
  color: string
  /** 人话解释，出现在属性面板与悬浮提示里 */
  desc: string
}

export const VALIDITY_META: Record<ValidityStatus, ValidityMeta> = {
  ok: {
    label: '正常',
    color: '#3fb950',
    desc: '目标程序存在且可运行。',
  },
  missingTarget: {
    label: '程序已不在电脑上',
    color: '#6e7681',
    desc: '已失效：启动记录还留着，但它指向的程序文件已经不在了，多半是卸载残留。它不会拖慢开机，但会让清单看起来很拥挤。',
  },
  notExecutable: {
    label: '目标不是可运行程序',
    color: '#d29922',
    desc: '指向的位置存在，但不是一个程序文件。多半来自失败的安装或写错的配置，系统实际上起不来它。',
  },
  unreachable: {
    label: '位置当前访问不到',
    color: '#58a6ff',
    desc: '它指向网络位置或可移动磁盘，而这些设备现在不在电脑上。插回来就会恢复正常——现在够不着不代表它坏了。',
  },
  duplicate: {
    label: '重复的启动入口',
    color: '#d29922',
    desc: '同一个程序注册了多个启动入口，它们都会被拉起。保留一个就够——多出来的不会让它启动得更快，反而容易在你想关掉它的时候漏掉一个。',
  },
  disabledRemnant: {
    label: '已停用但记录还在',
    color: '#6e7681',
    desc: '同一个程序已经有在用的启动入口，而这一条是重复的、并且已经被停用了。它现在不起任何作用，属于可以清理的残留。',
  },
  unknown: {
    label: '无法判定',
    color: '#6e7681',
    desc: '信息不足，无法确认它的目标是否有效。',
  },
}

/* ─────────────────────────────────────────────────────────────
   处置建议
   ───────────────────────────────────────────────────────────── */

export interface AdviceMeta {
  /** 清单行里的短标签 */
  label: string
  color: string
  /** 这个动作意味着什么、可不可逆 */
  desc: string
  /** 是否值得占用清单行的视觉空间 */
  visibleInList: boolean
}

export const ADVICE_META: Record<RecommendationAction, AdviceMeta> = {
  keep: {
    label: '无需处理',
    color: '#3fb950',
    desc: '没有需要你处理的地方。',
    visibleInList: false,
  },
  remove: {
    label: '建议清理',
    color: '#8b949e',
    desc: '这条记录已经没有作用了——要么它指向的程序已经不在这台电脑上，要么它是同一个程序里已经停用的重复入口。清理它不会影响任何正在使用的东西。',
    visibleInList: true,
  },
  disable: {
    label: '建议停用',
    color: '#d29922',
    desc: '停用是可逆的——记录会保留，你随时可以重新开启。',
    visibleInList: true,
  },
  review: {
    label: '建议确认',
    color: '#58a6ff',
    desc: '情况需要你自己判断，我们只把疑点指出来，替你做决定是不合适的。',
    // 「需要你自己看看」的信息不占清单行空间，留给属性面板讲清楚。
    // 否则未签名的小工具一多，清单上会挂满蓝标签，反而淹没真正该处理的项。
    visibleInList: false,
  },
}
