import { useAppStore } from '@/store/useAppStore'
import { usePlanStore } from '@/store/usePlanStore'
import {
  IMPACT_COLOR,
  IMPACT_LABEL,
  KIND_META,
  NAME_SOURCE_LABEL,
  PHASE_SLOGAN,
  SOURCE_PLAIN,
} from '@/constants'
import type { StartupItem } from '@/types/model'
import { AppIcon } from '@/components/common/Icon'
import { Badge } from '@/components/common/Badge'
import { Collapsible } from '@/components/common/Collapsible'
import { Field } from '@/components/common/Field'
import { displayNameOf, hasAlias, resolveKind, summarize } from '@/lib/item'
import { AttentionBlock, splitDiagnostics } from './AttentionBlock'
import { DiagnosticsBlock } from './DiagnosticsBlock'
import { SignerBlock } from './SignerBlock'
import { FutureFields } from './FutureFields'
import { OrchestrationBlock } from './OrchestrationBlock'

/**
 * 右侧属性面板＝**二级界面**。
 *
 * 分层原则：
 *   第一层（默认可见）→ 用户决策所需的一切：这是什么、谁做的、有没有问题
 *   第二层（默认展开）→ 排查与审计所需的一切：路径、命令行、注册位置、证书、原始诊断码
 *
 * 判据很简单：**一个不懂 HKLM 是什么的人，也应该能只看第一层就做出判断。**
 *
 * 二级区块默认展开：多数时候技术详情的「程序位置 / 命令行」正是要核对的内容，
 * 信息密度大于折叠省出的留白，多一次点击都是摩擦。
 */
export function PropertyPanel() {
  const items = useAppStore((s) => s.items)
  const selectedId = useAppStore((s) => s.selectedId)
  const mode = usePlanStore((s) => s.mode)
  /** 订阅 boolean：只有这一项的编排状态变了才重渲染整块面板 */
  const inPlan = usePlanStore((s) => (selectedId ? !!s.entries[selectedId] : false))
  const item = items.find((i) => i.id === selectedId)

  if (!item) {
    return (
      <aside className="flex items-center justify-center bg-panel px-6">
        <p className="text-center text-xs leading-5 text-ink-dim">
          从左侧选一个启动项
          <br />
          这里会用大白话告诉你它是什么、有没有问题
        </p>
      </aside>
    )
  }

  const title = displayNameOf(item)
  const kind = KIND_META[resolveKind(item)]
  const { notes } = splitDiagnostics(item.diagnostics)

  return (
    <aside className="flex min-h-0 flex-col bg-panel">
      {/* ——— 头部：名字、类型、状态 ——— */}
      <header className="shrink-0 border-b border-line px-3 py-3">
        <div className="flex items-start gap-2.5">
          <AppIcon name={title} iconData={item.iconData} size={34} />
          <div className="min-w-0 flex-1">
            <h2 className="selectable break-words text-[13px] font-semibold leading-5 text-ink">
              {title}
            </h2>

            {hasAlias(item) && (
              <p className="mt-0.5 truncate text-2xs leading-4 text-ink-dim" title={item.name}>
                注册名 {item.name}
              </p>
            )}

            <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
              <Badge color={kind.color} title={kind.desc}>
                {kind.label}
              </Badge>
              <Badge color={item.enabled ? '#3fb950' : '#6e7681'}>
                {item.enabled ? '已启用' : '已停用'}
              </Badge>
              {inPlan && (
                <Badge color="#a371f7" title="已放入变更篮，尚未生效">
                  已编排
                </Badge>
              )}
            </div>
          </div>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto scroll-thin">
        {/* ——— 这是什么：一段人话 ——— */}
        <section className="border-b border-line-subtle px-3 py-3">
          <h3 className="mb-1.5 text-mini font-medium tracking-wide text-ink-muted">这是什么</h3>
          <p className="selectable text-mini leading-[18px] text-ink-muted">{summarize(item)}</p>
        </section>

        {/* ——— 有没有问题 ——— */}
        <AttentionBlock item={item} />

        {/* ——— 二级：技术详情 ——— */}
        <Collapsible title="技术详情" hint="路径与参数" defaultOpen>
          <Field label="启动方式" labelWidth="w-16">
            {SOURCE_PLAIN[item.source]}
          </Field>
          <Field label="启动时机" labelWidth="w-16">
            {PHASE_SLOGAN[item.bootPhase]}
          </Field>
          <Field label="影响范围" labelWidth="w-16">
            {item.scope === 'machine' ? '所有用户' : '仅当前用户'}
          </Field>
          <Field label="耗时" labelWidth="w-16">
            <TimingText item={item} />
          </Field>
          <Field label="启动影响" labelWidth="w-16">
            <ImpactText item={item} />
          </Field>
          {item.resolvedPath && (
            <Field label="程序位置" mono labelWidth="w-16">
              {item.resolvedPath}
            </Field>
          )}
          <Field label="运行参数" mono labelWidth="w-16">
            {item.args.length > 0 ? item.args.join(' ') : '（无）'}
          </Field>
          <Field label="命令行" mono labelWidth="w-16">
            {item.command}
          </Field>
          <Field label="注册位置" mono labelWidth="w-16">
            {item.location}
          </Field>
          <Field label="注册名" mono labelWidth="w-16">
            {item.name}
          </Field>
          {item.nameFrom && (
            <Field label="名称来源" labelWidth="w-16">
              {NAME_SOURCE_LABEL[item.nameFrom]}
            </Field>
          )}
        </Collapsible>

        {/* ——— 二级：数字签名 ——— */}
        <Collapsible title="发布者与签名" hint="核验身份" defaultOpen>
          <SignerBlock signer={item.signer} />
        </Collapsible>

        {/* ——— 二级：补充说明（不构成问题，但值得一提）——— */}
        {notes.length > 0 && (
          <Collapsible title="补充说明" hint={`${notes.length} 条`} defaultOpen>
            <DiagnosticsBlock items={notes} />
          </Collapsible>
        )}

        {/* ——— 编排：可改的部分（体检模式下只读展示现状）——— */}
        <Collapsible
          title="编排设置"
          hint={inPlan ? '已编排' : mode === 'orchestrate' ? '可调整' : 'v1.5 起开放'}
          defaultOpen
        >
          <OrchestrationBlock item={item} />
        </Collapsible>

        {/* ——— 编排：尚未开放的部分，提前告知边界 ——— */}
        <Collapsible title="后续能力" hint="v2.0" defaultOpen>
          <FutureFields desired={item.desired} />
        </Collapsible>
      </div>
    </aside>
  )
}

/**
 * 耗时一栏。三档必须说清楚各自是**什么**，而不是只说"准不准"。
 *
 * 最容易混淆的是「实测耗时」与「实测出现时刻」：两者都是硬数据，
 * 但一个说的是"花了多久"，另一个说的是"什么时候开始"。
 * 把后者写成"实测 12.4s"就是彻头彻尾的谎——那个 12.4 秒是时刻，不是时长。
 */
function TimingText({ item }: { item: StartupItem }) {
  const t = item.timing

  if (t.confidence === 'none' && t.observedStartMs === undefined) {
    return <span className="text-ink-dim">系统未记录</span>
  }

  const observed =
    t.observedStartMs !== undefined ? (
      <span className="text-ink-muted">
        开机后 <span className="tnum text-ink-muted">{((t.observedStartMs ?? 0) / 1000).toFixed(1)}s</span> 出现
        <span className="text-ink-dim">（内核记的创建时刻，实测）</span>
      </span>
    ) : null

  const measured =
    t.confidence === 'measured' && t.durationMs !== undefined ? (
      <span className="text-ink-muted">
        系统实测启动耗时{' '}
        <span className="tnum text-ink-muted">{((t.durationMs ?? 0) / 1000).toFixed(2)}s</span>
        {t.sourceEventId && <span className="text-ink-dim"> （事件 {t.sourceEventId}）</span>}
      </span>
    ) : null

  // 两条通路都有：这是最完整的一档，两个数字分别说各自的口径
  if (observed && measured) {
    return (
      <span>
        {observed}
        <br />
        {measured}
      </span>
    )
  }
  if (observed) return observed
  if (measured) return measured

  return (
    <span className="text-ink-dim">
      推算 ≈ <span className="tnum">{((t.startEstimateMs ?? 0) / 1000).toFixed(1)}s</span> 起
      <span className="ml-1">（按开机相位推算，非实测）</span>
    </span>
  )
}

/**
 * 「启动影响」一栏 —— 与「耗时」**分开**，因为它是另一个量。
 *
 * Windows 每次登录后都会量一遍每一项占了多少 CPU 与磁盘，任务管理器
 * 「启动影响」列读的就是这份数据。放在这里是为了让用户能**当场对照**。
 *
 * 三句话必须写全，少一句这一栏就会被误读：
 *   1. 它是**资源占用**，不是"让开机慢了几秒"（CPU 时间跨核累加）；
 *   2. 档位用的是**微软的阈值**，不是我们拍的；
 *   3. 没有数据时要说清是"没在窗口里跑"还是"我们读不到"。
 */
function ImpactText({ item }: { item: StartupItem }) {
  const im = item.timing.impact

  if (!im) {
    return (
      <span className="text-ink-dim">
        本次未取得
        <span className="ml-1">（需管理员权限，或它没在登录窗口内启动）</span>
      </span>
    )
  }

  const fmtMs = (ms: number) => (ms >= 1000 ? `${(ms / 1000).toFixed(2)}s` : `${ms}ms`)
  const fmtBytes = (b: number) =>
    b >= 1_048_576 ? `${(b / 1_048_576).toFixed(1)} MB` : `${Math.round(b / 1024)} KB`

  return (
    <span>
      <span className="font-medium" style={{ color: IMPACT_COLOR[im.level] }}>
        {IMPACT_LABEL[im.level]}
      </span>
      <span className="text-ink-dim">（与任务管理器同一阈值）</span>
      <br />
      <span className="text-ink-muted">
        CPU <span className="tnum text-ink-muted">{fmtMs(im.cpuMs)}</span> · 磁盘{' '}
        <span className="tnum text-ink-muted">{fmtBytes(im.diskBytes)}</span>
        {im.processCount > 1 && (
          <span className="text-ink-dim">（{im.processCount} 个进程合计）</span>
        )}
      </span>
      <br />
      <span className="text-2xs text-ink-dim">
        这是占用，不是耗时——多线程程序的 CPU 时间会超过窗口长度。
      </span>
    </span>
  )
}
