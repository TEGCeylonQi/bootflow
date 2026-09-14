import { useMemo } from 'react'
import { Ban, Power, Trash2, X } from 'lucide-react'
import { useAppStore } from '@/store/useAppStore'
import { usePlanStore } from '@/store/usePlanStore'
import { useCheckedItems, useVisibleIds } from '@/hooks/useSelection'
import { isCleanable } from '@/lib/item'

const PLAN_COLOR = '#a371f7'

/**
 * 批量操作条。
 *
 * 出现在勾选了启动项之后，取代上方的「需要关注」提示条——同一块位置，
 * 同一时刻只回答一个问题："此刻你最可能想做什么"。
 *
 * 【两个刻意的设计】
 *
 * 1. **动作作用范围明确写出来。** 按钮上不写"停用"，而写"停用 3 项"。
 *    批量操作的破坏力全在"我没注意到它选了多少个"上，把数字摆在按钮上，
 *    是最便宜也最有效的一道护栏。
 *
 * 2. **不可编排的项被静默跳过，但明确告知。** 勾选时用户可能带上了系统组件
 *    （用 Ctrl+A 时几乎必然带上）。直接拒绝整个操作很烦人，直接改又很危险，
 *    所以：跳过它们，并在旁边说明跳过了几项、为什么。
 */
export function BatchBar() {
  const checked = useCheckedItems()
  const visibleIds = useVisibleIds()
  const setChecked = useAppStore((s) => s.setChecked)
  const clearChecked = useAppStore((s) => s.clearChecked)
  const stage = usePlanStore((s) => s.stage)
  const setMode = usePlanStore((s) => s.setMode)

  const { targets, skipped, toDisable, toEnable, toClean } = useMemo(() => {
    const allowed = checked.filter((i) => i.risk !== 'Locked' && i.kind !== 'system')
    return {
      targets: allowed,
      skipped: checked.length - allowed.length,
      toDisable: allowed.filter((i) => i.enabled),
      toEnable: allowed.filter((i) => !i.enabled),
      toClean: allowed.filter(isCleanable),
    }
  }, [checked])

  if (checked.length === 0) return null

  /** 进入编排模式再写草稿：否则用户点完看不出发生了什么 */
  const begin = () => setMode('orchestrate')

  const allChecked = visibleIds.length > 0 && checked.length >= visibleIds.length

  return (
    <div
      className="flex shrink-0 flex-col gap-1.5 border-b px-3 py-2"
      style={{ borderColor: `${PLAN_COLOR}4d`, background: `${PLAN_COLOR}0d` }}
    >
      <div className="flex items-center gap-1.5">
        <span className="text-mini text-ink">
          已选 <span className="tnum font-medium">{checked.length}</span> 项
        </span>
        {skipped > 0 && (
          // 不做静默忽略——用户勾了 20 个结果只改了 17 个，不说明就是坑
          <span className="text-2xs text-ink-faint" title="系统组件与禁改项不参与批量修改">
            （{skipped} 项系统组件已跳过）
          </span>
        )}
        <div className="flex-1" />
        <button
          type="button"
          onClick={() => setChecked(allChecked ? [] : visibleIds)}
          className="text-2xs text-ink-dim transition-colors hover:text-accent"
        >
          {allChecked ? '取消全选' : '全选'}
        </button>
        <button
          type="button"
          onClick={clearChecked}
          title="取消全部勾选（Esc）"
          className="text-ink-dim transition-colors hover:text-ink"
        >
          <X size={12} />
        </button>
      </div>

      <div className="flex flex-wrap items-center gap-1">
        <BatchAction
          icon={<Ban size={12} />}
          label="停用"
          count={toDisable.length}
          hint="让它们在开机时不再自动运行。可逆，随时能开回来。"
          onClick={() => {
            begin()
            for (const i of toDisable) stage(i, 'disable', { enabled: false })
          }}
        />
        <BatchAction
          icon={<Power size={12} />}
          label="启用"
          count={toEnable.length}
          hint="恢复这些项的开机自启。"
          onClick={() => {
            begin()
            for (const i of toEnable) stage(i, 'enable', { enabled: true })
          }}
        />
        <BatchAction
          icon={<Trash2 size={12} />}
          label="清理失效"
          count={toClean.length}
          hint="这些项指向的程序已经不在电脑上，或已被停用的重复入口。移除它们只是让清单变干净。"
          onClick={() => {
            begin()
            for (const i of toClean) stage(i, 'remove', { removed: true })
          }}
        />
        <span className="ml-1 text-2xs text-ink-faint">
          改动先收进底部变更篮，确认后才产出方案
        </span>
      </div>

      {targets.length === 0 && (
        <p className="text-2xs leading-4 text-ink-faint">
          所选项目都是 Windows 自带组件——它们不属于能修改的范围，这也是为了保护你的系统。
        </p>
      )}
    </div>
  )
}

function BatchAction({
  icon,
  label,
  count,
  hint,
  onClick,
}: {
  icon: React.ReactNode
  label: string
  count: number
  hint: string
  onClick: () => void
}) {
  const off = count === 0
  return (
    <button
      type="button"
      disabled={off}
      onClick={onClick}
      title={off ? '所选项中不需要这个操作' : hint}
      className="flex items-center gap-1 rounded border px-1.5 py-0.5 text-2xs transition-colors disabled:cursor-not-allowed disabled:opacity-35"
      style={{
        borderColor: off ? '#30363d' : `${PLAN_COLOR}66`,
        color: off ? '#6e7681' : PLAN_COLOR,
      }}
    >
      {icon}
      {label}
      {/* 数字直接标在按钮上：批量操作的风险全在"没注意选了多少" */}
      <span className="tnum">{count}</span>
    </button>
  )
}
