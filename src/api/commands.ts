/**
 * 与 Rust 后端的唯一调用面。
 *
 * 关键设计：浏览器里跑（`npm run dev`）时自动回退到 mock 数据，
 * 这样前端可以完全脱离 Rust 独立开发与预览；一旦运行在 Tauri 容器里
 * (`__TAURI_INTERNALS__` 存在) 则走真实 invoke。
 */
import type {
  BootRecord,
  BootTimeline,
  OsInfo,
  ScanResult,
  SourceKind,
  StartupItem,
} from '@/types/model'
import type { UpdateCheck } from '@/types/update'
import type {
  ApplyOutcome,
  BootPerformanceDiagnosis,
  DryRunOutcome,
  EditInput,
  ExportBundle,
  PlannedAction,
  RollbackOutcome,
  SnapshotSummary,
} from '@/types/snapshot'

/** 检测是否运行在 Tauri 容器内 */
export const isTauri = (): boolean =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

async function invokeSafe<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<T>(cmd, args)
}

/** 模拟扫描延迟，让加载动效可见 */
const delay = (ms: number) => new Promise((r) => setTimeout(r, ms))

/**
 * 开发用开关：地址后加 `?timeline=denied`，切到「读不到开机性能日志」那份数据。
 *
 * 有这个开关，是因为那条分支不是臆想出来的边界情况——**本机非提权运行就是这个结果**。
 * 系统日志通道的权限列表里没有普通用户，读取直接被拒。如果只凭"有数据"的那份
 * mock 开发，很容易把读不到写成"显示一张空图"，而空图在用户眼里等于
 * "你的开机不花时间"。让这条分支随时可复现，比事后靠想象力补界面可靠得多。
 */
function mockTimelineDenied(): boolean {
  if (typeof window === 'undefined') return false
  return new URLSearchParams(window.location.search).get('timeline') === 'denied'
}

export async function scanAll(): Promise<ScanResult> {
  if (!isTauri()) {
    const mod = await import('@/mock/mockData')
    await delay(900)
    const data = mockTimelineDenied() ? mod.MOCK_SCAN_RESULT_TIMELINE_DENIED : mod.MOCK_SCAN_RESULT
    return structuredClone(data)
  }
  return invokeSafe<ScanResult>('scan_all')
}

export async function scanSource(source: SourceKind): Promise<StartupItem[]> {
  if (!isTauri()) {
    const { MOCK_ITEMS } = await import('@/mock/mockData')
    await delay(300)
    return MOCK_ITEMS.filter((i) => i.source === source)
  }
  return invokeSafe<StartupItem[]>('scan_source', { source })
}

export async function getBootTimeline(): Promise<BootTimeline> {
  if (!isTauri()) {
    const mod = await import('@/mock/mockData')
    const data = mockTimelineDenied() ? mod.MOCK_SCAN_RESULT_TIMELINE_DENIED : mod.MOCK_SCAN_RESULT
    return structuredClone(data.bootTimeline)
  }
  return invokeSafe<BootTimeline>('get_boot_timeline')
}

/**
 * 「开机自记账」记录列表（旧 → 新）。
 *
 * 与 `getBootTimeline` 是**两条独立通路**：那条读系统事件日志（要提权、还只在
 * 慢启动时才有数据），这条读 BootFlow 自己在 `%LOCALAPPDATA%` 下写的文件，
 * 普通权限就能读到，且每次开机必有一条。
 */
export async function getBootRecords(): Promise<BootRecord[]> {
  if (!isTauri()) {
    const { MOCK_BOOT_RECORDS } = await import('@/mock/mockData')
    return structuredClone(MOCK_BOOT_RECORDS)
  }
  return invokeSafe<BootRecord[]>('get_boot_records')
}

export async function getOsInfo(): Promise<OsInfo> {
  if (!isTauri()) {
    const { MOCK_SCAN_RESULT } = await import('@/mock/mockData')
    return MOCK_SCAN_RESULT.os
  }
  return invokeSafe<OsInfo>('get_os_info')
}

export async function checkElevation(): Promise<boolean> {
  if (!isTauri()) return false
  return invokeSafe<boolean>('check_elevation')
}

/** 触发 UAC 重新以管理员身份拉起自身（参数会透传） */
export async function requestElevation(): Promise<void> {
  if (!isTauri()) {
    console.warn('[BootFlow] 浏览器环境无法提权，此调用被忽略')
    return
  }
  return invokeSafe<void>('request_elevation')
}

/** 取图标的请求。后端在扫描阶段不提取图标，所以要把路径一起回传。 */
export interface IconRequest {
  id: string
  path: string
  /** 像素尺寸，缺省由后端决定（32） */
  size?: number
}

/** 后进：批量取图标。返回 id -> base64 PNG 的映射（取不到的项不会出现在结果里） */
export async function getIcons(requests: IconRequest[]): Promise<Record<string, string>> {
  // 浏览器模式没有 Shell，取不到真实图标——直接返回空，
  // 由 AppIcon 的兜底色块顶上，不去伪造图标
  if (!isTauri() || requests.length === 0) return {}
  return invokeSafe<Record<string, string>>('get_icons', { requests })
}

/**
 * 检查有没有新版本。
 *
 * 后端的 `check_update` 设计上不会抛错——「连不上」「被限流」都会变成
 * `status: 'failed'` 的正常返回，由界面如实呈现。所以这里也不需要 try/catch，
 * **绝不能把失败悄悄咽掉**，那就等于谎报「已是最新」。
 *
 * 浏览器开发模式下回退到 mock，用 `?update=latest` / `?update=failed` 切换状态。
 */
export async function checkUpdate(): Promise<UpdateCheck> {
  if (!isTauri()) {
    const { mockUpdateResult } = await import('@/mock/mockUpdate')
    await delay(700)
    return mockUpdateResult()
  }
  return invokeSafe<UpdateCheck>('check_update')
}

/**
 * 用系统默认浏览器打开发布页。
 *
 * 地址只会来自后端返回的 `releaseUrl`；后端 `open_release_page` 还会再校验一次
 * 域名是否属于 GitHub。前端不做判断，也不允许传任意地址。
 */
export async function openReleasePage(url: string): Promise<void> {
  if (!isTauri()) {
    window.open(url, '_blank', 'noopener,noreferrer')
    return
  }
  return invokeSafe<void>('open_release_page', { url })
}

/**
 * 在事件查看器里打开「开机诊断」通道，让用户亲眼看到 Windows 自己的记录。
 *
 * 当程序因权限不足读不到开机耗时、或系统还没有记录时，前端提供这个入口：
 * 一键把用户带到 Windows 事件查看器的对应日志，而不是让他在系统里瞎翻。
 *
 * 浏览器开发模式下是空操作（没有事件查看器可开），由 UI 决定如何降级。
 */
export async function openBootLog(): Promise<void> {
  if (!isTauri()) return
  return invokeSafe<void>('open_boot_log')
}

/**
 * 下载并拉起新版本的安装包（在检查更新的结果里点「下载并安装」时调用）。
 *
 * 后端会先做两层校验（域名白名单 + 本仓库 Releases 前缀），再下载到
 * `%LOCALAPPDATA%\BootFlow\update-cache`，用系统默认方式打开安装向导，
 * 随即清理缓存文件。所以这个调用**成功返回时，安装向导已经在屏幕上**。
 *
 * `onProgress` 会在下载过程中被反复调用，参数为 (已下载字节, 总大小|undefined)。
 * 浏览器开发模式下用 mock 模拟一段带进度的下载。
 */
export async function installUpdate(
  url: string,
  onProgress?: (downloaded: number, total?: number) => void,
): Promise<void> {
  if (!isTauri()) {
    const { mockInstallUpdate } = await import('@/mock/mockUpdate')
    if (onProgress) {
      // mock 一段带进度的下载：分 5 步推进，每步 200ms
      for (let i = 1; i <= 5; i++) {
        await delay(200)
        onProgress(Math.round((i / 5) * 1_648_640), 1_648_640)
      }
    } else {
      await delay(1400)
    }
    return mockInstallUpdate(url)
  }

  const { Channel } = await import('@tauri-apps/api/core')
  const channel = new Channel<{ downloaded: number; total?: number }>()
  channel.onmessage = (msg) => {
    onProgress?.(msg.downloaded, msg.total)
  }
  await invokeSafe<void>('install_update', { url, onProgress: channel })
}

/**
 * 探测「系统是否允许记录开机性能」（只读，不写任何东西）。
 *
 * 与事件日志里**有没有**记录是两回事：
 * 允许记录 ≠ 每次开机都会写 Event 100 —— 系统很可能只在开机偏慢时写。
 * 前端用返回值决定「一键开启」按钮的文案与可用性。
 */
export interface BootRecordStatus {
  /** allowed / disabled / unreadable */
  state: 'allowed' | 'disabled' | 'unreadable'
  message: string
}

export async function probeBootRecord(): Promise<BootRecordStatus> {
  if (!isTauri()) return { state: 'allowed', message: '' }
  return invokeSafe<BootRecordStatus>('probe_boot_record')
}

/**
 * 「一键开启每次开机记录」。
 *
 * 可逆写操作：仅在 Boot 性能诊断被策略显式禁用时才写注册表（15 分钟后自动还原）；
 * 已验证允许则什么都不做。需要管理员权限，失败文案已是人话。
 * 浏览器开发模式下直接模拟成功。
 */
export async function enableBootRecord(): Promise<void> {
  if (!isTauri()) return
  return invokeSafe<void>('enable_boot_record')
}

/** 用户设置。目前只有「每次开机自记账」一项。 */
export interface AppSettings {
  schemaVersion: number
  /** 默认 **false**：不注册任何自启条目 */
  bootRecording: boolean
}

/**
 * 读用户设置。
 *
 * 后端读不到文件／文件损坏时返回默认值（自记账关闭），所以这里不会失败——
 * 设置页需要的是一个确定的状态，而不是"读不到所以显示不出来"。
 */
export async function getSettings(): Promise<AppSettings> {
  if (!isTauri()) return { schemaVersion: 1, bootRecording: false }
  return invokeSafe<AppSettings>('get_settings')
}

/**
 * 打开 / 关闭「每次开机自记账」。
 *
 * 这是少数会**往用户系统里放东西**的操作（一个默认关闭的自启条目）。
 * 后端只在系统层面确实改完之后才落盘设置值，所以返回的设置可以直接信任。
 * 失败文案已是人话（如"注册开机自记账任务失败：拒绝访问"），直接展示即可。
 */
export async function setBootRecording(enabled: boolean): Promise<AppSettings> {
  if (!isTauri()) {
    // 浏览器开发模式下不真注册自启，只回一个自洽的状态
    return { schemaVersion: 1, bootRecording: enabled }
  }
  return invokeSafe<AppSettings>('set_boot_recording', { enabled })
}

/**
 * 清理 `update-cache` 里上次下载残留的安装包，返回清理的文件数。
 *
 * 正常情况下安装完成后缓存即被后端清掉；这条命令是给安装向导被取消、
 * 或下载中断等场景兜底的手动入口。
 */
export async function cleanInstallCache(): Promise<number> {
  if (!isTauri()) return 0
  return invokeSafe<number>('clean_install_cache')
}

// ============================================================
// v0.2.0「可写可控」—— 快照 / 预演 / 应用 / 回滚 / 导出
// ============================================================

/**
 * 预演：把若干编辑请求编译成动作清单，**不写系统**。
 *
 * 浏览器开发模式没有后端，用 mock 生成与真实渲染形状一致的结果：
 * 前端可以照常开发预演弹层，接上 Tauri 后自然换真。
 */
export async function dryRunEdits(
  items: StartupItem[],
  edits: EditInput[],
): Promise<DryRunOutcome> {
  if (!isTauri()) {
    await delay(500)
    return mockDryRun(items, edits)
  }
  return invokeSafe<DryRunOutcome>('dry_run_edits', { items, edits })
}

/** 应用一批编辑（单改 / 批改共用）。写前建快照，失败自动回滚。 */
export async function applyEdits(
  items: StartupItem[],
  edits: EditInput[],
): Promise<ApplyOutcome> {
  if (!isTauri()) {
    // mock：模拟成功，返回一个伪快照 id
    await delay(600)
    return {
      snapshotId: `mock-${Date.now().toString(36)}`,
      results: edits.map((e) => ({ itemId: e.itemId, ok: true })),
      rollbackAvailable: true,
    }
  }
  return invokeSafe<ApplyOutcome>('apply_edits', { items, edits })
}

/** 列出全部快照（新→旧）。 */
export async function listSnapshots(): Promise<SnapshotSummary[]> {
  if (!isTauri()) return []
  return invokeSafe<SnapshotSummary[]>('list_snapshots')
}

/** 回滚到某份快照。回滚本身也产生新快照，可再次回滚。 */
export async function rollbackTo(targetId: string): Promise<RollbackOutcome> {
  if (!isTauri()) {
    await delay(500)
    return { restored: 1, skipped: [], newSnapshotId: `mock-rb-${Date.now().toString()}` }
  }
  return invokeSafe<RollbackOutcome>('rollback_to', { targetId })
}

/** 导出某份快照为独立脚本（PowerShell + .reg）。 */
export async function exportSnapshot(targetId: string): Promise<ExportBundle> {
  if (!isTauri()) {
    return { ps1: '# 浏览器模式无真实快照\n', reg: '' }
  }
  return invokeSafe<ExportBundle>('export_snapshot', { targetId })
}

/* ——— 预演 mock（仅浏览器开发模式） ——— */

function mockDryRun(items: StartupItem[], edits: EditInput[]): DryRunOutcome {
  const steps: PlannedAction[] = []
  const denied: string[] = []

  for (const edit of edits) {
    const item = items.find((i) => i.id === edit.itemId)
    if (!item) {
      denied.push(`找不到 id=${edit.itemId} 的启动项（可能已移除，请重新扫描）`)
      continue
    }
    if (item.risk === 'Locked') {
      denied.push(`「${item.displayName ?? item.name}」系统关键组件，不提供修改入口`)
      continue
    }
    const willChange =
      edit.enabled !== undefined && edit.enabled !== item.enabled
    if (willChange) {
      steps.push({
        itemId: item.id,
        displayName: item.displayName ?? item.name,
        field: 'enabled',
        before: String(item.enabled),
        after: String(edit.enabled),
        consequence: edit.enabled
          ? '改为「启用」，开机/登录时运行'
          : '改为「停用」，不再随开机启动（实体保留，可随时恢复）',
        risk: item.risk,
      })
    }
  }

  return { steps, denied, noop: steps.length === 0 && denied.length === 0 }
}

/**
 * 一键诊断「为什么没有开机性能数据」。
 *
 * 后端只读两处：策略开关（允不允许记录）+ 性能日志（有没有真的记录），
 * 组合出可行动结论。任意设备都可用；浏览器模式返回「无后端」占位。
 */
export async function diagnoseBootPerformance(): Promise<BootPerformanceDiagnosis> {
  if (!isTauri()) {
    const mod = await import('@/mock/mockData')
    // 浏览器模式随手引一份 timeline，让「诊断 → 耗时页联动」也能演示：
    // denied 模式下是无数据时间轴，否则是有数据时间轴。
    const data = mockTimelineDenied() ? mod.MOCK_SCAN_RESULT_TIMELINE_DENIED : mod.MOCK_SCAN_RESULT
    return {
      verdict: mockTimelineDenied() ? '没有读取权限' : '正常',
      summary: mockTimelineDenied()
        ? 'Windows 没有向普通账户开放开机性能日志的读取权限。'
        : '系统一直在记录开机性能，最近一次开机主事件：刚刚（共 64 条相关记录）。',
      action: mockTimelineDenied()
        ? '以管理员身份重新打开 BootFlow 后重试。'
        : '无需操作，启动时序图已可用。',
      needsElevation: mockTimelineDenied(),
      recordCount: mockTimelineDenied() ? 0 : 64,
      channel: 'Microsoft-Windows-Diagnostics-Performance/Operational',
      recordSwitch: 'allowed',
      timeline: structuredClone(data.bootTimeline),
    }
  }
  return invokeSafe<BootPerformanceDiagnosis>('diagnose_boot_performance')
}
