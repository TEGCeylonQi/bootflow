import { useState } from 'react'
import { Loader2, Stethoscope } from 'lucide-react'
import { diagnoseBootPerformance, isTauri, requestElevation } from '@/api/commands'
import type { BootPerformanceDiagnosis } from '@/types/snapshot'

/**
 * 「一键诊断：为什么没有开机性能数据」。
 *
 * 与 BootRecordGuide（只问“开关能不能开”）不同的是**完整诊断**：
 * 后端一次读策略开关 + 性能日志，组合出可行动结论（权限 / 策略禁用 /
 * 快速启动 / 从未记录 / 正常）。
 *
 * 组件分两件套，由外层按需组装：
 * - `useBootDiag()`：状态机（idle/checking/done/error）+ run() + 结果
 * - `DiagTrigger`：按钮（图标 + 文案）
 * - `BootDiagResult`：结果卡（结论 + 说明 + 提权入口 + 证据）
 * - `BootDiagInline`：按钮 + 结果卡直接铺开（嵌在卡片/面板里用）
 */

type DiagState = 'idle' | 'checking' | 'done' | 'error'

export function useBootDiag() {
  const [state, setState] = useState<DiagState>('idle')
  const [diag, setDiag] = useState<BootPerformanceDiagnosis | null>(null)
  const [msg, setMsg] = useState('')

  const run = async () => {
    setState('checking')
    setMsg('')
    setDiag(null)
    try {
      const d = await diagnoseBootPerformance()
      setDiag(d)
      setState('done')
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e))
      setState('error')
    }
  }

  return { state, diag, msg, run }
}

function toneOf(d: BootPerformanceDiagnosis): { color: string; border: string; bg: string } {
  if (d.verdict === '正常') return { color: '#3fb950', border: '#3fb9504d', bg: '#3fb9500f' }
  if (d.verdict === '还没有开机性能记录') return { color: '#d29922', border: '#d299224d', bg: '#d299220f' }
  return { color: '#f0883e', border: '#f0883e4d', bg: '#f0883e0f' }
}

/** 结果卡。 */
export function BootDiagResult({ diag }: { diag: BootPerformanceDiagnosis }) {
  const t = toneOf(diag)
  const elevate = async () => {
    if (isTauri()) await requestElevation()
  }
  return (
    <div className="rounded-md border px-3 py-2" style={{ borderColor: t.border, background: t.bg }}>
      <div className="flex items-center gap-2">
        <span className="text-xs font-medium" style={{ color: t.color }}>
          {diag.verdict}
        </span>
        {diag.needsElevation && isTauri() && (
          <button
            type="button"
            onClick={() => void elevate()}
            className="rounded border border-line px-1.5 py-0.5 text-2xs text-ink-muted transition-colors hover:text-ink"
            title="权限类结论：以管理员身份重开即可读取"
          >
            以管理员权限重开
          </button>
        )}
      </div>

      <p className="mt-1.5 text-2xs leading-5 text-ink-muted">{diag.summary}</p>
      <p className="mt-1 text-2xs leading-5 text-ink-dim">下一步：{diag.action}</p>

      {/* 证据区：可复核 */}
      <p className="mt-1.5 text-2xs leading-4 text-ink-faint">
        依据：系统允许记录={diag.recordSwitch} · 日志记录数={diag.recordCount}
        {diag.lastBootAt ? ` · 最近开机=${diag.lastBootAt}` : ''}
      </p>
    </div>
  )
}

/** 触发按钮。 */
export function DiagTrigger({
  state,
  onClick,
  label = '一键诊断',
}: {
  state: DiagState
  onClick: () => void
  label?: string
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={state === 'checking'}
      className="flex items-center gap-1.5 rounded-md border border-line px-2.5 py-1 text-xs text-ink-muted transition-colors hover:bg-hover hover:text-ink disabled:opacity-50"
      title="一键查出卡在哪一层：权限 / 策略 / 快速启动 / 从未记录"
    >
      {state === 'checking' ? (
        <>
          <Loader2 size={12} className="animate-spin" />
          诊断中…
        </>
      ) : (
        <>
          <Stethoscope size={12} />
          {label}
        </>
      )}
    </button>
  )
}

/** 内嵌形态：按钮 + 结果卡直接铺在下面（GanttView 用法）。 */
export function BootDiagInline() {
  const { state, diag, msg, run } = useBootDiag()
  return (
    <div>
      <DiagTrigger state={state} onClick={() => void run()} />
      {state === 'error' && <p className="mt-2 text-2xs leading-5 text-danger">{msg}</p>}
      {diag && (
        <div className="mt-2">
          <BootDiagResult diag={diag} />
        </div>
      )}
    </div>
  )
}