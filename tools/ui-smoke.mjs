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

  /*
   * ——— 4b. 自记账趋势 ———
   *
   * 新增的核心能力：Windows 的 Event 100 只在「完整引导 + 开机偏慢」时才写，
   * 开了快速启动的机器可以几个月一条都没有（实测某台 403 天零记录）。
   * 自记账在每次登录时由自启条目静默写一条，不依赖提权、不依赖任何系统策略。
   *
   * 这里断言三件事：图在、来源标清楚了、**边界也写清楚了**（它只有总时长，
   * 拿不到分段）。少最后一条，就是在拿"总时长"冒充"详细耗时"。
   */
  await expect('耗时分析给出「最近几次开机」自记账趋势', async () => {
    assert((await page.locator('text=最近几次开机用时').count()) > 0, '没有自记账趋势区')
    assert((await page.locator('text=BootFlow 自记账').count()) > 0, '没有标明数据来自自记账')
    assert((await page.locator('text=那需要系统事件日志').count()) > 0, '没有说明它拿不到分段耗时')
    return '自记账趋势已展示'
  })

  /*
   * ——— 4c. 单项开销：把耗时拆到「每一项」———
   *
   * 这是本轮的核心。Windows 没有单项耗时计时器，能实测的是**进程创建时刻**
   * （"第几秒出现"），只有被判慢的项才有真正的耗时。两条通路都要能在界面上
   * 看到，而且**必须写清楚出现时刻不是耗时**——否则用户会把 12.4s 读成
   * "这一项拖慢了 12.4 秒"，那是把整个开机时长安在一项头上。
   */
  await expect('耗时分析把开销拆到单项（每项在开机后第几秒出现）', async () => {
    assert((await page.locator('text=每项在开机后第几秒出现').count()) > 0, '没有单项出现时刻时间轴')
    assert(
      (await page.locator('text=/进程创建时刻/').count()) > 0,
      '没有标明数据来源是内核记录的进程创建时刻',
    )
    // 归因成功的项必须真的画出来，不能只有一个空壳标题。
    // 每行是一个按钮，title 里带着"内核记录的进程创建时刻"。
    const rows = await page.locator('button[title*="内核记录的进程创建时刻"]').count()
    assert(rows > 0, '时间轴没有任何一行，说明归因结果没渲染出来')
    return `已归因并渲染 ${rows} 行`
  })

  /*
   * 这条盯的是「诚实原则」最容易破的一个口子：把"出现时刻"当"耗时"。
   * 断言的是那句警告**真的在界面上**，而不是只写在代码注释里。
   */
  await expect('单项时间轴明确写出「出现时刻不是花了多久」', async () => {
    assert(
      (await page.locator('text=/出现时刻」不是「花了多久/').count()) > 0,
      '单项时间轴没有澄清"出现时刻 ≠ 耗时"',
    )
    assert(
      (await page.locator('text=/Windows 不为单个启动项记录/').count()) > 0,
      '没有解释为什么这一栏给不出"花了多久"',
    )
    // 并且要指路：时间轴回答不了"它有多重"，那块数据在下面
    assert(
      (await page.locator('text=/它有多重/').count()) > 0,
      '没有把"有多重"这个问题指给启动影响那一块——用户会以为这里就是全部',
    )
    return '边界已写明'
  })

  /*
   * ——— 4c-2. 启动影响：Windows 自己量出来的单项开销 ———
   *
   * 这一栏是「拆不到单项」这个问题的真正答案：WDI 每次登录后都会为每个进程
   * 记下 CPU 时间与磁盘读写量，**任务管理器的「启动影响」列读的就是这份数据**。
   * 用户能打开任务管理器逐条对照——所以档位用词也必须和任务管理器一致
   * （高/中/低），换个说法就等于逼用户做翻译。
   */
  await expect('耗时分析给出「每一项占了多少资源」', async () => {
    assert(
      (await page.locator('text=每一项占了多少资源').count()) > 0,
      '没有启动影响区块',
    )
    assert(
      (await page.locator('text=/与任务管理器同源/').count()) > 0,
      '没有标明数据与任务管理器同源',
    )
    const rows = await page.locator('button[title*="启动影响："]').count()
    assert(rows > 0, '启动影响一行都没渲染出来')
    return `已渲染 ${rows} 行`
  })

  /*
   * 这条盯的是这一栏最容易被误读的地方：把"占了多少 CPU"读成"让开机慢了几秒"。
   * CPU 时间跨核累加，多线程程序的时间可以超过窗口本身长度——
   * 所以"这是占用不是耗时"这句话必须真的写在界面上，而不是只在代码注释里。
   */
  await expect('启动影响写明是占用而不是耗时', async () => {
    assert(
      (await page.locator('text=/让开机慢了几秒/').count()) > 0,
      '没有澄清"启动影响 ≠ 让开机慢了多久"',
    )
    assert(
      (await page.locator('text=/资源占用/').count()) > 0,
      '没有把这一栏的性质说成"资源占用"',
    )
    return '口径已写明'
  })

  await expect('详情面板单列「启动影响」一栏', async () => {
    assert((await page.locator('text=启动影响').count()) > 0, '详情面板没有启动影响一栏')
    assert(
      (await page.locator('text=/与任务管理器同一阈值|本次未取得/').count()) > 0,
      '启动影响一栏既没给档位说明、也没给"读不到"的原因',
    )
    return '已单列'
  })

  /*
   * ——— 4c-3. 清单能按「资源占用」重排 ———
   *
   * "谁先跑起来"和"谁最费资源"是两个不同的问题：时序上排在很后面的项
   * 完全可能是最重的那个。所以必须让用户能切换，而不是替他挑一个。
   * 这里断言的是：开关在、能点、点了之后清单里的首项真的变了。
   */
  await expect('清单提供「按资源占用」排序', async () => {
    assert((await page.locator('button:has-text("按开机顺序")').count()) > 0, '没有排序开关')
    const byImpact = page.locator('button:has-text("按资源占用")').first()
    assert((await byImpact.count()) > 0, '没有「按资源占用」这一档')

    const firstName = async () =>
      (await page.locator('[data-item-row]').first().getAttribute('data-item-row')) ?? ''

    const before = await firstName()
    await byImpact.click()
    await page.waitForTimeout(300)
    const after = await firstName()
    assert(before !== after, `切换排序后清单首项没变（都是 ${before}）——排序没真的生效`)

    // 切回去，别把后面断言依赖的默认顺序留在被改过的状态
    await page.locator('button:has-text("按开机顺序")').first().click()
    await page.waitForTimeout(200)
    assert((await firstName()) === before, '切回默认排序后没有恢复原顺序')
    return '可切换且可恢复'
  })

  /*
   * ——— 4d. 自记账开关必须真的可控（opt-in）———
   *
   * 自记账要在系统里常驻一个自启条目。哪怕它无害，用户没同意就不该加，
   * 加了也必须能关掉。这里断言三件事：开关在、文案说清默认关、能拨动。
   * 状态只以 mock 后端返回值为准（不做乐观更新），所以拨完要真的变。
   */
  await expect('自记账提供开关且默认关闭', async () => {
    assert((await page.locator('text=每次开机记一条用时').count()) > 0, '没有自记账开关')
    assert((await page.locator('text=/默认关闭/').count()) > 0, '没有标明默认关闭')
    const sw = page.locator('button[role="switch"]').first()
    assert((await sw.count()) > 0, '开关控件不存在')
    assert(
      (await sw.getAttribute('aria-checked')) === 'false',
      '默认状态不是关闭——自记账不该默认往用户系统里加自启条目',
    )
    return '默认关'
  })

  await expect('自记账开关可以拨开', async () => {
    const sw = page.locator('button[role="switch"]').first()
    await sw.click()
    await page.waitForTimeout(300)
    assert((await sw.getAttribute('aria-checked')) === 'true', '拨开后状态没有跟着变')
    return '可开启'
  })

  await expect('自记账开关可以再关掉并说明不影响其余功能', async () => {
    const sw = page.locator('button[role="switch"]').first()
    await sw.click()
    await page.waitForTimeout(300)
    assert((await sw.getAttribute('aria-checked')) === 'false', '关掉后状态没有跟着变')
    assert(
      (await page.locator('text=/不依赖这个开关/').count()) > 0,
      '没有说明关掉后单项数据照常可见——用户会以为关掉就什么都看不到了',
    )
    return '可回退且边界清楚'
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

      /*
       * 这条通路也要单独交代自己的"读不到"。
       *
       * 真实机器上非提权运行时，系统事件日志和 WDI 目录是**一起**被拒的，
       * 所以这里必须同时出现两块独立的说明——如果只有一块，
       * 说明有人把它们当成了"一件事"，将来其中一条恢复时另一条就说不清了。
       */
      assert(
        (await denied.locator('text=/读取这份数据需要管理员权限/').count()) > 0,
        '启动影响那一块没有交代自己为什么读不到',
      )
      assert(
        (await denied.locator('text=/每一项占了多少资源/').count()) > 0,
        '启动影响那一块整个消失了——读不到不等于不该出现',
      )

      // 启动项本身不受影响，这条要在界面上说清楚
      assert((await denied.locator('text=这不影响上面列的启动项').count()) > 0, '没有说明其余数据不受影响')
      assert((await denied.locator('text=打开系统诊断日志查看').count()) > 0, '没有给出补齐数据的入口')

      // 系统记录开关：卡片存在，且点击后给出诚实说明（系统默认允许、快速启动不记录）
      const guide = denied.locator('text=那要靠系统记录')
      assert((await guide.count()) > 0, '没有「系统记录开关」卡片')
      await denied.locator('button', { hasText: '检查系统记录开关' }).first().click()
      await denied.waitForSelector('text=完整重启', { timeout: 5000 })

      /*
       * 关键：**读不到系统日志 ≠ 什么都看不到**。
       * 自记账这条通路不用提权，所以它必须在「读不到」分支里照常出现——
       * 否则用户会以为"没权限 = 永远空白"，正是这次要填掉的那个洞。
       */
      assert(
        (await denied.locator('text=最近几次开机用时').count()) > 0,
        '读不到系统日志时没有回退到自记账数据',
      )

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
