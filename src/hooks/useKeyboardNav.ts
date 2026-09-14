import { useEffect } from 'react'
import { useAppStore } from '@/store/useAppStore'
import { usePlanStore } from '@/store/usePlanStore'
import { visibleItemIds } from '@/lib/dom'

/**
 * 全局键盘导航。
 *
 * 【为什么导航顺序从 DOM 读，而不是重新算一遍过滤结果】
 * 界面上一共只有一处列表（左侧清单），而"可见顺序"这件事已经由
 * `useFilteredItems` + 分组 + 组内排序共同决定。如果这里再复现一遍同样的
 * 排序逻辑，两份实现迟早会不一致——键盘上下键的落点和眼睛看到的位置错开
 * 一两格，是很难查、又很恼人的问题。
 *
 * 所以直接按 DOM 里 `[data-item-row]` 的出现顺序取 id：
 * **它天然等于用户眼睛看到的顺序**，永远不会漂。
 *
 * 【快捷键清单】（`/` 聚焦搜索、Esc 逐层退出，是这类工具最省心的两个约定）
 *
 *   ↑ / ↓        上/下一个启动项
 *   空格         勾选或取消勾选当前项
 *   Ctrl+A       全选当前可见项
 *   /            跳到搜索框
 *   Esc          依次退出：勾选 → 搜索词 → 编排模式
 *   1 / 2        切换「启动时序 / 耗时分析」
 *   E            切换「体检 / 编排」模式
 *   Ctrl+Z       撤销上一步编排改动
 *   Ctrl+Shift+Z 重做
 */
export function useKeyboardNav() {
  useEffect(() => {
    /** 正在输入时，除了 Esc 全部让给输入框 */
    const isTyping = (el: EventTarget | null): boolean => {
      const t = el as HTMLElement | null
      if (!t) return false
      return t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable
    }

    const move = (delta: number) => {
      const list = visibleItemIds()
      if (list.length === 0) return
      const st = useAppStore.getState()
      const cur = st.selectedId ? list.indexOf(st.selectedId) : -1
      // 没有选中项时，向下从头、向上从尾开始——和大多数列表的手感一致
      const next =
        cur < 0
          ? delta > 0
            ? 0
            : list.length - 1
          : Math.min(list.length - 1, Math.max(0, cur + delta))
      const id = list[next]
      st.select(id)

      // 选中项要跟着滚进视野，否则键盘操作两下之后用户就不知道选到哪了
      requestAnimationFrame(() => {
        document
          .querySelector<HTMLElement>(`[data-item-row="${CSS.escape(id)}"]`)
          ?.scrollIntoView({ block: 'nearest' })
      })
    }

    const onKey = (e: KeyboardEvent) => {
      const st = useAppStore.getState()
      const plan = usePlanStore.getState()
      const typing = isTyping(e.target)
      const mod = e.ctrlKey || e.metaKey

      // ——— Ctrl+Z / Ctrl+Shift+Z：编排撤销与重做 ———
      // 即使正在输入也允许，这是全局习惯；输入框自身的历史很短，冲突可忽略
      if (mod && e.key.toLowerCase() === 'z') {
        e.preventDefault()
        if (e.shiftKey) plan.redo()
        else plan.undo()
        return
      }

      if (e.key === 'Escape') {
        // 逐层退出，不做"一键回到初始状态"——那会让用户丢掉正在做的事
        if (st.checkedIds.length > 0) {
          st.clearChecked()
        } else if (st.query !== '') {
          st.setQuery('')
        } else if (plan.mode === 'orchestrate') {
          plan.setMode('inspect')
        }
        return
      }

      if (typing) return

      switch (e.key) {
        case 'ArrowDown':
          e.preventDefault()
          move(1)
          break

        case 'ArrowUp':
          e.preventDefault()
          move(-1)
          break

        case ' ': {
          // 空格勾选。防止页面滚动
          if (!st.selectedId) break
          e.preventDefault()
          st.toggleCheck(st.selectedId, visibleItemIds(), { additive: true })
          break
        }

        case '/':
          e.preventDefault()
          document.querySelector<HTMLInputElement>('[data-search-input]')?.focus()
          break

        case '1':
          st.setView('swimlane')
          break

        case '2':
          st.setView('gantt')
          break

        case 'e':
        case 'E':
          plan.toggleMode()
          break

        case 'a':
        case 'A':
          if (mod) {
            e.preventDefault()
            st.setChecked(visibleItemIds())
          }
          break
      }
    }

    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
}
