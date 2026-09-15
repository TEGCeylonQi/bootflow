/**
 * 报告导出。
 *
 * 设计取舍：格式化完全在前端完成，后端只负责把数据给出来。
 * 这样避免了「桌面应用以普通权限运行却要往受保护目录写文件」的权限问题，
 * 落盘交给浏览器的下载能力或系统保存对话框。
 *
 * 导出的内容与界面口径严格一致：一律用统一识别出的友好名称与类型，
 * 不用 `localsend_app` 这类机器名，也不用 `Run · HKCU` 这类技术术语。
 *
 * ⚠️ 有一条额外约束：**报告必须自带"我什么都没改"的声明**。
 * 这份文件的用途是拿给别人看（发给修电脑的、发到群里问人），
 * 而"启动项报告"这个词在很多人的经验里等于"某个软件又要改我电脑了"。
 * 写在开头一行，能省掉一次误会。
 */
import type { OsInfo, ScanResult, StartupItem } from '@/types/model'
import {
  ADVICE_META,
  CONFIDENCE_LABEL,
  KIND_META,
  PHASE_LABEL,
  RISK_META,
  SOURCE_PLAIN,
  VALIDITY_META,
} from '@/constants'
import { displayNameOf, isCleanable, isProblem, needsAttention, resolveKind, tallyAttention } from '@/lib/item'

export type ExportFormat = 'json' | 'csv' | 'markdown'

export const EXPORT_META: Record<ExportFormat, { label: string; ext: string; mime: string }> = {
  json: { label: 'JSON', ext: 'json', mime: 'application/json' },
  csv: { label: 'CSV', ext: 'csv', mime: 'text/csv' },
  markdown: { label: 'Markdown', ext: 'md', mime: 'text/markdown' },
}

const fmtMs = (ms: number) => `${(ms / 1000).toFixed(2)}s`

/**
 * 耗时文案。**估算值绝不给具体时长**——只给"大约什么时候启动"。
 * 给它一个数字，用户就会当成实测值去比较，那是伪造精度。
 */
const timingText = (it: StartupItem): string => {
  const t = it.timing
  if (t.confidence === 'none') return '未记录'
  if (t.confidence === 'measured') return `实测 ${fmtMs(t.durationMs ?? 0)}`
  return `估算（约在第 ${((t.startEstimateMs ?? 0) / 1000).toFixed(1)}s 启动）`
}

function csvCell(v: unknown): string {
  const s = String(v ?? '')
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s
}

/** Markdown 表格单元格：竖线不转义会把一列撑成两列 */
const mdCell = (v: unknown): string => String(v ?? '').replace(/\|/g, '\\|').replace(/\r?\n/g, ' ')

const sysName = (os: OsInfo | null): string =>
  os ? `Windows ${os.major === 10 && os.build >= 22000 ? '11' : '10'} (build ${os.build})` : '—'

const localTime = (iso: string | null): string =>
  iso ? new Date(iso).toLocaleString('zh-CN') : '未记录'

export function filenameFor(format: ExportFormat, scannedAt: string | null): string {
  const d = scannedAt ? new Date(scannedAt) : new Date()
  const p = (n: number) => String(n).padStart(2, '0')
  const stamp = `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}`
  return `bootflow-体检报告-${stamp}.${EXPORT_META[format].ext}`
}

/* ───────────────────────── CSV ───────────────────────── */

export function toCSV(items: StartupItem[]): string {
  const header = [
    '名称',
    '系统内名称',
    '类型',
    '启动方式',
    '风险',
    '有效性',
    '处置建议',
    '建议理由',
    '作用域',
    '是否启用',
    '启动时机',
    '耗时',
    '数据可信度',
    '签名',
    '发布者',
    '注册位置',
    '程序位置',
  ]

  const lines = [header.join(',')]

  for (const it of items) {
    lines.push(
      [
        displayNameOf(it),
        it.name,
        KIND_META[resolveKind(it)].label,
        SOURCE_PLAIN[it.source],
        RISK_META[it.risk].label,
        VALIDITY_META[it.validity].label,
        it.recommendation ? ADVICE_META[it.recommendation.action].label : '',
        it.recommendation?.reason ?? '',
        it.scope === 'machine' ? '所有用户' : '当前用户',
        it.enabled ? '是' : '否',
        PHASE_LABEL[it.bootPhase],
        timingText(it),
        CONFIDENCE_LABEL[it.timing.confidence] ?? it.timing.confidence,
        it.signer.isSigned ? '已签名' : '未签名',
        it.signer.publisher ?? '',
        it.location,
        it.resolvedPath,
      ]
        .map(csvCell)
        .join(','),
    )
  }

  // 加 BOM，避免 Excel 打开中文乱码
  return '\ufeff' + lines.join('\r\n')
}

/* ─────────────────────── Markdown ─────────────────────── */

export function toMarkdown(result: ScanResult): string {
  const { items, os, scannedAt, elevated, bootTimeline } = result
  const tally = tallyAttention(items)
  const risky = items.filter(isProblem)
  const cleanable = items.filter(isCleanable)

  const out: string[] = []
  out.push('# BootFlow 启动项报告')
  out.push('')
  out.push('> 由 BootFlow 生成，只读取，未修改任何系统设置。')
  out.push('')
  out.push(`- 扫描时间：${localTime(scannedAt)}`)
  out.push(`- 系统：${sysName(os)}`)
  out.push(`- 运行权限：${elevated ? '管理员' : '普通权限（个别系统位置可能读取受限）'}`)
  out.push(`- 启动项总数：${items.length}`)
  out.push(
    `- 风险分布：高危 ${items.filter((i) => i.risk === 'High').length} · 注意 ${items.filter((i) => i.risk === 'Medium').length} · 安全 ${items.filter((i) => i.risk === 'Safe').length} · 系统组件 ${items.filter((i) => i.risk === 'Locked').length}`,
  )
  out.push(
    `- 需要你关注：${tally.total} 项（其中高危 ${tally.severe} · 已失效的记录 ${tally.dead} · 有可执行建议 ${tally.advice}）`,
  )

  if (bootTimeline.unavailableReason) {
    out.push(`- 开机耗时：**未读取** —— ${bootTimeline.unavailableReason}`)
  } else if (bootTimeline.totalBootMs) {
    out.push(`- 本次开机耗时：${fmtMs(bootTimeline.totalBootMs)}`)
  } else {
    out.push('- 本次开机耗时：系统还没有记录开机性能数据')
  }
  out.push('')

  /* ——— 需要关注的项 ——— */
  out.push('## 需要你关注的项')
  out.push('')
  if (risky.length > 0) {
    for (const it of risky) {
      out.push(`### ${displayNameOf(it)}　\`${RISK_META[it.risk].label}\``)
      out.push('')
      out.push(`- 类型：${KIND_META[resolveKind(it)].label}（${SOURCE_PLAIN[it.source]}）`)
      out.push(`- 状态：${VALIDITY_META[it.validity].label}`)
      out.push(`- 程序位置：\`${it.resolvedPath || it.command}\``)
      out.push(`- 启动时机：${PHASE_LABEL[it.bootPhase]} · 耗时：${timingText(it)}`)
      if (it.recommendation) {
        out.push(`- 建议：**${ADVICE_META[it.recommendation.action].label}** —— ${it.recommendation.reason}`)
      }
      if (it.validityDetail) out.push(`- 有效性说明：${it.validityDetail}`)
      if (it.riskReasons.length) {
        out.push('- 判定依据：')
        for (const r of it.riskReasons) out.push(`  - ${r}`)
      }
      if (it.diagnostics.length) {
        out.push('- 诊断结论：')
        for (const d of it.diagnostics) {
          out.push(`  - ${d.message}${d.evidence ? `（依据：${d.evidence}）` : ''}`)
        }
      }
      out.push('')
    }
  } else {
    out.push('未发现需要处理的问题。')
    out.push('')
  }

  /* ——— 失效记录 ——— */
  out.push('## 已失效的启动记录')
  out.push('')
  if (cleanable.length > 0) {
    out.push(
      `有 ${cleanable.length} 条记录指向的程序已经不在这台电脑上（或这条记录已被停用）。`,
    )
    out.push('')
    out.push(
      '它们**不会拖慢开机**——系统找不到目标就直接跳过了。把它们找出来，是为了让你看清"真正占着开机的到底有几个"。',
    )
    out.push('')
    for (const it of cleanable) {
      out.push(`- **${displayNameOf(it)}** —— ${it.validityDetail ?? VALIDITY_META[it.validity].desc}`)
    }
    out.push('')
  } else {
    out.push('没有发现失效的启动记录。')
    out.push('')
  }

  /* ——— 开机耗时拆解 ——— */
  out.push('## 开机耗时拆解')
  out.push('')
  if (bootTimeline.unavailableReason) {
    out.push(`**未读取**：${bootTimeline.unavailableReason}`)
    out.push('')
    out.push('这段数据来自系统事件日志，读取它需要管理员权限。它**不影响**上面任何一条启动项的分析。')
    out.push('')
  } else if (bootTimeline.phases.length === 0) {
    out.push('系统还没有为这台电脑记录开机性能数据。Windows 通常在开机偏慢时才会写这份记录。')
    out.push('')
  } else {
    out.push('| 阶段 | 起 | 止 | 耗时 |')
    out.push('| --- | --- | --- | --- |')
    for (const p of bootTimeline.phases) {
      out.push(
        `| ${PHASE_LABEL[p.name]} | ${(p.startMs / 1000).toFixed(1)}s | ${(p.endMs / 1000).toFixed(1)}s | ${(
          (p.endMs - p.startMs) / 1000
        ).toFixed(1)}s |`,
      )
    }
    out.push('')
    if (bootTimeline.slowServices.length > 0) {
      out.push('系统自己标记为「启动偏慢」的项：')
      out.push('')
      out.push('| 名称 | 来源 | 总耗时 | 其中多花 |')
      out.push('| --- | --- | --- | --- |')
      for (const s of bootTimeline.slowServices) {
        const src = s.eventId === 103 ? '服务' : s.eventId === 101 ? '应用' : s.eventId === 102 ? '驱动' : '未知'
        out.push(
          `| ${mdCell(s.friendlyName ?? s.name)} | ${src} | ${fmtMs(s.durationMs)} | ${
            s.degradationMs > 0 ? fmtMs(s.degradationMs) : '—'
          } |`,
        )
      }
      out.push('')
      out.push('> 「其中多花」是系统算出的退化时间：总耗时里有一部分是这项本来就要用的，多花的那部分才真正拖慢了开机。')
      out.push('')
    }
  }

  /* ——— 全部启动项 ——— */
  out.push('## 全部启动项')
  out.push('')
  out.push('| 名称 | 类型 | 启动方式 | 风险 | 状态 | 作用域 | 启动时机 | 耗时 | 发布者 |')
  out.push('| --- | --- | --- | --- | --- | --- | --- | --- | --- |')
  for (const it of items) {
    out.push(
      `| ${mdCell(displayNameOf(it))} | ${KIND_META[resolveKind(it)].label} | ${SOURCE_PLAIN[it.source]} | ${
        RISK_META[it.risk].label
      } | ${VALIDITY_META[it.validity].label} | ${it.scope === 'machine' ? '所有用户' : '当前用户'} | ${
        PHASE_LABEL[it.bootPhase]
      } | ${timingText(it)} | ${mdCell(it.signer.publisher ?? (it.signer.isSigned ? '已签名' : '未签名'))} |`,
    )
  }
  out.push('')

  /* ——— 口径说明 ——— */
  out.push('## 关于这些数字')
  out.push('')
  out.push('- 标「实测」的耗时来自系统事件日志，是这台电脑上一次开机的真实记录。')
  out.push('- 标「估算」的只表示"大约在这个阶段启动"，**不给出时长**——系统没测过的东西，我们不会编一个数字出来。')
  out.push('- 标「未记录」的表示系统没有关于它的任何耗时数据。')
  out.push(`- 本次共 ${items.filter(needsAttention).length} 项被标记为需要关注；其余项没有发现异常。`)
  out.push('')
  if (result.errors.length > 0) {
    out.push('扫描过程中的部分失败（不影响其余结果）：')
    out.push('')
    for (const e of result.errors) out.push(`- ${e}`)
    out.push('')
  }

  return out.join('\n')
}

/* ───────────────────────── 入口 ───────────────────────── */

export function serialize(format: ExportFormat, result: ScanResult): string {
  switch (format) {
    case 'csv':
      return toCSV(result.items)
    case 'markdown':
      return toMarkdown(result)
    case 'json':
      // JSON 是给程序看的，保留完整结构（含 raw），不做"去术语化"处理
      return JSON.stringify(result, null, 2)
  }
}

/** 触发浏览器下载（桌面端 WebView2 同样适用） */
export function download(filename: string, content: string, mime: string): void {
  const blob = new Blob([content], { type: `${mime};charset=utf-8` })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.click()
  URL.revokeObjectURL(url)
}
