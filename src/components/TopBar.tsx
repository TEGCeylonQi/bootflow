import { useMemo } from 'react'
import { Clock, Loader2, Monitor, RefreshCw, ShieldCheck, ShieldX, SlidersHorizontal, Stethoscope } from 'lucide-react'
import { useAppStore } from '@/store/useAppStore'
import { usePlanStore } from '@/store/usePlanStore'
import { RISK_META } from '@/constants'
import type { RiskLevel } from '@/types/model'
import type { AppMode } from '@/types/plan'
import { Badge, Dot } from '@/components/common/Badge'
import { ExportMenu } from '@/components/ExportMenu'

const PLAN_COLOR = '#a371f7'

function fmtTime(iso: string | null): string {
  if (!iso) return '尚未扫描'
  const d = new Date(iso)
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}:${String(
    d.getSeconds(),
  ).padStart(2, '0')}`
}

function fmtSeconds(ms?: number): string {
  if (!ms) return '—'
  return `${(ms / 1000).toFixed(1)}s`
}

/**
 * 顶栏。
 *
 * 【模式切换为什么放在这里，而不是藏在某个菜单里】
 * 「体检」和「编排」回答的是两个不同的问题：**现在是什么样** 与 **我想要什么样**。
 * 它们决定了整屏每一个数值的语义，所以必须是常驻可见的、一眼可辨的状态，
 * 而不是一个需要点开才知道当前处于哪种模式的开关。
 *
 * 编排模式下顶栏左侧会亮起一道紫色竖线：用户切走视线再回来时，
 * 能立刻确认"我现在是在动真格的那一侧"。
 */
export function TopBar() {
  const items = useAppStore((s) => s.items)
  const os = useAppStore((s) => s.os)
  const elevated = useAppStore((s) => s.elevated)
  const scannedAt = useAppStore((s) => s.scannedAt)
  const status = useAppStore((s) => s.status)
  const bootTimeline = useAppStore((s) => s.bootTimeline)
  const scan = useAppStore((s) => s.scan)

  const mode = usePlanStore((s) => s.mode)
  const setMode = usePlanStore((s) => s.setMode)
  const changeCount = usePlanStore((s) => s.order.length)

  const riskCount = useMemo(() => {
    const c: Record<RiskLevel, number> = { Locked: 0, High: 0, Medium: 0, Safe: 0 }
    for (const it of items) c[it.risk] += 1
    return c
  }, [items])

  const scanning = status === 'scanning'
  const attention = riskCount.High + riskCount.Medium

  return (
    <header
      className="flex items-center gap-4 border-b bg-panel pl-3 pr-4"
      style={{
        borderColor: mode === 'orchestrate' ? `${PLAN_COLOR}4d` : '#30363d',
        // 模式色的极淡底纹：够看出状态，又不至于让顶栏变成两种颜色
        background: mode === 'orchestrate' ? `${PLAN_COLOR}0a` : undefined,
      }}
    >
      {/*
       * 模式闸门。放在最左侧是有意的：它决定右边所有数字的含义，
       * 所以必须在读数字之前先读到它。
       */}
      <ModeSwitch mode={mode} onChange={setMode} changeCount={changeCount} />

      <div className="h-5 w-px shrink-0 bg-line" />

      {/* 概览指标 */}
      <div className="flex items-center gap-4 text-xs">
        <div className="flex items-center gap-1.5" title="扫描到的自启项总数">
          <span className="text-ink-muted">启动项</span>
          <span className="tnum font-semibold text-ink">{items.length}</span>
        </div>

        {attention > 0 && (
          <div
            className="flex items-center gap-1.5"
            title={`其中高危 ${riskCount.High} 项，注意 ${riskCount.Medium} 项`}
          >
            <Dot color={RISK_META.High.color} />
            <span className="text-ink-muted">待处理</span>
            <span className="tnum font-semibold" style={{ color: RISK_META.High.color }}>
              {attention}
            </span>
          </div>
        )}

        {/*
         * 「开机耗时」有三种状态，必须分开呈现。
         * 尤其第三种：**没权限读**显示成"—"最容易被读成"0 秒、开机飞快"，
         * 那是这个界面最容易撒的一个谎，所以这里显式写成「未读取」并给出原因。
         */}
        <div
          className="flex items-center gap-1.5"
          title={
            bootTimeline?.unavailableReason
              ? `未读取：${bootTimeline.unavailableReason}`
              : bootTimeline?.totalBootMs
                ? '本次开机各阶段耗时合计'
                : '系统还没有记录开机性能数据'
          }
        >
          <Clock size={13} className="text-ink-dim" />
          <span className="text-ink-muted">开机耗时</span>
          {bootTimeline?.unavailableReason ? (
            <span className="font-semibold" style={{ color: '#d29922' }}>
              未读取
            </span>
          ) : (
            <span className="tnum font-semibold text-ink">{fmtSeconds(bootTimeline?.totalBootMs)}</span>
          )}
        </div>
      </div>

      <div className="flex-1" />

      {/* 环境状态 */}
      <div className="flex items-center gap-2">
        <Badge color="#8b949e" title="操作系统版本">
          <Monitor size={11} />
          {os ? `Windows ${os.major === 10 && os.build >= 22000 ? '11' : '10'} · ${os.build}` : '—'}
        </Badge>

        <Badge
          color={elevated ? '#3fb950' : '#8b949e'}
          title={elevated ? '已以管理员身份运行，可读取全部来源' : '普通权限运行，个别系统位置可能读取受限'}
        >
          {elevated ? <ShieldCheck size={11} /> : <ShieldX size={11} />}
          {elevated ? '管理员' : '普通权限'}
        </Badge>

        <span className="text-2xs text-ink-dim" title="上次扫描完成时间">
          {fmtTime(scannedAt)}
        </span>

        <button
          type="button"
          onClick={() => void scan()}
          disabled={scanning}
          className="flex items-center gap-1.5 rounded-md border border-line bg-elevated px-2.5 py-1 text-xs text-ink transition-colors hover:border-accent hover:text-accent disabled:cursor-not-allowed disabled:opacity-60"
        >
          {scanning ? (
            <>
              <Loader2 size={13} className="animate-spin" />
              扫描中
            </>
          ) : (
            <>
              <RefreshCw size={13} />
              重新扫描
            </>
          )}
        </button>

        <ExportMenu />
      </div>
    </header>
  )
}

function ModeSwitch({
  mode,
  onChange,
  changeCount,
}: {
  mode: AppMode
  onChange: (m: AppMode) => void
  changeCount: number
}) {
  return (
    <div className="flex shrink-0 items-center gap-1.5">
      <span className="text-[15px] font-semibold tracking-tight text-ink">BootFlow</span>

      <div className="ml-1 flex items-center rounded-md border border-line bg-base p-0.5">
        <ModeTab
          active={mode === 'inspect'}
          onClick={() => onChange('inspect')}
          icon={<Stethoscope size={12} />}
          label="体检"
          tone="neutral"
          hint="只读查看：这台电脑开机时到底跑了些什么"
        />
        <ModeTab
          active={mode === 'orchestrate'}
          onClick={() => onChange('orchestrate')}
          icon={<SlidersHorizontal size={12} />}
          label="编排"
          tone="plan"
          hint="表达意图：把想改的收进变更篮，确认后生成方案（按 E 切换）"
        />
      </div>

      {/*
       * 变更计数贴在切换器外面而不是里面：它属于"编排"这个模式的结果，
       * 但用户可能已经切回体检去核对别的项，这时仍需要知道手上还攥着几个改动。
       */}
      {changeCount > 0 && (
        <span
          className="tnum rounded px-1.5 py-[1px] text-2xs leading-4"
          style={{ color: PLAN_COLOR, background: `${PLAN_COLOR}1f` }}
          title={`已记录 ${changeCount} 项待应用的变更`}
        >
          {changeCount}
        </span>
      )}
    </div>
  )
}

function ModeTab({
  active,
  onClick,
  icon,
  label,
  tone,
  hint,
}: {
  active: boolean
  onClick: () => void
  icon: React.ReactNode
  label: string
  /** plan 用编排紫，neutral 用普通高亮——两种模式的视觉语言必须分开 */
  tone: 'neutral' | 'plan'
  hint: string
}) {
  const accent = tone === 'plan' ? PLAN_COLOR : '#e6edf3'
  return (
    <button
      type="button"
      onClick={onClick}
      title={hint}
      aria-pressed={active}
      className="flex items-center gap-1 rounded px-2 py-[3px] text-mini transition-colors"
      style={{
        background: active ? (tone === 'plan' ? `${PLAN_COLOR}26` : '#21262d') : 'transparent',
        color: active ? accent : '#8b949e',
      }}
    >
      {icon}
      {label}
    </button>
  )
}
