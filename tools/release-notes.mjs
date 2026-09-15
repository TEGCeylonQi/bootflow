#!/usr/bin/env node
/**
 * 把 RELEASE_NOTES.md 里当前版本的那一段提取出来，作为 Release 正文的
 * 「本版本更新」段，拼在通用介绍的后面，然后调用 `gh release edit` 更新发布页。
 *
 * 用法（发布流水线里调用）：
 *   node tools/release-notes.mjs <tag>          # tag 形如 v0.1.4
 *
 * 为什么要这一步：
 *   Release 正文同时包含两类内容——「本版本改了什么」（每版不同）与
 *   「这工具是什么 / 下载哪个 / 已知限制」（每版相同）。前者如果写死在
 *   workflow 里，每次发版都要手改流水线；让它从 RELEASE_NOTES.md 现取，
 *   README 那里只管大版本路线，发布页成为唯一权威的更新日志。
 *
 * 找不到当前版本的段落时直接报错退出（不发版），不会发一个没有日志的版本。
 */
import { readFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..')
const TAG = process.argv[2]

if (!TAG || !/^v\d+\.\d+\.\d+/.test(TAG)) {
  console.error('用法：node tools/release-notes.mjs <v0.1.4>')
  process.exit(2)
}

const VERSION = TAG.replace(/^v/, '')

// ── 1. 从 RELEASE_NOTES.md 提取自己那一节 ────────────────────
const notes = readFileSync(join(ROOT, 'RELEASE_NOTES.md'), 'utf8')

const lines = notes.split('\n')
let start = -1
let end = lines.length
for (let i = 0; i < lines.length; i++) {
  const m = lines[i].match(/^##\s+v(\d+\.\d+\.\d+)/)
  if (m && m[1] === VERSION) {
    start = i
    continue
  }
  if (m && m[1] !== VERSION && start >= 0) {
    end = i
    break
  }
}

if (start < 0) {
  console.error(`RELEASE_NOTES.md 里找不到 「## v${VERSION}」这一节，先补上再发版。`)
  process.exit(1)
}

const section = lines
  .slice(start + 1, end)
  .join('\n')
  .trim()

if (!section) {
  console.error(`「## v${VERSION}」这一节是空的，发布页不该没有更新日志。`)
  process.exit(1)
}

// ── 2. 通用说明（与 src-tauri 里 update.rs 的「下载哪个」保持一致）──
const generic = `## 这是什么

BootFlow 把 Windows 上散落在多处的开机自启机制统一收拢，让你看清
「到底是什么拖慢了我的开机」。

- 扫出四类启动来源：启动文件夹、注册表 Run / RunOnce（含 32 位视图）、
  带开机/登录触发器的计划任务、开机自启服务；同时检测 AppInit_DLLs 与
  IFEO 这类系统级注入
- 拆解开机各阶段耗时，标出拖慢启动的具体项目
- 对每一项做三层风险评级，划出「不该动」的禁改区
- 识别失效启动项（程序被卸载、路径不存在、网络位置不可达）
- 导出报告（JSON / CSV / Markdown）

**这是只读体检版，不会修改系统任何一项配置**；以「可用最高权限」运行，
拒绝 UAC 或未提权时仍以普通权限启动，只有开机时长读不到（界面如实说明）。

## 下载哪个

| 文件 | 说明 |
|---|---|
| \`BootFlow_${VERSION}_x64-setup.exe\` | 安装版，用户级安装，无需管理员权限；运行时自动以可用最高权限启动 |
| \`bootflow-portable-x64.zip\` | 便携版，解压直接运行，不写注册表；权限行为与安装版一致 |

两个都需要 [WebView2 运行时](https://developer.microsoft.com/microsoft-edge/webview2/)，
Windows 11 与较新的 Windows 10 已内置。

## 已知限制

- 仅在 Windows 11 24H2 上验证过；Windows 10 / Server / ARM64 未验证
- 空窗口需要 WebView2；系统过旧可能提示安装
- 尚未覆盖 BHO、UWP StartupTask、Winlogon、驱动类自启

详见 [README](https://github.com/TEGCeylonQi/bootflow#readme)。`

const body =
  `# BootFlow v${VERSION}\n\n## 本版本更新\n\n${section}\n\n---\n\n${generic}\n`

// ── 3. 写临时文件 + gh release edit ──────────────────────────
const os = await import('node:os')
const fs = await import('node:fs')
const path = await import('node:path')

const tmp = path.join(os.tmpdir(), `bootflow-notes-${VERSION}.md`)
fs.writeFileSync(tmp, body, 'utf8')

try {
  execFileSync('gh', ['release', 'edit', TAG, '--notes-file', tmp], {
    stdio: 'inherit',
    // gh 用 GH_TOKEN / GITHUB_TOKEN 环境变量鉴权，workflow 会在步骤级注入
    env: process.env,
  })
  console.log(`[ok] Release ${TAG} 的正文已更新为本版本更新日志。`)
} finally {
  try { fs.unlinkSync(tmp) } catch { /* 清理失败无所谓 */ }
}