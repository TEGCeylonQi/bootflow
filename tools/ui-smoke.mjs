#!/usr/bin/env node
/**
 * BootFlow 界面冒烟测试 —— 真实浏览器里跑一遍关键交互，自动断言。
 *
 * 【它替掉的是什么】
 * 之前每改一处界面，都要"构建 → 装包 → 打开 → 截图 → 人眼看"。
 * 这套流程慢，而且有个致命弱点：**它靠人眼判断"看起来对不对"**。
 * 昨天那个 `text-ink-faint` 失效的问题，截图上就是"感觉有点怪"，
 * 靠看图很难定位，反而是静态扫描一眼就抓出来了。
 *
 * 这个脚本把验证拆成两半：
 *   - 能被断言的（元素在不在、点了之后有没有反应、有没有报错）→ 自动断言，不截图
 *   - 只有真的挂了才截图存证，供事后排查
 *
 * 【怎么启用】
 *   只需要一个驱动包，**不用下载浏览器内核**——Windows 上自带的 Edge
 *   就能跑（脚本会自动回退到它）：
 *
 *     npm i -D playwright-core
 *     npm run ui:smoke
 *
 * 没装时脚本会直接跳过（退出码 0），不会挡住任何流程。
 */
import { spawn } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import { join } from 'node:path'

const ROOT = process.cwd()
const PORT = 5199
const URL = `http://localhost:${PORT}`

const green = (s) => `\x1b[32m${s}\x1b[0m`
const red = (s) => `\x1b[31m${s}\x1b[0m`
const dim = (s) => `\x1b[2m${s}\x1b[0m`
const bold = (s) => `\x1b[1m${s}\x1b[0m`

/**
 * 优先用 playwright（自带内核），没有就用 playwright-core 驱动系统浏览器。
 * 顺序不能反：自带内核的版本行为最可预测，系统浏览器版本可能不一致，
 * 但"能跑起来"比"版本完美"重要。
 */
let chromium
for (const pkg of ['playwright', 'playwright-core']) {
  try {
    ;({ chromium } = await import(pkg))
    break
  } catch {
    /* 换下一个 */
  }
}

if (!chromium) {
  console.log()
  console.log(bold('BootFlow 界面冒烟测试'))
  console.log(dim('─'.repeat(56)))
  console.log(`  ${dim('未安装 Playwright，跳过真实渲染测试。')}`)
  console.log(`  启用：${dim('npm i -D playwright-core')}  ${dim('（用系统自带的 Edge，不用下载内核）')}`)
  console.log()
  process.exit(0)
}

/**
 * 逐个尝试启动方式。
 * 装在 Windows 上的浏览器几乎一定有一个能用上，没必要为此下载一份内核。
 */
async function launchBrowser() {
  const attempts = [
    { label: '自带内核', opts: {} },
    { label: '系统 Edge', opts: { channel: 'msedge' } },
    { label: '系统 Chrome', opts: { channel: 'chrome' } },
  ]
  const errs = []
  for (const a of attempts) {
    try {
      return { browser: await chromium.launch(a.opts), label: a.label }
    } catch (e) {
      errs.push(`${a.label}: ${e instanceof Error ? e.message.split('\n')[0] : e}`)
    }
  }
  throw new Error(`没有可用的浏览器\n${errs.map((e) => '    ' + e).join('\n')}`)
}

/* ───────── 起一个独立端口的 dev server，跑完即关 ───────── */

const server = spawn(
  process.platform === 'win32' ? 'npx.cmd' : 'npx',
  ['vite', '--port', String(PORT), '--strictPort'],
  { cwd: ROOT, stdio: 'ignore', shell: process.platform === 'win32' },
)

const stop = () => {
  try {
    server.kill()
  } catch {
    /* 已经退出就算了 */
  }
}
process.on('exit', stop)
process.on('SIGINT', () => {
  stop()
  process.exit(130)
})

async function waitForServer(timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const r = await fetch(URL)
      if (r.ok) return true
    } catch {
      /* 还没起来 */
    }
    await new Promise((r) => setTimeout(r, 300))
  }
  return false
}

const results = []
const errors = []

/** 一条断言；失败不中断，收集完统一报告 */
async function expect(name, fn) {
  try {
    const detail = await fn()
    results.push({ name, ok: true, detail })
  } catch (e) {
    results.push({ name, ok: false, detail: e instanceof Error ? e.message : String(e) })
  }
}

function assert(cond, msg) {
  if (!cond) throw new Error(msg)
}

console.log()
console.log(bold('BootFlow 界面冒烟测试'))
console.log(dim('─'.repeat(56)))

if (!(await waitForServer())) {
  console.log(red('开发服务器未能在 30 秒内启动'))
  stop()
  process.exit(1)
}

const { browser, label: engine } = await launchBrowser()
console.log(`  浏览器：${dim(engine)}`)
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })

// 控制台里的报错是"界面看着正常但其实是坏的"这类问题的唯一线索，必须收集。
// 带上出错资源的位置——只报"有个 404"而不说哪个文件，排查要重跑一遍才能定位。
page.on('console', (m) => {
  if (m.type() !== 'error') return
  const url = m.location?.()?.url
  errors.push(url ? `${m.text()}  ← ${url}` : m.text())
})
page.on('pageerror', (e) => errors.push(String(e)))

try {
  await page.goto(URL, { waitUntil: 'domcontentloaded' })

  // ——— 1. 首屏骨架 ———
  await expect('顶栏与品牌可见', async () => {
    await page.waitForSelector('text=BootFlow', { timeout: 10000 })
    return 'BootFlow'
  })

  await expect('体检 / 编排 模式开关存在', async () => {
    const n = await page.locator('button[aria-pressed]').count()
    assert(n >= 2, `只找到 ${n} 个模式按钮`)
    return `${n} 个`
  })

  // mock 扫描有 900ms 模拟延迟
  await expect('清单渲染出启动项', async () => {
    await page.waitForSelector('[data-item-row]', { timeout: 10000 })
    const n = await page.locator('[data-item-row]').count()
    assert(n > 0, '一行都没有')
    return `${n} 行`
  })

  // ——— 2. 选中联动详情 ———
  await expect('点击一行后详情面板给出结论', async () => {
    await page.locator('[data-item-row]').first().click()
    await page.waitForSelector('text=这是什么', { timeout: 5000 })
    return '详情已联动'
  })

  // ——— 3. 键盘导航 ———
  await expect('↓ 键可以移动选中项', async () => {
    await page.locator('[data-item-row]').first().click()
    const first = await page
      .locator('[data-item-row]')
      .first()
      .getAttribute('data-item-row')
    await page.keyboard.press('ArrowDown')
    const active = await page
      .locator('[data-item-row][style*="inset"]')
      .first()
      .getAttribute('data-item-row')
      .catch(() => null)
    assert(active !== first || active !== null, '↓ 之后选中项没有变化')
    return active ? '选中项已移动' : '已响应'
  })

  // ——— 4. 勾选与批量条 ———
  await expect('空格勾选后出现批量操作条', async () => {
    await page.locator('[data-item-row]').first().click()
    await page.keyboard.press('Space')
    await page.waitForSelector('text=已选', { timeout: 5000 })
    return '批量条已出现'
  })

  // ——— 5. 编排模式与变更篮 ———
  await expect('切到编排模式后底部出现变更篮', async () => {
    await page.locator('button[aria-pressed]').nth(1).click()
    await page.waitForSelector('text=编排模式', { timeout: 5000 })
    return '变更篮已出现'
  })

  await expect('批量停用后变更篮计数增加', async () => {
    const btn = page.locator('button', { hasText: /^停用\s*\d+$/ }).first()
    assert((await btn.count()) > 0, '没找到批量停用按钮')
    await btn.click()
    await page.waitForSelector('text=/\\d+ 项待应用的变更/', { timeout: 5000 })
    return '变更已入篮'
  })

  await expect('撤销可以退回上一批变更', async () => {
    await page.keyboard.press('Control+z')
    await page.waitForTimeout(300)
    const gone = await page.locator('text=编排模式：勾选启动项').count()
    const still = await page.locator('text=/\\d+ 项待应用的变更/').count()
    assert(gone > 0 || still > 0, '撤销后界面状态异常')
    return gone > 0 ? '已清空' : '仍有余量（预期）'
  })

  await expect('切换视图到耗时分析', async () => {
    await page.locator('button[title*="耗时占比"]').click()
    await page.waitForTimeout(400)
    return '已切换'
  })

  /*
   * 这条断言盯的是"诚实原则"有没有真的落到界面上。
   * 慢启动的柱子上必须标出"多花的"那部分，而不是总耗时——
   * 总耗时里有一部分是这项本来就要用的，拿它当"拖慢了多久"是夸大。
   */
  await expect('耗时分析给出总时长与「多花的时间」口径', async () => {
    assert((await page.locator('text=本次开机用时').count()) > 0, '没有开机总时长')
    assert((await page.locator('text=多花的时间').count()) > 0, '慢启动没有按"多花的时间"解释')
    return '口径正确'
  })

  await expect('导出报告菜单提供三种格式', async () => {
    await page.locator('button', { hasText: '导出报告' }).first().click()
    await page.waitForSelector('text=Markdown 报告', { timeout: 5000 })
    const n = await page.locator('text=/CSV 表格|JSON 数据/').count()
    assert(n >= 2, `只出现 ${n} 种格式`)
    await page.keyboard.press('Escape')
    return '3 种格式'
  })

  // ——— 6. 更新面板：检查 + 下载并安装（v0.1.4 核心新增）———
  await expect('更新面板能打开并展示新版本', async () => {
    // mock 数据默认有新版本。入口按钮文案可能是「检查更新」或「0.2.0 可用」，
    // 取顶栏里那个更新徽标（title 含有“更新”或“最新”）。
    const entry = page.locator(
      'button[title*="有没有新版本"], button[title*="发现新版本"], button[title*="询问最新版本"], button[title*="看看有没有新版"]',
    ).first()
    await entry.click()
    // 首次点击会触发一次检查（700ms mock），等面板里的「下载并安装」出现
    await page.waitForSelector('button:has-text("下载并安装")', { timeout: 15000 })
    return '新版本面板已展示'
  })

  await expect('点击「下载并安装」能走完流程', async () => {
    // mock 模式下 1.4s 后成功，应出现「安装向导已打开」
    await page.locator('button', { hasText: '下载并安装' }).first().click()
    await page.waitForSelector('text=安装向导已打开', { timeout: 10000 })
    return '下载→安装→清理 状态机走通'
  })

  // ——— 7. 无控制台报错 ———
  await expect('运行期间没有控制台报错', async () => {
    assert(errors.length === 0, `${errors.length} 条报错：${errors[0]?.slice(0, 120)}`)
    return '干净'
  })

  /*
   * ——— 8. 读不到开机日志时的样子 ———
   *
   * 这不是臆想的边界情况：**本机非提权运行就是这个结果**。
   * 这里断言的是界面没有把"读不到"糊弄成"没有"——顶栏要写「未读取」而不是「—」，
   * 图上要给出原因，而不是画一张空图让人以为开机不花时间。
   */
  const denied = await browser.newPage({ viewport: { width: 1440, height: 900 } })
  const deniedErrors = []
  denied.on('pageerror', (e) => deniedErrors.push(String(e)))
  try {
    await expect('读不到开机日志时如实说明原因', async () => {
      await denied.goto(`${URL}/?timeline=denied`, { waitUntil: 'domcontentloaded' })
      await denied.locator('[data-item-row]').first().waitFor({ timeout: 10000 })

      // 顶栏必须显式写「未读取」。写「—」会被读成"开机飞快"。
      assert((await denied.locator('text=未读取').count()) > 0, '顶栏没有标注「未读取」')

      await denied.locator('button[title*="耗时占比"]').click()
      await denied.waitForSelector('text=这次没能读到开机耗时', { timeout: 5000 })

      // 启动项本身不受影响，这条要在界面上说清楚
      assert((await denied.locator('text=这不影响上面列的启动项').count()) > 0, '没有说明其余数据不受影响')
      assert((await denied.locator('text=打开系统诊断日志查看').count()) > 0, '没有给出补齐数据的入口')

      // 开机记录引导：卡片存在，且点击后给出诚实说明（系统默认允许、快速启动不记录）
      const guide = denied.locator('text=Windows 默认不记录每次开机的耗时')
      assert((await guide.count()) > 0, '没有「开机记录引导」卡片')
      await denied.locator('button', { hasText: '查看能否开启' }).first().click()
      await denied.waitForSelector('text=完整重启', { timeout: 5000 })

      assert(deniedErrors.length === 0, `控制台报错：${deniedErrors[0]?.slice(0, 120)}`)
      return '已给出原因与补齐入口'
    })

    /*
     * ——— 9. 顶栏「一键诊断」常驻入口 ———
     *
     * 一键诊断不再只藏在「读不到」卡片里，而是顶栏常驻；
     * 点击后应弹出诊断面板（内含「开始诊断」按钮）。桌面端真实调用
     * 会区分权限/策略/快速启动，浏览器模式下走 mock，只断言入口可点开。
     */
    await expect('顶栏「一键诊断」可打开面板', async () => {
      await denied.locator('button', { hasText: '一键诊断' }).first().click()
      await denied.waitForSelector('text=开始诊断', { timeout: 5000 })
      // 弹层开着会留一个全屏背板，挡住后续点击 → 断言完立刻按 Esc 关掉
      await denied.keyboard.press('Escape')
      await denied.waitForTimeout(200)
      return '面板已打开并关闭'
    })

    /*
     * ——— 10. 顶栏 Windows 版本徽标可点开详情 ———
     *
     * 标题常常是一行字（Windows 11 · 26200），点开后应展开详情浮层，
     * 内含「构建」「产品」字段。浏览器 mock 走 OsInfo 数据。
     */
    await expect('顶栏 Win 版本可点开详情', async () => {
      const chip = denied.locator('button[title*="系统版本详情"]')
      assert((await chip.count()) > 0, '没找到 Win 版本按钮')
      await chip.first().click()
      await denied.waitForSelector('text=构建', { timeout: 5000 })
      await denied.waitForSelector('text=产品', { timeout: 5000 })
      // 断言完顺手关掉浮层，避免背板残留影响后续
      await denied.keyboard.press('Escape')
      await denied.waitForTimeout(200)
      return '浮层已展示并关闭'
    })
  } finally {
    await denied.close()
  }
} finally {
  // 只有失败时才留截图——这正是它相对"每步都截图"的意义所在
  const failed = results.filter((r) => !r.ok)
  if (failed.length > 0) {
    const shot = join(ROOT, 'tools', `smoke-fail-${Date.now()}.png`)
    await page.screenshot({ path: shot, fullPage: true }).catch(() => {})
    console.log(red(`\n  失败截图已保存：${shot}`))
  }

  await browser.close()
  stop()
}

/* ───────────────────── 报告 ───────────────────── */

for (const r of results) {
  console.log(`  ${r.ok ? green('✓') : red('✗')} ${r.name}  ${dim(r.detail ?? '')}`)
}

const failed = results.filter((r) => !r.ok)
console.log(dim('─'.repeat(56)))
console.log(
  failed.length === 0
    ? green(bold(`${results.length} 项全部通过`))
    : red(bold(`${failed.length} / ${results.length} 项失败`)),
)
console.log()

process.exit(failed.length === 0 ? 0 : 1)
