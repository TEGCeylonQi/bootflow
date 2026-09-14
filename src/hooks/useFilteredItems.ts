import { useMemo } from 'react'
import { useAppStore } from '@/store/useAppStore'
import type { StartupItem } from '@/types/model'
import { displayNameOf, needsAttention, resolveKind } from '@/lib/item'

/**
 * 应用当前的搜索词、类型筛选与风险筛选，返回过滤后的启动项。
 * 左侧清单与中间画布共用此 hook，保证两边始终一致。
 *
 * 搜索范围包含友好名称、原始名称、发布者与命令行——用户既可能按产品名搜，
 * 也可能直接按进程名搜，两种都该命中。
 *
 * 「只看需要关注的」用的是 `needsAttention` 而非单纯的风险等级：
 * 一个目标已被卸载的残留项风险等级是"安全"（它确实无害），
 * 但它显然是用户想处理的东西，所以必须能筛出来。
 *
 * 系统组件默认不显示（`hideSystem`）。`\Microsoft\Windows\` 下的系统维护任务
 * 在任何一台机器上都会占掉清单的大半——它们不是用户能处理的东西，
 * 平铺在画布上只会盖住真正该看的那十几项。
 * 左侧清单早就默认折叠了这一组，画布此前没有跟上，两边对不上。
 * 用户点一下「系统组件」开关就能全部看回来，**数据一条都没丢**。
 */
export function useFilteredItems(): StartupItem[] {
  const items = useAppStore((s) => s.items)
  const query = useAppStore((s) => s.query)
  const kindFilter = useAppStore((s) => s.kindFilter)
  const riskFilter = useAppStore((s) => s.riskFilter)
  const onlyProblems = useAppStore((s) => s.onlyProblems)
  const hideSystem = useAppStore((s) => s.hideSystem)

  return useMemo(() => {
    let list = items

    // 用户显式点了「系统」这个类型标签时不再隐藏——那说明他此刻就想看它们。
    // 两个条件不互相抵消，界面上就不会出现"勾了系统却一个都不显示"的死结。
    if (hideSystem && !kindFilter.includes('system')) {
      list = list.filter((i) => resolveKind(i) !== 'system')
    }

    if (kindFilter.length > 0) list = list.filter((i) => kindFilter.includes(resolveKind(i)))
    if (riskFilter.length > 0) list = list.filter((i) => riskFilter.includes(i.risk))
    if (onlyProblems) list = list.filter(needsAttention)

    const q = query.trim().toLowerCase()
    if (q) {
      list = list.filter(
        (i) =>
          displayNameOf(i).toLowerCase().includes(q) ||
          i.name.toLowerCase().includes(q) ||
          (i.signer.publisher ?? '').toLowerCase().includes(q) ||
          i.command.toLowerCase().includes(q) ||
          i.location.toLowerCase().includes(q) ||
          i.resolvedPath.toLowerCase().includes(q),
      )
    }
    return list
  }, [items, query, kindFilter, riskFilter, onlyProblems, hideSystem])
}
