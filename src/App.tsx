import { useEffect, useRef } from 'react'
import { useAppStore } from '@/store/useAppStore'
import { useUpdateStore } from '@/store/useUpdateStore'
import { useKeyboardNav } from '@/hooks/useKeyboardNav'
import { TopBar } from '@/components/TopBar'
import { Sidebar } from '@/components/Sidebar/Sidebar'
import { WorkCanvas } from '@/components/Canvas/WorkCanvas'
import { PropertyPanel } from '@/components/Property/PropertyPanel'
import { ChangeDock } from '@/components/Plan/ChangeDock'
import { ColResizer } from '@/components/common/ColResizer'

/** 分栏宽度的默认值与约束，与 store 里的夹取范围保持一致 */
const PANE = {
  sidebar: { def: 300, min: 220, max: 460 },
  property: { def: 340, min: 260, max: 520 },
} as const

/**
 * 启动后多久去问一次「有没有新版本」。
 *
 * 刻意延后：开机的第一件事是让清单出来，一次网络请求不该跟扫描抢那几百毫秒。
 */
const UPDATE_CHECK_DELAY_MS = 3000

/**
 * 四区布局 + 底部变更篮：
 *   ┌────────────────── TopBar ──────────────────┐
 *   │ 清单 │ 画布（启动时序 / 耗时分析） │ 详情  │
 *   ├────────────────────────────────────────────┤
 *   │  变更篮（仅在编排模式或存在草稿时出现）      │
 *   └────────────────────────────────────────────┘
 *
 * 两个细节：
 *
 * - **分栏宽度用户可调、并被记住。** 三栏各司其职，但各人的机器上
 *   "该看清单还是该看详情"差别很大（有人先扫一遍清单，有人点一项看半天）。
 *   写死宽度等于替用户做这个决定。
 *
 * - **变更篮占一整行而不是浮在右侧。** 编排是一件事，不是一个面板里的一个区块。
 *   它需要能被一眼看到、被整体审视，而不是藏在某栏的某个折叠区里。
 *
 * 另外，**更新检查是"顺手问一句"，不是开屏流程的一环**：
 * 它失败不弹窗、不阻塞、不影响任何扫描结果，结论全收在顶栏那个小按钮里，
 * 用户想看才点开。把一件后台的事变成开屏必经的一步，是这类工具常见的失礼。
 */
export default function App() {
  const scan = useAppStore((s) => s.scan)
  const sidebarWidth = useAppStore((s) => s.sidebarWidth)
  const propertyWidth = useAppStore((s) => s.propertyWidth)
  const setPaneWidth = useAppStore((s) => s.setPaneWidth)
  const checkUpdate = useUpdateStore((s) => s.check)
  const booted = useRef(false)
  const updateChecked = useRef(false)

  useKeyboardNav()

  useEffect(() => {
    // StrictMode 下 effect 会跑两次，用 ref 挡住重复扫描
    if (booted.current) return
    booted.current = true
    void scan()
  }, [scan])

  useEffect(() => {
    // 同上：开发模式下 effect 会跑两次，挡住重复的更新检查
    if (updateChecked.current) return
    updateChecked.current = true

    const timer = window.setTimeout(() => void checkUpdate(), UPDATE_CHECK_DELAY_MS)
    return () => window.clearTimeout(timer)
  }, [checkUpdate])

  return (
    <div className="grid h-full grid-rows-[48px_minmax(0,1fr)_auto] overflow-hidden bg-base">
      <TopBar />

      <div
        className="grid min-h-0"
        style={{
          gridTemplateColumns: `${sidebarWidth}px 5px minmax(0,1fr) 5px ${propertyWidth}px`,
        }}
      >
        <Sidebar />

        <ColResizer
          label="调整清单宽度"
          side="left"
          width={sidebarWidth}
          onChange={(w) => setPaneWidth('sidebar', w)}
          min={PANE.sidebar.min}
          max={PANE.sidebar.max}
          resetWidth={PANE.sidebar.def}
        />

        <main className="min-h-0 min-w-0">
          <WorkCanvas />
        </main>

        <ColResizer
          label="调整详情宽度"
          side="right"
          width={propertyWidth}
          onChange={(w) => setPaneWidth('property', w)}
          min={PANE.property.min}
          max={PANE.property.max}
          resetWidth={PANE.property.def}
        />

        <PropertyPanel />
      </div>

      <ChangeDock />
    </div>
  )
}
