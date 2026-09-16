import type { UpdateCheck } from '@/types/update'

/**
 * 更新检测的演示数据（仅浏览器开发模式使用）。
 *
 * **三档状态都要有 fixture。** 其中「检查失败」那一档尤其不能省——
 * 它在真机上是最难复现、又最容易被写坏的一个分支：网络不通、公司代理拦掉、
 * 接口限流，都会落到这里。如果只照着"有新版"的样子开发，
 * 很容易把它写成"什么都不显示"，而用户看到的会是一个永远不提示更新的界面。
 *
 * 用地址栏参数切换：
 *   ?update=latest   已是最新
 *   ?update=failed   检查失败
 *   （缺省）          有新版本
 */

const NOTES = `### 这一版做了什么

- **从「只看」变成「能改」**：编排模式里勾选要处理的项，点「应用」先看到一份
  预演（改前 → 改后、会发生什么、风险高低），动手前自动建立快照，随时可一键回滚
- 不可逆的系统关键项（服务栈、Winlogon、explorer、安全中心、驱动）仍然不给碰
- 新增「启动影响」（CPU 时间与磁盘读写量，与任务管理器同源）与
  「开机后第几秒出现」；两者与「启动花了多久」是三个不同的问题，界面分开呈现
- 「每次开机的总时长」可以自己记了（可选，默认关闭）

### 下载哪个

| 文件 | 说明 |
|---|---|
| \`BootFlow_0.2.0_x64-setup.exe\` | 安装版，用户级安装，不需要管理员权限 |
| \`bootflow-portable-x64.zip\` | 便携版，解压直接运行 |

**默认什么都不改**：体检始终是只读的。写操作要先看预演、先建快照、可一键回滚，
且程序不请求提权、不弹 UAC。`

const CHECKED_AT = '2026-09-15T00:40:00Z'

export const MOCK_UPDATE_AVAILABLE: UpdateCheck = {
  status: 'available',
  currentVersion: '0.1.1',
  latestVersion: '0.2.0',
  releaseName: 'BootFlow v0.2.0 — 可写可控',
  releaseUrl: 'https://github.com/TEGCeylonQi/bootflow/releases/tag/v0.2.0',
  publishedAt: '2026-09-15T00:30:00Z',
  notes: NOTES,
  assets: [
    {
      name: 'BootFlow_0.2.0_x64-setup.exe',
      url: 'https://github.com/TEGCeylonQi/bootflow/releases/download/v0.2.0/BootFlow_0.2.0_x64-setup.exe',
      size: 1_648_640,
    },
    {
      name: 'bootflow-portable-x64.zip',
      url: 'https://github.com/TEGCeylonQi/bootflow/releases/download/v0.2.0/bootflow-portable-x64.zip',
      size: 1_907_916,
    },
  ],
  reason: null,
  checkedAt: CHECKED_AT,
}

export const MOCK_UPDATE_LATEST: UpdateCheck = {
  status: 'upToDate',
  currentVersion: '0.1.1',
  latestVersion: '0.1.1',
  releaseName: 'BootFlow v0.1.1 — 更新检测',
  releaseUrl: 'https://github.com/TEGCeylonQi/bootflow/releases/tag/v0.1.1',
  publishedAt: '2026-09-15T00:10:00Z',
  notes: null,
  assets: [],
  reason: null,
  checkedAt: CHECKED_AT,
}

export const MOCK_UPDATE_FAILED: UpdateCheck = {
  status: 'failed',
  currentVersion: '0.1.1',
  latestVersion: null,
  releaseName: null,
  releaseUrl: null,
  publishedAt: null,
  notes: null,
  assets: [],
  reason: '连接 GitHub 超时，检查一下网络或代理设置后重试',
  checkedAt: CHECKED_AT,
}

/** 按地址栏参数挑一份演示数据；缺省给「有新版本」，方便直接看到完整面板 */
export function mockUpdateResult(): UpdateCheck {
  if (typeof window === 'undefined') return MOCK_UPDATE_AVAILABLE

  const want = new URLSearchParams(window.location.search).get('update')
  if (want === 'latest') return MOCK_UPDATE_LATEST
  if (want === 'failed') return MOCK_UPDATE_FAILED
  return MOCK_UPDATE_AVAILABLE
}

/**
 * 浏览器开发模式下的「下载并安装」模拟。
 *
 * 假装下载完成并成功唤起安装向导。用地址栏参数可以注入失败：
 *   ?install=fail   下载失败（模拟网络错误）
 *   ?install=launch 下载成功但唤起安装器失败
 * 这两种失败分支在真机上都不好随手复现，给 mock 留个口子，
 * 前端的状态机就不会只照着"一路成功"的样子写。
 */
export async function mockInstallUpdate(_url: string): Promise<void> {
  if (typeof window === 'undefined') return

  const want = new URLSearchParams(window.location.search).get('install')
  if (want === 'fail') {
    throw new Error('下载安装包失败（HTTP 504）：网关超时')
  }
  if (want === 'launch') {
    throw new Error('已下载但没能打开安装向导（ShellExecute 返回 5）')
  }
}
