import { Lock } from 'lucide-react'
import type { DesiredState } from '@/types/model'
import { Field } from '@/components/common/Field'

/**
 * 尚未开放的编排能力（预留说明）。
 *
 * 这里**只列还不能改的东西**——启用 / 延迟 / 优先级已经在
 * `OrchestrationBlock` 里做成了真控件，再在这里列一遍只会让人以为没生效。
 *
 * 保留这几个字段是为了让用户提前知道产品走向，同时明确标注"当前不可用"。
 * 说清楚边界比假装完整重要：用户看到灰字段的第一反应是"功能没做完"，
 * 所以要有一句话解释它是什么、为什么还没有。
 */
export function FutureFields({ desired }: { desired: DesiredState }) {
  const rows: [string, string, string][] = [
    ['IO 优先级', desired.ioPriority === undefined ? '未设置' : String(desired.ioPriority), '磁盘读写繁忙时让谁先走'],
    ['就绪探针', desired.probe ?? '未设置', '等某个信号出现再启动（例如等网络可用）'],
    [
      '依赖项',
      desired.dependsOn?.length ? `${desired.dependsOn.length} 个上游` : '未设置',
      '等另一个程序先就绪，再做自己的事',
    ],
  ]

  return (
    <div className="space-y-1.5">
      <div className="flex items-start gap-1.5 rounded bg-hover/50 px-2 py-1.5">
        <Lock size={11} className="mt-[3px] shrink-0 text-ink-dim" />
        <p className="text-2xs leading-4 text-ink-dim">
          这几项比"启动顺序"更难做对：判断错了会让程序迟迟不启动，或者干脆起不来。
          <br />
          所以排在后面单独打磨，计划在 v2.0 开放，届时每一项都可单独回退。
        </p>
      </div>

      <div className="select-none">
        {rows.map(([label, value, why]) => (
          <Field key={label} label={label} labelWidth="w-16">
            <span className="text-ink-dim">{value}</span>
            <span className="ml-2 text-2xs text-ink-faint">{why}</span>
          </Field>
        ))}
      </div>
    </div>
  )
}
