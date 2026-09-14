import { useMemo } from 'react'
import { useAppStore } from '@/store/useAppStore'
import { useFilteredItems } from './useFilteredItems'
import type { StartupItem } from '@/types/model'

/**
 * 当前**既被勾选、又可见**的项。
 *
 * 为什么一定要同时满足两个条件，而不是只取勾选集合：
 * 用户可以勾选一批、再改搜索词把其中一部分过滤掉。这时候如果他执行
 * "批量停用"，按勾选集合去做就会改到屏幕上根本看不见的项——
 * 这是这类功能里最容易出、后果也最严重的一类事故。
 *
 * 所以对外一律以这个 hook 为准：**眼睛看不到的，手就改不到。**
 */
export function useCheckedItems(): StartupItem[] {
  const filtered = useFilteredItems()
  const checkedIds = useAppStore((s) => s.checkedIds)

  return useMemo(() => {
    if (checkedIds.length === 0) return []
    const set = new Set(checkedIds)
    return filtered.filter((i) => set.has(i.id))
  }, [filtered, checkedIds])
}

/** 可勾选（可编排）的子集——批量操作拿去用这个 */
export function useOrchestrableChecked(): StartupItem[] {
  const checked = useCheckedItems()
  return useMemo(
    () => checked.filter((i) => i.risk !== 'Locked' && i.kind !== 'system'),
    [checked],
  )
}

/** 当前可见项的 id 顺序，用于范围选择与全选 */
export function useVisibleIds(): string[] {
  const filtered = useFilteredItems()
  return useMemo(() => filtered.map((i) => i.id), [filtered])
}
