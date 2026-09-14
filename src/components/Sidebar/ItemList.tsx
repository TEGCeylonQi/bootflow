import { useState } from 'react'
import { ChevronRight } from 'lucide-react'
import { KIND_META } from '@/constants'
import type { ItemKind, StartupItem } from '@/types/model'
import { Dot } from '@/components/common/Badge'
import { ItemRow } from './ItemRow'

interface Props {
  groups: [ItemKind, StartupItem[]][]
}

export function ItemList({ groups }: Props) {
  return (
    <div className="pb-2">
      {groups.map(([kind, list]) => (
        <Group key={kind} kind={kind} items={list} />
      ))}
    </div>
  )
}

/**
 * 分组标题按「类型」而非「注册位置」组织。
 *
 * 用户需要的是「这台机器开机时会跑哪些应用、哪些服务」，
 * 而不是「HKCU\...\Run 里有什么、HKLM\WOW6432Node 里有什么」。
 * 系统组件默认折叠——它们不是用户能处理的事，展开着只会干扰视线。
 */
function Group({ kind, items }: { kind: ItemKind; items: StartupItem[] }) {
  const meta = KIND_META[kind]
  const [open, setOpen] = useState(kind !== 'system')

  return (
    <section>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        title={meta.desc}
        className="flex w-full items-center gap-1.5 py-1.5 pl-[25px] pr-3 text-left transition-colors hover:bg-hover/40"
      >
        <ChevronRight
          size={12}
          className={['shrink-0 text-ink-dim transition-transform', open ? 'rotate-90' : ''].join(' ')}
        />
        <Dot color={meta.color} size={6} />
        <span className="text-mini font-medium tracking-wide text-ink-muted">{meta.label}</span>
        <span className="tnum text-2xs text-ink-dim">{items.length}</span>
      </button>

      {open && (
        <div>
          {items.map((it) => (
            <ItemRow key={it.id} item={it} />
          ))}
        </div>
      )}
    </section>
  )
}
