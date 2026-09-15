import { useAppStore } from '@/store/useAppStore'
import { usePlanStore } from '@/store/usePlanStore'
import { KIND_META, NAME_SOURCE_LABEL, PHASE_SLOGAN, SOURCE_PLAIN } from '@/constants'
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
 *   第二层（默认折叠）→ 排查与审计所需的一切：路径、命令行、注册位置、证书、原始诊断码
 *
 * 判据很简单：**一个不懂 HKLM 是什么的人，也应该能只看第一层就做出判断。**
 *
 * 编排设置默认展开：用户切到编排模式，就是为了来改这一项。
 * 让他再点一次折叠标题才看到控件，是多此一举。
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
        <Collapsible title="技术详情" hint="路径与参数">
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
        <Collapsible title="发布者与签名" hint="核验身份">
          <SignerBlock signer={item.signer} />
        </Collapsible>

        {/* ——— 二级：补充说明（不构成问题，但值得一提）——— */}
        {notes.length > 0 && (
          <Collapsible title="补充说明" hint={`${notes.length} 条`}>
            <DiagnosticsBlock items={notes} />
          </Collapsible>
        )}

        {/* ——— 编排：可改的部分（体检模式下只读展示现状）——— */}
        <Collapsible
          title="编排设置"
          hint={inPlan ? '已编排' : mode === 'orchestrate' ? '可调整' : 'v1.5 起开放'}
          defaultOpen={mode === 'orchestrate'}
        >
          <OrchestrationBlock item={item} />
        </Collapsible>

        {/* ——— 编排：尚未开放的部分，提前告知边界 ——— */}
        <Collapsible title="后续能力" hint="v2.0">
          <FutureFields desired={item.desired} />
        </Collapsible>
      </div>
    </aside>
  )
}

/** 明确区分实测与估算——不把推算值包装成事实 */
function TimingText({ item }: { item: StartupItem }) {
  const t = item.timing

  if (t.confidence === 'none') {
    return <span className="text-ink-dim">系统未记录</span>
  }

  if (t.confidence === 'measured') {
    return (
      <span className="text-ink-muted">
        实测 <span className="tnum text-ink-muted">{((t.durationMs ?? 0) / 1000).toFixed(2)}s</span>
        {t.sourceEventId && <span className="text-ink-dim"> （事件 {t.sourceEventId}）</span>}
      </span>
    )
  }

  return (
    <span className="text-ink-dim">
      推算 ≈ <span className="tnum">{((t.startEstimateMs ?? 0) / 1000).toFixed(1)}s</span> 起
      <span className="ml-1">（非实测）</span>
    </span>
  )
}
