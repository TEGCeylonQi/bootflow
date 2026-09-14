/**
 * 启动项的「人性化」转换层。
 *
 * 这个文件的职责只有一个：把后端扫出来的技术原始数据，翻译成普通人能读懂的东西。
 * 界面层不允许自己去拼 `HKLM\WOW6432Node` 之类的字符串，也不允许直接把
 * `localsend_app` 这类程序内部名丢给用户看——一律走这里。
 *
 * 【与 Rust 侧的分工】后端负责读 exe 版本信息拿到真正的友好名称；
 * 前端这层的派生函数是**兜底**，保证后端字段缺失时界面不会崩、也不会露出生数据。
 */
import type { BootPhase, ItemKind, Recommendation, StartupItem } from '@/types/model'
import { ADVICE_META, KIND_META, PHASE_SLOGAN } from '@/constants'

type KindInput = Pick<StartupItem, 'source' | 'signer'>

/**
 * 类型判定链（顺序即优先级）：
 *   系统注入 > Windows 自带组件 > 服务 > 计划任务 > 应用程序
 *
 * 为什么 isOsComponent 排在 Service 前面？因为「Windows Audio」「RPC」这类东西
 * 虽然形态上是服务，但对用户而言它们属于「系统自带、别碰」这一档，
 * 归到「系统组件」组比混在第三方服务里更符合直觉。
 */
export function deriveKind(i: KindInput): ItemKind {
  if (i.source === 'SystemHook') return 'hook'
  if (i.signer.isOsComponent) return 'system'
  if (i.source === 'Service') return 'service'
  if (i.source === 'ScheduledTask') return 'task'
  return 'app'
}

/** 取类型：优先后端字段，缺失时前端兜底派生。所有界面都该用这个，不要直接读 item.kind */
export function resolveKind(i: StartupItem): ItemKind {
  const explicit = i.kind as ItemKind | undefined
  return explicit ?? deriveKind(i)
}

/** 统一名称：优先友好名，回退原始名。界面上一律用这个，不要直接读 item.name */
export function displayNameOf(i: StartupItem): string {
  const d = i.displayName?.trim()
  return d || i.name
}

/** 友好名与原始名不一致时，说明存在「系统里实际叫另一个名字」的情况 */
export function hasAlias(i: StartupItem): boolean {
  const d = i.displayName?.trim()
  return !!d && d !== i.name
}

/**
 * 少数几个「证书 CN 是全称、中文用户更认简称」的发布者。
 *
 * 刻意保持极简，而且只收**公司级**的通用名称。
 * 具体某个软件的品牌不要往这里加——那既无法穷举，也会让这层退化成一份
 * 特定软件清单（维护不了，也不该由代码硬编码）。其余情况走下面的通用截断。
 */
const PUBLISHER_ALIASES: [string, string][] = [
  ['tencent', '腾讯'],
  ['netease', '网易'],
]

/**
 * 发布者简称。证书 CN 经常是
 * 「Tencent Technology(Shenzhen) Company Limited」这种全称，直接显示会撑爆版面。
 *
 * 通用策略：取第一个分隔符（逗号 / 中文逗号 / 括号）之前的部分，过长再截断。
 * 这样任何厂商都能得到一个可读的短名，不需要为它单独写规则。
 */
export function shortPublisher(publisher?: string): string | undefined {
  if (!publisher) return undefined
  const low = publisher.toLowerCase()
  for (const [needle, label] of PUBLISHER_ALIASES) {
    if (low.includes(needle)) return label
  }
  const first = publisher.split(/[,，（(]/)[0].trim()
  if (!first) return undefined
  return first.length > 24 ? `${first.slice(0, 22)}…` : first
}

/** 启动时机的口语表达 */
export function phaseSlogan(phase: BootPhase): string {
  return PHASE_SLOGAN[phase] ?? '不在开机流程中'
}

/**
 * 清单行的副标题：`发布者 · 启动时机`
 * 例：`Valve · 登录后启动`、`未签名 · 登录后启动`
 */
export function subtitleOf(i: StartupItem): string {
  const who = shortPublisher(i.signer.publisher) ?? (i.signer.isSigned ? '来源未知' : '未签名')
  return `${who} · ${phaseSlogan(i.bootPhase)}`
}

const KIND_NOUN: Record<ItemKind, string> = {
  app: '应用程序',
  service: '后台服务',
  task: '计划任务',
  hook: '系统注入项',
  system: 'Windows 系统组件',
}

/**
 * 一段人话说明。后端提供 summary 时优先用它，否则按
 * 「类型 + 发布者 + 启动时机」组装。
 */
export function summarize(i: StartupItem): string {
  const custom = i.summary?.trim()
  if (custom) return custom

  const kind = resolveKind(i)
  if (kind === 'system') {
    return `${KIND_NOUN.system}，由 Windows 自带。这类组件不建议改动，本工具也不提供修改入口。`
  }

  const who = shortPublisher(i.signer.publisher)
  const whoPart = who ? `由 ${who} 提供` : '发布者无法核验'
  const whenPart = i.enabled ? `，会在${phaseSlogan(i.bootPhase)}时运行` : '，当前已停用'

  if (kind === 'hook') {
    return `${KIND_NOUN.hook}，${whoPart}。它不是一个独立程序，而是被注入到其它程序里一起运行——因此影响范围会远超它自己。`
  }

  return `这是一个${KIND_NOUN[kind]}，${whoPart}${whenPart}。`
}

/** 类型徽章文案 */
export function kindShort(i: StartupItem): string {
  return KIND_META[resolveKind(i)].short
}

/* ─────────────── 问题分级：用于「需要关注」提示条 ─────────────── */

/** 是否值得用户关注（禁改区的系统组件不算——那不是用户能处理的事） */
export function isProblem(i: StartupItem): boolean {
  return i.risk === 'High' || i.risk === 'Medium'
}

export function isSevere(i: StartupItem): boolean {
  return i.risk === 'High'
}

/* ─────────────── 有效性（无效启动项） ─────────────── */

/**
 * 目标已经不存在或不可执行。
 *
 * 注意这类项**不会拖慢开机**——系统找不到文件就直接跳过了。
 * 它们的危害是"污染判断"：让用户以为自己的启动项比实际多。
 * 所以界面上把它们淡化处理，而不是标红吓人。
 */
export function isDeadItem(i: StartupItem): boolean {
  return i.validity === 'missingTarget' || i.validity === 'notExecutable'
}

/**
 * 这条记录是否"已经没有作用了"——清掉它不会影响任何正在使用的东西。
 *
 * 比 `isDeadItem` 宽一档：除了"目标已经不在"，还包含
 * `disabledRemnant`（同一个程序已有在用的入口，这条是重复且已停用的）。
 * 后者的程序**还在**，所以 `isDeadItem` 为假，但这条记录同样是纯残留。
 *
 * 判据统一放在这里，是为了让批量清理和单项建议用同一把尺子——
 * 两边各写一套的话，用户会遇到"单看说是残留、批量清理却跳过了它"的矛盾。
 */
export function isCleanable(i: StartupItem): boolean {
  return (
    i.validity === 'missingTarget' ||
    i.validity === 'notExecutable' ||
    i.validity === 'disabledRemnant'
  )
}

/* ─────────────── 处置建议 ─────────────── */

/**
 * 值得占用清单行空间的建议（`remove` / `disable`）。
 *
 * `review`（"建议你自己确认一下"）刻意不在这里返回——
 * 未签名的小工具一多，清单会挂满蓝色标签，反而淹没真正该处理的那几条。
 * 它仍然会在属性面板里完整呈现。
 */
export function listableAdvice(i: StartupItem): Recommendation | undefined {
  const r = i.recommendation
  if (!r) return undefined
  return ADVICE_META[r.action]?.visibleInList ? r : undefined
}

/** 统一的"需要关注"判据：有风险、已失效、或存在可执行建议 */
export function needsAttention(i: StartupItem): boolean {
  return isProblem(i) || isDeadItem(i) || !!listableAdvice(i)
}

export interface AttentionTally {
  /** 风险等级为高 */
  severe: number
  /** 风险等级为注意 */
  minor: number
  /** 目标已不存在（卸载残留） */
  dead: number
  /** 有可执行建议（建议清理 / 建议停用） */
  advice: number
  /** 满足 needsAttention 的项数，用于提示条的主数字 */
  total: number
}

export function tallyAttention(items: StartupItem[]): AttentionTally {
  let severe = 0
  let minor = 0
  let dead = 0
  let advice = 0
  let total = 0

  for (const i of items) {
    if (i.risk === 'High') severe += 1
    else if (i.risk === 'Medium') minor += 1
    if (isDeadItem(i)) dead += 1
    if (listableAdvice(i)) advice += 1
    if (needsAttention(i)) total += 1
  }

  return { severe, minor, dead, advice, total }
}
