import { Eye, ShieldCheck, Trash2, TriangleAlert } from 'lucide-react'
import type { DiagnosticInfo, Recommendation, StartupItem } from '@/types/model'
import { ADVICE_META, CONFIDENCE_LABEL, VALIDITY_META } from '@/constants'
import { RiskBadge } from './RiskBadge'
import { DiagnosticsBlock } from './DiagnosticsBlock'

/**
 * 把诊断结论拆成两类：
 * - issues：真正需要用户处理的问题（High / Medium）
 * - notes：只是补充说明，不构成问题（Safe 级，例如「该任务不含开机触发器」）
 *
 * 混在一起会让用户以为一切都是毛病，所以必须分开。
 */
export function splitDiagnostics(items: DiagnosticInfo[]): {
  issues: DiagnosticInfo[]
  notes: DiagnosticInfo[]
} {
  const issues: DiagnosticInfo[] = []
  const notes: DiagnosticInfo[] = []
  for (const d of items) {
    if (d.severity === 'High' || d.severity === 'Medium') issues.push(d)
    else notes.push(d)
  }
  return { issues, notes }
}

const ADVICE_ICON = {
  remove: Trash2,
  disable: TriangleAlert,
  review: Eye,
  keep: ShieldCheck,
} as const

/**
 * 「建议怎么做」区块。
 *
 * 这是本版本新增的核心能力：**只给结论和理由，不给按钮。**
 * 界面上必须显式说明"为什么没有一键处理"——否则用户会觉得功能没做完，
 * 或者更糟，去别处找工具乱删。诚实说明边界比假装完整重要。
 */
function AdviceCard({ rec }: { rec: Recommendation }) {
  const meta = ADVICE_META[rec.action]
  const Icon = ADVICE_ICON[rec.action]

  return (
    <div
      className="rounded border px-2 py-1.5"
      style={{ borderColor: `${meta.color}4d`, background: `${meta.color}14` }}
    >
      <div className="mb-1 flex items-center gap-1.5">
        <Icon size={12} className="shrink-0" style={{ color: meta.color }} />
        <span className="text-mini font-medium leading-4" style={{ color: meta.color }}>
          {meta.label}
        </span>
        <span className="flex-1" />
        <span className="shrink-0 text-2xs leading-3 text-ink-faint">
          把握：{CONFIDENCE_LABEL[rec.confidence] ?? '未知'}
        </span>
      </div>

      <p className="text-mini leading-4 text-ink-muted">{rec.reason}</p>

      <p className="mt-1.5 border-t border-line-subtle pt-1.5 text-2xs leading-3 text-ink-faint">
        本版本只做检测与建议，不提供一键处理。执行能力将在下个版本开放，届时每一步都可撤销。
      </p>
    </div>
  )
}

/**
 * 「需要你关注」区块。
 *
 * 顺序是刻意的：**结论在前、证据在后**。
 * 1. 处置建议（最有用，一句话说清该怎么办）
 * 2. 有效性结论（它是不是还活着）
 * 3. 风险等级与诊断证据（技术细节，支持前两条）
 */
export function AttentionBlock({ item }: { item: StartupItem }) {
  const { issues } = splitDiagnostics(item.diagnostics)
  const clean = issues.length === 0

  const rec = item.recommendation
  const showAdvice = !!rec && rec.action !== 'keep'
  const showValidity = item.validity !== 'ok'
  const validity = VALIDITY_META[item.validity]

  const nothingToSay = !showAdvice && !showValidity && clean && item.risk === 'Safe'

  return (
    <section className="border-b border-line-subtle px-3 py-3">
      <h3 className="mb-2 text-mini font-medium tracking-wide text-ink-muted">
        {nothingToSay ? '运行情况' : '需要你关注'}
      </h3>

      <div className="space-y-2">
        {showAdvice && <AdviceCard rec={rec} />}

        {showValidity && (
          <div className="flex items-start gap-1.5 rounded border border-line-subtle bg-panel-soft px-2 py-1.5">
            <span
              className="mt-[5px] h-1.5 w-1.5 shrink-0 rounded-full"
              style={{ background: validity.color }}
            />
            <div className="min-w-0">
              <p className="text-mini leading-4" style={{ color: validity.color }}>
                {validity.label}
              </p>
              <p className="mt-0.5 text-mini leading-4 text-ink-muted">
                {item.validityDetail ?? validity.desc}
              </p>
            </div>
          </div>
        )}

        {item.risk !== 'Safe' && <RiskBadge level={item.risk} showDesc />}

        {clean ? (
          nothingToSay && (
            <div className="flex items-start gap-1.5 rounded border border-ok/30 bg-ok/5 px-2 py-1.5">
              <ShieldCheck size={12} className="mt-[2px] shrink-0 text-ok" />
              <p className="text-mini leading-4 text-ink-muted">
                {item.risk === 'Locked'
                  ? '这是系统自带组件，不属于需要处理的问题。本工具不提供修改入口。'
                  : '未发现异常，这一项没有需要你处理的问题。'}
              </p>
            </div>
          )
        ) : (
          <DiagnosticsBlock items={issues} />
        )}
      </div>
    </section>
  )
}
