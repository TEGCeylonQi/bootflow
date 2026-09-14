import { memo } from 'react'
import { Check } from 'lucide-react'
import { useAppStore } from '@/store/useAppStore'
import { usePlanStore } from '@/store/usePlanStore'
import { ADVICE_META, KIND_META, RISK_META, VALIDITY_META } from '@/constants'
import type { StartupItem } from '@/types/model'
import { AppIcon } from '@/components/common/Icon'
import { Dot } from '@/components/common/Badge'
import { visibleItemIds } from '@/lib/dom'
import {
  displayNameOf,
  isDeadItem,
  isProblem,
  listableAdvice,
  resolveKind,
  subtitleOf,
} from '@/lib/item'

const PLAN_COLOR = '#a371f7'

/**
 * 清单行。设计原则：**这一行里不出现任何技术术语。**
 *
 * - 名称用统一识别出的友好名（`localsend_app` → `LocalSend`）
 * - 类型徽章回答「这是什么」（应用 / 服务 / 任务 / 注入 / 系统）
 * - 副标题回答「谁做的、什么时候跑」（`Valve · 登录后启动`），
 *   若这一项有处置建议，再追加一个短标签（`· 建议清理`）
 * - 只有确实存在风险的项才出现状态点，正常项保持干净，减少视觉噪音
 *
 * **已失效的项（目标已被卸载）用灰字加删除线淡化**，而不是标红——
 * 它不会拖慢开机（系统找不到文件就跳过了），危害只是让清单显得拥挤。
 * 把它渲染成"危险"会误导用户以为机器有问题。
 *
 * 【编排相关】行首的勾选框只做三件事：表达"我要批量处理这几项"、
 * 让用户看清自己选了多少、以及把结果交给底部的变更篮。
 * 它**不直接改任何东西**——这也让用户敢随便点。
 */
export const ItemRow = memo(function ItemRow({ item }: { item: StartupItem }) {
  const selectedId = useAppStore((s) => s.selectedId)
  const select = useAppStore((s) => s.select)
  const checkedIds = useAppStore((s) => s.checkedIds)
  const toggleCheck = useAppStore((s) => s.toggleCheck)
  /**
   * 订阅 boolean 而非整个 entries 对象。
   * selector 返回布尔值时，zustand 用 Object.is 比较，只有这一项真的
   * 进了/出了草稿才会重渲染——否则任何一项改动都会让整张列表重画。
   */
  const inPlan = usePlanStore((s) => !!s.entries[item.id])

  const active = selectedId === item.id
  const checked = checkedIds.includes(item.id)
  const kind = KIND_META[resolveKind(item)]
  const title = displayNameOf(item)
  const dead = isDeadItem(item)
  // 只取「建议清理 / 建议停用」这类可执行建议；「建议确认」不占清单行空间
  const advice = listableAdvice(item)
  const problem = isProblem(item)

  const validity = VALIDITY_META[item.validity]
  const locked = item.risk === 'Locked' || resolveKind(item) === 'system'

  return (
    <button
      type="button"
      data-item-row={item.id}
      onClick={(e) => {
        // Ctrl / Shift 点击 = 勾选（范围 / 累加），这是列表类界面的通用约定
        if (e.ctrlKey || e.metaKey || e.shiftKey) {
          e.preventDefault()
          toggleCheck(item.id, visibleItemIds(), {
            range: e.shiftKey,
            additive: true,
          })
          return
        }
        select(item.id)
      }}
      title={`${title}\n${item.command}`}
      className={[
        'group flex w-full items-center gap-2 py-1.5 pl-1.5 pr-3 text-left transition-colors',
        active ? 'bg-hover' : 'hover:bg-hover/60',
      ].join(' ')}
      style={
        active
          ? { boxShadow: `inset 2px 0 0 ${checked ? PLAN_COLOR : '#58a6ff'}` }
          : undefined
      }
    >
      {/* 勾选框：系统组件不给勾——它们本来就不可编排，给个能点的框是误导 */}
      {locked ? (
        <span className="w-[15px] shrink-0" />
      ) : (
        <span
          role="checkbox"
          aria-checked={checked}
          aria-label={`选择 ${title}`}
          onClick={(e) => {
            e.stopPropagation()
            toggleCheck(item.id, visibleItemIds(), { additive: true })
          }}
          className={[
            'mt-[1px] flex h-[15px] w-[15px] shrink-0 items-center justify-center rounded-[3px] border transition-colors',
            checked
              ? 'border-transparent'
              : 'border-line opacity-0 group-hover:opacity-100 hover:border-ink-dim',
          ].join(' ')}
          style={checked ? { background: PLAN_COLOR, color: '#0d1117' } : undefined}
        >
          {checked && <Check size={11} strokeWidth={3.5} />}
        </span>
      )}

      <AppIcon name={title} iconData={item.iconData} size={22} />

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span
            className={[
              'truncate text-xs leading-4',
              dead ? 'text-ink-dim line-through decoration-1' : 'text-ink',
            ].join(' ')}
          >
            {title}
          </span>
          <span
            className="shrink-0 rounded px-1 text-2xs font-medium leading-4"
            style={{ color: kind.color, background: `${kind.color}1f` }}
            title={kind.desc}
          >
            {kind.short}
          </span>
        </div>

        <div className="mt-[3px] flex items-center gap-1 text-mini leading-[13px] text-ink-dim">
          {dead ? (
            // 失效项：用有效性结论替掉「谁做的·什么时候跑」——
            // 「它已经不在电脑上了」比「Tencent · 登录后启动」重要得多
            <span className="truncate" title={item.validityDetail ?? validity.desc}>
              {validity.label}
            </span>
          ) : (
            <>
              <span className="truncate">{subtitleOf(item)}</span>
              {advice && (
                <>
                  <span className="shrink-0 text-ink-faint">·</span>
                  <span
                    className="shrink-0 font-medium"
                    style={{ color: ADVICE_META[advice.action].color }}
                    title={ADVICE_META[advice.action].desc}
                  >
                    {ADVICE_META[advice.action].label}
                  </span>
                </>
              )}
            </>
          )}
        </div>
      </div>

      {/* 已在变更篮里：给一个安静但确凿的标记，让用户扫一眼就知道自己动过哪些 */}
      {inPlan && (
        <span
          className="shrink-0 rounded px-1 text-2xs leading-4"
          style={{ color: PLAN_COLOR, background: `${PLAN_COLOR}1f` }}
          title="已放入变更篮，尚未应用"
        >
          已编排
        </span>
      )}

      {problem && (
        <Dot
          color={RISK_META[item.risk].color}
          size={7}
          title={`${RISK_META[item.risk].label}：${RISK_META[item.risk].desc}`}
        />
      )}
    </button>
  )
})
