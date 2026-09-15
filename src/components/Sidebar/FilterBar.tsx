import { Search, X } from 'lucide-react'
import { useAppStore } from '@/store/useAppStore'
import { KIND_META, KIND_ORDER, RISK_META, RISK_ORDER } from '@/constants'
import type { RiskLevel, SortBy } from '@/types/model'
import { Dot } from '@/components/common/Badge'

interface Props {
  total: number
  shown: number
}

/**
 * 筛选条。第一行筛选「这是什么」（用户能理解的维度），第二行筛选「有没有问题」。
 * 「注册在注册表还是启动文件夹」属于二级信息，不放在这里。
 */
/**
 * 排序选项。
 *
 * ⚠️ 这两个不是同一件事的两种排法：一个按**时间**，一个按**开销**。
 * 时序上排在后面的项完全可能是最费资源的那个，所以必须让用户能切换，
 * 而不是替他决定看哪个。措辞也不写"排序方式"，直接写问题本身。
 */
const SORT_OPTIONS: { value: SortBy; label: string; hint: string }[] = [
  {
    value: 'default',
    label: '按开机顺序',
    hint: '谁先跑起来：有问题的排前面，其余按实测出现时刻',
  },
  {
    value: 'impact',
    label: '按资源占用',
    hint: '谁最费资源：按 Windows 自记的启动影响从重到轻（需管理员权限才有这份数据；没有数据的项排在最后）',
  },
]

export function FilterBar({ total, shown }: Props) {
  const query = useAppStore((s) => s.query)
  const setQuery = useAppStore((s) => s.setQuery)
  const kindFilter = useAppStore((s) => s.kindFilter)
  const toggleKind = useAppStore((s) => s.toggleKind)
  const riskFilter = useAppStore((s) => s.riskFilter)
  const toggleRisk = useAppStore((s) => s.toggleRisk)
  const onlyProblems = useAppStore((s) => s.onlyProblems)
  const hideSystem = useAppStore((s) => s.hideSystem)
  const toggleSystem = useAppStore((s) => s.toggleSystem)
  const clearFilters = useAppStore((s) => s.clearFilters)
  const sortBy = useAppStore((s) => s.sortBy)
  const setSortBy = useAppStore((s) => s.setSortBy)

  const dirty =
    query.trim() !== '' || kindFilter.length > 0 || riskFilter.length > 0 || onlyProblems

  return (
    <div className="shrink-0 border-b border-line-subtle px-3 pb-2 pt-2.5">
      <div className="flex items-center gap-1.5 rounded-md border border-line bg-base px-2 py-1 focus-within:border-accent">
        <Search size={13} className="shrink-0 text-ink-dim" />
        <input
          data-search-input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜索名称或发布者（按 / 聚焦）"
          className="w-full bg-transparent text-xs text-ink placeholder:text-ink-dim focus:outline-none"
        />
        {query && (
          <button type="button" onClick={() => setQuery('')} className="text-ink-dim hover:text-ink">
            <X size={12} />
          </button>
        )}
      </div>

      <div className="mt-2 flex flex-wrap items-center gap-1">
        {KIND_ORDER.map((k) => (
          <Chip
            key={k}
            label={KIND_META[k].short}
            color={KIND_META[k].color}
            hint={KIND_META[k].desc}
            active={kindFilter.includes(k)}
            onClick={() => toggleKind(k)}
          />
        ))}

        {/*
         * 「系统组件」不是筛选条件而是**显示开关**，所以用竖线隔开，
         * 而且语义相反：点亮表示"把它们也显示出来"。
         *
         * 为什么必须有它：接上计划任务扫描后本机实测 75 项里 41 项是
         * `\Microsoft\Windows\` 下 Windows 自带的维护任务。它们是背景而不是
         * 问题，铺在画布上会把真正要看的那十几项埋掉。
         */}
        <span className="mx-0.5 h-3 w-px shrink-0 bg-line" />
        <Chip
          label="系统组件"
          color={KIND_META.system.color}
          hint="Windows 自带的组件（含系统维护计划任务）。默认不显示——它们不是你能改动的东西，铺在画面上只会盖住真正该看的项。"
          active={!hideSystem}
          onClick={toggleSystem}
        />
      </div>

      <div className="mt-1.5 flex flex-wrap items-center gap-1">
        {RISK_ORDER.map((r: RiskLevel) => (
          <Chip
            key={r}
            label={RISK_META[r].label}
            color={RISK_META[r].color}
            hint={RISK_META[r].desc}
            active={riskFilter.includes(r)}
            onClick={() => toggleRisk(r)}
          />
        ))}

        <div className="flex-1" />

        {/*
         * 排序开关。两个选项回答**两个不同的问题**：
         * 「按开机顺序」= 谁先跑起来；「按资源占用」= 谁最费资源。
         * 刻意不合成一个"综合分"——那种分数一旦排出来就没人说得清
         * 某一项为什么在前面，而用户要的恰恰是"能不能自己复核"。
         *
         * 第二档只在**拿到了 Windows 自记的影响数据**时才有意义（需提权），
         * 所以两个选项共用同一套禁用与提示逻辑，而不是藏起那个按钮——
         * 藏起来用户就永远不知道有这个能力。
         */}
        <div className="flex items-center gap-0.5 rounded border border-line bg-elevated p-[1px]">
          {SORT_OPTIONS.map((o) => (
            <button
              key={o.value}
              type="button"
              onClick={() => setSortBy(o.value)}
              title={o.hint}
              className={[
                'rounded px-1.5 py-[1px] text-2xs transition-colors',
                sortBy === o.value ? 'bg-hover text-ink' : 'text-ink-dim hover:text-ink',
              ].join(' ')}
            >
              {o.label}
            </button>
          ))}
        </div>

        {dirty && (
          <button
            type="button"
            onClick={clearFilters}
            className="flex items-center gap-1 rounded border border-line bg-elevated px-1.5 py-[1px] text-2xs text-ink transition-colors hover:border-accent hover:text-accent"
            title="清除全部筛选条件"
          >
            <X size={11} />
            清除
          </button>
        )}
      </div>

      <div className="mt-1.5 text-2xs text-ink-dim">
        显示 <span className="tnum text-ink-muted">{shown}</span> / {total} 项
        {hideSystem && !kindFilter.includes('system') && (
          // 数字对不上时用户会怀疑是软件漏扫了，必须当场说明原因
          <span className="ml-1 text-ink-faint">· 已隐藏系统组件</span>
        )}
      </div>
    </div>
  )
}

function Chip({
  label,
  color,
  hint,
  active,
  onClick,
}: {
  label: string
  color: string
  hint: string
  active: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={hint}
      className="flex items-center gap-1 rounded border px-1.5 py-[1px] text-2xs transition-colors"
      style={{
        borderColor: active ? color : '#30363d',
        color: active ? color : '#8b949e',
        background: active ? `${color}33` : 'transparent',
        fontWeight: active ? 600 : 400,
      }}
    >
      <Dot color={color} size={5} />
      {label}
    </button>
  )
}
