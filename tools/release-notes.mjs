#!/usr/bin/env node
/**
 * 把 RELEASE_NOTES.md 里当前版本的那一段提取出来，作为 Release 正文的
 * 「本版本更新」段，拼在通用介绍的后面，然后调用 `gh release edit` 更新发布页。
 *
 * 用法（发布流水线里调用）：
 *   node tools/release-notes.mjs <tag>          # tag 形如 v0.1.4
 *   node tools/release-notes.mjs <tag> --dry-run # 只打印正文，不碰 Release
 *
 * `--dry-run` 存在的理由：正文里有大半是「通用说明」这种每版都一样的段落，
 * 最容易悄悄过时——本项目的通用段在 v0.2.0 之前一直写着「这是只读体检版，
 * 不会修改系统任何一项配置」，而那个版本恰恰第一次能改系统。发布页是对外的，
 * 自相矛盾的文案会直接伤信任。**tag 还没打时就能本地预览一遍正文**，
 * 是让这类过时文案在发版前就被看见的唯一办法。
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
const ARGS = process.argv.slice(2)
const DRY_RUN = ARGS.includes('--dry-run')
const TAG = ARGS.find((a) => !a.startsWith('--'))

if (!TAG || !/^v\d+\.\d+\.\d+/.test(TAG)) {
  console.error('用法：node tools/release-notes.mjs <v0.1.4> [--dry-run]')
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

// ── 2. 通用说明（与 README 的口径保持一致）────────────────────
//
// ⚠️ 这一段描述的是**当前版本的能力与权限行为**，不是历史。发新版本时
// 必须回头核对一遍——它曾经长期写着「这是只读体检版，不会修改系统任何一项配置」
// 和「以可用最高权限运行」，而应用早已能改系统、权限也早已回退成 asInvoker。
// **旧文案会让发布页一边说「不会改你的系统」、一边在更新日志里写
// 「第一次可以真正停用启动项」，自相矛盾。**
//
// 篇幅上刻意收得很紧：发布页要让人一眼看完"这是什么、下载哪个、有什么坑"，
// 详细说明留给 README 和程序内的「数据口径与来源」。
const generic = `## 这是什么

BootFlow 把 Windows 上散落在多处的开机自启机制统一收拢成一份清单，让你看清
「到底是什么拖慢了我的开机」，并把不需要的项关掉——**预演在前、快照在后、随时可回滚**。

- 扫出五类启动来源：启动文件夹、注册表 Run / RunOnce（含 32 位视图）、
  带开机 / 登录触发器的计划任务、开机自启服务，以及 AppInit_DLLs、IFEO 这类系统级注入
- 每项给出风险分级与一句话处置理由，并识别失效残留（程序已卸载、路径不存在）与重复入口
- 开机开销分三条通路陈述：各阶段耗时、每项在开机后第几秒出现、
  每项的资源占用（与任务管理器「启动影响」同源同阈值），**不合成一个数字**
- 可逆的启用 / 停用：改动前自动建立快照，可一键回滚，快照还能导出成
  不依赖本程序的 \`.reg\` / \`.ps1\`
- 导出报告（JSON / CSV / Markdown）

**默认什么都不改**：扫描与体检全程只读。写操作必须先看一眼预演、动手前自动建快照；
服务栈、Winlogon、explorer、安全中心、驱动这类系统关键项在任何版本都不提供写入口，
服务启动类型也只允许「自动 ↔ 手动」，不提供无法回头的「禁用」。

程序以**当前用户权限**运行（manifest 为 asInvoker），**不请求提权、不弹 UAC**，
用户级安装也不要求管理员。代价是少数数据读不到：分段耗时与「启动影响」所在的
事件通道 ACL 只给了管理员。读不到时界面如实说明并给出「以管理员身份重开」入口，
不会用空数据充数。

## 下载哪个

| 文件 | 说明 |
|---|---|
| \`BootFlow_${VERSION}_x64-setup.exe\` | 安装版，**用户级安装、无需管理员权限**，双击即装 |
| \`bootflow-portable-x64.zip\` | 便携版，解压直接运行，不写注册表 |

两者权限行为完全一致，都需要 [WebView2 运行时](https://developer.microsoft.com/microsoft-edge/webview2/)，
Windows 11 与较新的 Windows 10 已内置。

## 已知限制

- 仅在 Windows 11 上实测；Windows 10 1809+ 支持但未实测，Server / ARM64 未验证
- 尚未覆盖 BHO、UWP StartupTask、Winlogon、驱动类自启
- **写能力（停用 / 启用、回滚）尚未在真机上走完一次完整的「写入 → 回滚」闭环**，
  相关测试为纯逻辑、不触真实注册表。界面与命令都已接通，但这是第一个能改用户系统的版本，
  **首次使用建议先在虚拟机里试一遍**；验证完成后这一条会移除
- 系统只在完整引导且开机偏慢时才写性能记录，所以「没有分段耗时数据」是常态，
  不等于开机不花时间

详见 [README](https://github.com/TEGCeylonQi/bootflow#readme)。`

const body =
  `# BootFlow v${VERSION}\n\n## 本版本更新\n\n${section}\n\n---\n\n${generic}\n`

// ── 3. 写临时文件 + gh release edit ──────────────────────────
if (DRY_RUN) {
  // 只看不写：发布页是对外的，正文该在打 tag 之前就能被核对。
  console.log(body)
  console.log(`[dry-run] 以上是 Release ${TAG} 将要写入的正文，未调用 gh。`)
  process.exit(0)
}

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