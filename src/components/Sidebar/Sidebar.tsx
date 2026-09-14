import { useMemo } from 'react'
import { useAppStore } from '@/store/useAppStore'
import { useFilteredItems } from '@/hooks/useFilteredItems'
import { KIND_ORDER } from '@/constants'
import type { ItemKind, StartupItem } from '@/types/model'
import { isProblem, resolveKind } from '@/lib/item'
import { FilterBar } from './FilterBar'
import { ItemList } from './ItemList'
import { AttentionBanner } from './AttentionBanner'
import { BatchBar } from './BatchBar'

export function Sidebar() {
  const items = useAppStore((s) => s.items)
  const status = useAppStore((s) => s.status)
  const checkedIds = useAppStore((s) => s.checkedIds)
  const filtered = useFilteredItems()

  /** 按「类型」分组，空组不显示；组内把有问题的项排到前面 */
  const grouped = useMemo(() => {
    const map = new Map<ItemKind, StartupItem[]>()
    for (const k of KIND_ORDER) map.set(k, [])
    for (const it of filtered) map.get(resolveKind(it))?.push(it)

    for (const [, list] of map) {
      list.sort((a, b) => {
        const pa = isProblem(a) ? 0 : 1
        const pb = isProblem(b) ? 0 : 1
        if (pa !== pb) return pa - pb
        return (a.timing.startEstimateMs ?? 1e9) - (b.timing.startEstimateMs ?? 1e9)
      })
    }

    return [...map.entries()].filter(([, v]) => v.length > 0)
  }, [filtered])

  const loading = status === 'scanning' && items.length === 0

  return (
    <aside className="flex min-h-0 flex-col bg-panel">
      <FilterBar total={items.length} shown={filtered.length} />

      {/*
       * 勾选之后，「需要关注」提示条让位给批量操作条。
       * 同一块位置同一时刻只回答一个问题："你现在最可能想做什么"。
       * 两条一起堆着，用户得先分辨哪条跟当前动作有关。
       */}
      {checkedIds.length > 0 ? <BatchBar /> : <AttentionBanner />}
      <div className="min-h-0 flex-1 overflow-y-auto scroll-thin">
        {loading ? (
          <SkeletonList />
        ) : grouped.length === 0 ? (
          <EmptyState total={items.length} />
        ) : (
          <ItemList groups={grouped} />
        )}
      </div>
    </aside>
  )
}

/**
 * 空状态。
 *
 * 必须区分两种「空」——它们的含义完全相反：
 * - **筛选后为空**：机器上有启动项，只是当前筛选条件没匹配上
 * - **完全为空**：一个都没读到
 *
 * 第二种在正常机器上几乎不可能发生（每台 Windows 都有一批自带的计划任务）。
 * 一旦出现，最可能的原因是权限：部分位置需要管理员才能读取。
 * 所以这里**不能**说成"你的电脑很干净"——那会把一次失败的读取
 * 说成一个好消息，用户再也不会去查。
 */
function EmptyState({ total }: { total: number }) {
  if (total > 0) {
    return (
      <p className="px-3 py-6 text-center text-xs text-ink-dim">没有符合条件的启动项</p>
    )
  }

  return (
    <div className="px-4 py-8 text-center">
      <p className="text-xs leading-5 text-ink-dim">没有读到任何启动项</p>
      <p className="mt-1.5 text-mini leading-4 text-ink-faint">
        启动文件夹、注册表启动项、计划任务三个位置都已扫描。
        <br />
        每台 Windows 都自带一批计划任务，这里完全为空通常意味着读取被拒绝——
        可以试试以管理员身份运行。
      </p>
    </div>
  )
}

function SkeletonList() {
  return (
    <div className="space-y-2 p-3">
      {Array.from({ length: 10 }).map((_, i) => (
        <div
          key={i}
          className="pulse-soft flex items-center gap-2"
          style={{ animationDelay: `${i * 90}ms` }}
        >
          <div className="h-[22px] w-[22px] rounded bg-hover" />
          <div className="flex-1 space-y-1">
            <div className="h-3 rounded bg-hover" style={{ width: `${50 + ((i * 13) % 40)}%` }} />
            <div className="h-2.5 rounded bg-hover/70" style={{ width: `${30 + ((i * 7) % 30)}%` }} />
          </div>
        </div>
      ))}
    </div>
  )
}
