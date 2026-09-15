/**
 * Mock 数据 —— 让前端可以在没有 Rust 后端的情况下独立跑起来（浏览器里 `npm run dev`）。
 *
 * 这份数据是**演示例样本**，不是任何一台具体机器的扫描结果。
 * 它刻意覆盖了工具需要处理的全部情形：
 *
 * - 五种来源：服务 / 计划任务 / 启动文件夹 / 注册表 Run / 系统注入
 * - 四种安装位置风格：`Program Files`、`Program Files (x86)`、
 *   `%LOCALAPPDATA%`（展开后）、`%APPDATA%`（展开后）
 * - 三档风险：Locked / High / Medium / Safe
 * - 三档耗时置信度：实测（事件日志有记录）/ 估算（按相位推算）/ 未知
 * - 各类诊断：慢启动、本该自动却被改手动、全局注入、IFEO 覆盖、重复入口、
 *   未签名、启动文件夹时机不可控、不影响开机的计划任务
 * - 两条失效记录：目标文件已被卸载的残留
 *
 * 数据是手工写的，但字段形状与 Rust 侧 `model.rs` 完全一致——
 * 后端有的字段这里都有，后端不给的字段（如 `kind` / `validity`）由 `enrich()` 补，
 * 模拟 `pipeline::finalize()` 跑完之后的样子。
 *
 * 真实运行时这些数据由 `api/commands.ts` 的 invoke 调用产出，本文件仅在浏览器
 * 开发模式下使用（也保留作为界面测试的 fixture）。
 */
import type {
  BootRecord,
  NameSource,
  Recommendation,
  ScanResult,
  SignerInfo,
  StartupItem,
  ValidityStatus,
} from '../types/model'
import { deriveKind } from '../lib/item'

const OS_COMPONENT: SignerInfo = {
  isSigned: true,
  publisher: 'Microsoft Windows',
  isMicrosoft: true,
  certValid: true,
  isOsComponent: true,
}

const MS_SIGNED: SignerInfo = {
  isSigned: true,
  publisher: 'Microsoft Corporation',
  isMicrosoft: true,
  certValid: true,
  isOsComponent: false,
}

const THIRD_SIGNED = (publisher: string): SignerInfo => ({
  isSigned: true,
  publisher,
  isMicrosoft: false,
  certValid: true,
  isOsComponent: false,
})

const UNSIGNED: SignerInfo = {
  isSigned: false,
  isMicrosoft: false,
  isOsComponent: false,
}

/**
 * 后端扫出来的**原始形态**：只包含系统里真实存在的字段。
 * 面向用户的 `kind` / `displayName` / `nameFrom` 一律由文件末尾的 `enrich()` 补全。
 *
 * 这样做的目的是让 mock 与将来 Rust 侧的产出形状保持一致：
 * 后端负责读 exe 版本信息拿到友好名，前端只做兜底派生。
 *
 * 同理，`validity` / `recommendation` 也由 `enrich()` 补齐——
 * 它们在后端是 `pipeline::finalize()` 的产物（有效性检测 → 去重 → 建议），
 * 不是扫描器直接读出来的原始字段。
 */
type RawItem = Omit<
  StartupItem,
  | 'kind'
  | 'displayName'
  | 'nameFrom'
  | 'summary'
  | 'validity'
  | 'validityDetail'
  | 'recommendation'
  | 'duplicateOf'
>

const RAW_ITEMS: RawItem[] = [
  // ————— 服务 —————
  {
    id: 'svc-audiosrv',
    source: 'Service',
    identityKey: 'c:\\windows\\system32\\svchost.exe -k localservicenetworkrestricted',
    name: 'Windows Audio',
    command: 'C:\\Windows\\System32\\svchost.exe -k LocalServiceNetworkRestricted -p',
    resolvedPath: 'C:\\Windows\\System32\\svchost.exe',
    args: ['-k', 'LocalServiceNetworkRestricted', '-p'],
    location: 'HKLM\\SYSTEM\\CurrentControlSet\\Services\\Audiosrv',
    scope: 'machine',
    enabled: true,
    signer: OS_COMPONENT,
    risk: 'Locked',
    riskReasons: ['系统关键服务', '禁用会导致音频完全失效'],
    diagnostics: [],
    bootPhase: 'smss',
    timing: { confidence: 'none' },
    raw: { startType: 2, delayed: false },
    desired: {},
  },
  {
    id: 'svc-clicktorun',
    source: 'Service',
    identityKey: 'c:\\program files\\common files\\microsoft shared\\clicktorun\\officeclicktorun.exe',
    name: 'Microsoft Office Click-to-Run Service',
    command: '"C:\\Program Files\\Common Files\\Microsoft Shared\\ClickToRun\\OfficeClickToRun.exe" /service',
    resolvedPath: 'C:\\Program Files\\Common Files\\Microsoft Shared\\ClickToRun\\OfficeClickToRun.exe',
    args: ['/service'],
    location: 'HKLM\\SYSTEM\\CurrentControlSet\\Services\\ClickToRunSvc',
    scope: 'machine',
    enabled: true,
    signer: MS_SIGNED,
    risk: 'High',
    riskReasons: ['启动类型被改为「手动」（正常应为「自动」）', '被记录为慢启动服务'],
    diagnostics: [
      {
        code: 'MANUAL_BUT_SHOULD_AUTO',
        severity: 'High',
        message:
          '该服务启动类型为「手动」，但 Office 应用启动时依赖它。开机后首次打开 Word/Excel 会卡住等待服务冷启动，是「点几次才打开」的直接原因。',
        evidence: 'Start=3 (DEMAND_START)，EventID=103 记录耗时 8.42s',
      },
      {
        code: 'SLOW_START',
        severity: 'High',
        message: '系统事件日志记录其启动耗时 8.42 秒，超过慢启动阈值。',
        evidence: 'EventID=103, 8420ms',
      },
    ],
    bootPhase: 'userInit',
    timing: { confidence: 'measured', durationMs: 8420, sourceEventId: 103 },
    raw: { startType: 3, delayed: false },
    desired: {},
  },
  {
    id: 'svc-mactype',
    source: 'Service',
    identityKey: 'c:\\program files\\mactype\\mactray.exe -service',
    name: 'MacType',
    command: '"C:\\Program Files\\MacType\\MacTray.exe" -service',
    resolvedPath: 'C:\\Program Files\\MacType\\MacTray.exe',
    args: ['-service'],
    location: 'HKLM\\SYSTEM\\CurrentControlSet\\Services\\MacType',
    scope: 'machine',
    enabled: true,
    signer: THIRD_SIGNED('MacType Project'),
    risk: 'High',
    riskReasons: ['以 LocalSystem 权限运行', '通过 GDI Hook 注入所有 GUI 进程', '被记录为慢启动服务'],
    diagnostics: [
      {
        code: 'GLOBAL_HOOK',
        severity: 'High',
        message:
          '以系统权限对全进程注入字体渲染 Hook。所有 GUI 程序启动时都要重建渲染环境，是开机后一段时间内程序反应迟钝的主要来源之一。建议在排除列表中加入 WINWORD.EXE / EXCEL.EXE / POWERPNT.EXE。',
        evidence: 'ImagePath=MacTray.exe -service，Start=2 (AUTO_START)',
      },
      {
        code: 'SLOW_START',
        severity: 'High',
        message: '系统事件日志记录其启动耗时 3.18 秒。',
        evidence: 'EventID=103, 3180ms',
      },
    ],
    bootPhase: 'userInit',
    timing: { confidence: 'measured', durationMs: 3180, sourceEventId: 103 },
    raw: { startType: 2, delayed: false },
    desired: {},
  },
  {
    id: 'svc-rpcss',
    source: 'Service',
    identityKey: 'c:\\windows\\system32\\svchost.exe -k rpcss',
    name: 'Remote Procedure Call (RPC)',
    command: 'C:\\Windows\\system32\\svchost.exe -k rpcss -p',
    resolvedPath: 'C:\\Windows\\System32\\svchost.exe',
    args: ['-k', 'rpcss', '-p'],
    location: 'HKLM\\SYSTEM\\CurrentControlSet\\Services\\RpcSs',
    scope: 'machine',
    enabled: true,
    signer: OS_COMPONENT,
    risk: 'Locked',
    riskReasons: ['系统关键服务，被大量组件依赖'],
    diagnostics: [],
    bootPhase: 'smss',
    timing: { confidence: 'none' },
    raw: { startType: 2, delayed: false },
    desired: {},
  },
  {
    id: 'svc-windefend',
    source: 'Service',
    identityKey: 'c:\\programdata\\microsoft\\windows defender\\platform\\msmpeng.exe',
    name: 'Microsoft Defender Antivirus Service',
    command: '"C:\\ProgramData\\Microsoft\\Windows Defender\\Platform\\4.18\\MsMpEng.exe"',
    resolvedPath: 'C:\\ProgramData\\Microsoft\\Windows Defender\\Platform\\4.18\\MsMpEng.exe',
    args: [],
    location: 'HKLM\\SYSTEM\\CurrentControlSet\\Services\\WinDefend',
    scope: 'machine',
    enabled: true,
    signer: OS_COMPONENT,
    risk: 'Locked',
    riskReasons: ['安全中心组件'],
    diagnostics: [],
    bootPhase: 'driver',
    timing: { confidence: 'none' },
    raw: { startType: 2, delayed: true },
    desired: {},
  },
  {
    id: 'svc-wsearch',
    source: 'Service',
    identityKey: 'c:\\windows\\system32\\searchindexer.exe -k ime',
    name: 'Windows Search',
    command: 'C:\\Windows\\System32\\SearchIndexer.exe /Embedding',
    resolvedPath: 'C:\\Windows\\System32\\SearchIndexer.exe',
    args: ['/Embedding'],
    location: 'HKLM\\SYSTEM\\CurrentControlSet\\Services\\WSearch',
    scope: 'machine',
    enabled: true,
    signer: OS_COMPONENT,
    risk: 'Medium',
    riskReasons: ['延迟自动启动', '开机后持续建立索引，占用磁盘 IO'],
    diagnostics: [
      {
        code: 'IO_HEAVY',
        severity: 'Medium',
        message: '延迟启动服务。开机后一段时间内持续索引磁盘，与其他启动项争抢 IO。',
        evidence: 'DelayedAutostart=1',
      },
    ],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 25000 },
    raw: { startType: 2, delayed: true },
    desired: {},
  },

  // ————— 计划任务 —————
  {
    id: 'task-office-monitor',
    source: 'ScheduledTask',
    identityKey: 'c:\\program files\\common files\\microsoft shared\\clicktorun\\officec2rclient.exe /watchservice',
    name: 'Office ClickToRun Service Monitor',
    command: '"C:\\Program Files\\Common Files\\Microsoft Shared\\ClickToRun\\OfficeC2RClient.exe" /WatchService',
    resolvedPath: 'C:\\Program Files\\Common Files\\Microsoft Shared\\ClickToRun\\OfficeC2RClient.exe',
    args: ['/WatchService'],
    location: '\\Microsoft\\Office\\Office ClickToRun Service Monitor',
    scope: 'machine',
    enabled: true,
    signer: MS_SIGNED,
    risk: 'Safe',
    riskReasons: [],
    diagnostics: [
      {
        code: 'NO_BOOT_IMPACT',
        severity: 'Safe',
        message: '触发器为每天 04:00（含随机延迟 6 小时），不含开机或登录触发器，不参与开机路径，不产生开机耗时。',
        evidence: 'CalendarTrigger 04:00, RandomDelay PT6H',
      },
    ],
    bootPhase: 'unknown',
    timing: { confidence: 'none' },
    raw: { triggerTypes: ['CalendarTrigger'] },
    desired: {},
  },
  {
    id: 'task-onedrive',
    source: 'ScheduledTask',
    identityKey: 'c:\\program files\\microsoft onedrive\\onedrive.exe',
    name: 'OneDrive Standalone Update Task',
    command: '"C:\\Program Files\\Microsoft OneDrive\\OneDrive.exe" /background',
    resolvedPath: 'C:\\Program Files\\Microsoft OneDrive\\OneDrive.exe',
    args: ['/background'],
    location: '\\Microsoft\\OneDrive\\OneDrive Standalone Update Task-S-1-5-21',
    scope: 'machine',
    enabled: true,
    signer: MS_SIGNED,
    risk: 'Safe',
    riskReasons: [],
    diagnostics: [],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 20000 },
    raw: { triggerTypes: ['LogonTrigger'] },
    desired: {},
  },

  // ————— 启动文件夹 —————
  {
    id: 'folder-icue',
    source: 'StartupFolderUser',
    identityKey: 'c:\\program files\\corsair\\corsair icue software\\icue launcher.exe',
    name: 'Corsair iCUE',
    command: 'C:\\Program Files\\Corsair\\CORSAIR iCUE Software\\iCUE Launcher.exe',
    resolvedPath: 'C:\\Program Files\\Corsair\\CORSAIR iCUE Software\\iCUE Launcher.exe',
    args: [],
    location: '%APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\Corsair iCUE.lnk',
    scope: 'user',
    enabled: true,
    signer: THIRD_SIGNED('Corsair Memory, Inc.'),
    risk: 'Safe',
    riskReasons: [],
    diagnostics: [
      {
        code: 'STARTUP_FOLDER_LOW_CONTROL',
        severity: 'Medium',
        message:
          '通过启动文件夹快捷方式启动（无参数、未提权）。启动时机由 Explorer 决定，可能早于 USB 外设枚举完成，此时读不到设备，灯光与按键配置不会应用。若要保证生效，建议迁移为登录后延迟启动的计划任务。',
        evidence: 'LNK 无 RunAs 标记，无参数',
      },
    ],
    bootPhase: 'shell',
    timing: { confidence: 'estimated', startEstimateMs: 21000 },
    raw: { isShortcut: true, elevated: false },
    desired: {},
  },

  // ————— 注册表 Run（HKCU）—————
  //
  // 下面这一组刻意混用了几种安装位置风格，用来说明扫描器不假设程序装在哪个目录，
  // 也支持把程序装到系统盘之外：
  //   · `Program Files` / `Program Files (x86)` —— 安装器默认位置
  //   · `%LOCALAPPDATA%` / `%APPDATA%` —— 用户级安装（无需管理员）
  //   · 其他盘符（如 `D:\Apps\`）—— 自定义安装位置
  {
    id: 'run-dropbox',
    source: 'RunUser',
    identityKey: 'c:\\users\\user\\appdata\\local\\dropbox\\dropbox.exe /home',
    name: 'Dropbox',
    command: '"C:\\Users\\user\\AppData\\Local\\Dropbox\\Dropbox.exe" /home',
    resolvedPath: 'C:\\Users\\user\\AppData\\Local\\Dropbox\\Dropbox.exe',
    args: ['/home'],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: THIRD_SIGNED('Dropbox, Inc.'),
    risk: 'Safe',
    riskReasons: ['后台常驻同步进程，占用磁盘与网络 IO'],
    diagnostics: [
      {
        code: 'IO_HEAVY',
        severity: 'Medium',
        message: '云盘同步客户端，登录后立即启动并持续扫描本地目录比对差异，与其它启动项争抢 IO。',
      },
    ],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 16800 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-everything',
    source: 'RunUser',
    identityKey: 'd:\\apps\\everything\\everything.exe -startup',
    name: 'Everything',
    command: '"D:\\Apps\\Everything\\Everything.exe" -startup',
    resolvedPath: 'D:\\Apps\\Everything\\Everything.exe',
    args: ['-startup'],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: THIRD_SIGNED('voidtools'),
    risk: 'Safe',
    riskReasons: ['启动时建立 MFT 索引，短时高磁盘占用'],
    diagnostics: [
      {
        code: 'IO_HEAVY',
        severity: 'Medium',
        message: '启动阶段读取 MFT 建立索引，短时间产生较高磁盘队列深度。',
      },
    ],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 17200 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-steam',
    source: 'RunUser',
    identityKey: 'c:\\program files (x86)\\steam\\steam.exe -silent',
    name: 'Steam',
    command: '"C:\\Program Files (x86)\\Steam\\steam.exe" -silent',
    resolvedPath: 'C:\\Program Files (x86)\\Steam\\steam.exe',
    args: ['-silent'],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: THIRD_SIGNED('Valve Corp.'),
    risk: 'Safe',
    riskReasons: ['占用内存与网络，登录后非必需'],
    diagnostics: [],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 19000 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-parsec',
    source: 'RunUser',
    identityKey: 'c:\\program files\\parsec\\parsecd.exe',
    name: 'Parsec',
    command: '"C:\\Program Files\\Parsec\\parsecd.exe"',
    resolvedPath: 'C:\\Program Files\\Parsec\\parsecd.exe',
    args: [],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: THIRD_SIGNED('Parsec Cloud, Inc.'),
    risk: 'Safe',
    riskReasons: [],
    diagnostics: [],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 19400 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-spotify',
    source: 'RunUser',
    identityKey: 'c:\\users\\user\\appdata\\roaming\\spotify\\spotify.exe --autostart --minimized',
    name: 'Spotify',
    command: '"C:\\Users\\user\\AppData\\Roaming\\Spotify\\Spotify.exe" --autostart --minimized',
    resolvedPath: 'C:\\Users\\user\\AppData\\Roaming\\Spotify\\Spotify.exe',
    args: ['--autostart', '--minimized'],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: THIRD_SIGNED('Spotify AB'),
    risk: 'Safe',
    riskReasons: [],
    diagnostics: [],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 19800 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-localsend',
    source: 'RunUser',
    identityKey: 'c:\\users\\user\\appdata\\local\\programs\\localsend\\localsend_app.exe',
    name: 'LocalSend',
    command: '"C:\\Users\\user\\AppData\\Local\\Programs\\LocalSend\\localsend_app.exe"',
    resolvedPath: 'C:\\Users\\user\\AppData\\Local\\Programs\\LocalSend\\localsend_app.exe',
    args: [],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: THIRD_SIGNED('LocalSend'),
    risk: 'Medium',
    riskReasons: ['与另一启动项指向同一程序，重复启动'],
    diagnostics: [
      {
        code: 'DUPLICATE_ENTRY',
        severity: 'Medium',
        message:
          '检测到同一程序存在两个自启入口（LocalSend 与 localsend_app）。重复启动会互相争抢单实例锁，其中一个会静默退出并可能弹出错误提示。建议保留一个。',
        evidence: 'identityKey 归一化后与 run-localsend-2 相同',
      },
    ],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 20200 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-localsend-2',
    source: 'RunUser',
    identityKey: 'c:\\users\\user\\appdata\\local\\programs\\localsend\\localsend_app.exe',
    name: 'localsend_app',
    command: '"C:\\Users\\user\\AppData\\Local\\Programs\\LocalSend\\localsend_app.exe" -minimized',
    resolvedPath: 'C:\\Users\\user\\AppData\\Local\\Programs\\LocalSend\\localsend_app.exe',
    args: ['-minimized'],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: THIRD_SIGNED('LocalSend'),
    risk: 'Medium',
    riskReasons: ['与 LocalSend 为同一程序的重复启动项'],
    diagnostics: [
      {
        code: 'DUPLICATE_ENTRY',
        severity: 'Medium',
        message: '与 LocalSend 指向同一可执行文件，属重复启动项。',
      },
    ],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 20300 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-afterburner',
    source: 'RunUser',
    identityKey: 'c:\\program files (x86)\\msi afterburner\\msiafterburner.exe -silent',
    name: 'MSI Afterburner',
    command: '"C:\\Program Files (x86)\\MSI Afterburner\\MSIAfterburner.exe" -silent',
    resolvedPath: 'C:\\Program Files (x86)\\MSI Afterburner\\MSIAfterburner.exe',
    args: ['-silent'],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: UNSIGNED,
    risk: 'Medium',
    riskReasons: ['未签名', '与 MacType 同为全进程注入类工具，同时启用会叠加开销'],
    diagnostics: [
      {
        code: 'UNSIGNED',
        severity: 'Medium',
        message: '未检测到有效数字签名，无法核验发布者。',
      },
      {
        code: 'CONFLICT',
        severity: 'Medium',
        message:
          '与 MacType 同属全进程注入类工具（一个 Hook 字体渲染，一个向游戏进程注入监控浮层）。两者同时启用会让每个 GUI 进程的启动都多做一轮注入，建议只保留实际需要的那个。',
      },
    ],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 20600 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-slack',
    source: 'RunUser',
    identityKey: 'c:\\users\\user\\appdata\\local\\slack\\slack.exe --startup',
    name: 'Slack',
    command: '"C:\\Users\\user\\AppData\\Local\\slack\\slack.exe" --startup',
    resolvedPath: 'C:\\Users\\user\\AppData\\Local\\slack\\slack.exe',
    args: ['--startup'],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: THIRD_SIGNED('Slack Technologies, LLC'),
    risk: 'Safe',
    riskReasons: [],
    diagnostics: [],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 21000 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-discord',
    source: 'RunUser',
    identityKey: 'c:\\users\\user\\appdata\\local\\discord\\discord.exe --startup',
    name: 'Discord',
    command: '"C:\\Users\\user\\AppData\\Local\\Discord\\Discord.exe" --startup',
    resolvedPath: 'C:\\Users\\user\\AppData\\Local\\Discord\\Discord.exe',
    args: ['--startup'],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: THIRD_SIGNED('Discord Inc.'),
    risk: 'Safe',
    riskReasons: [],
    diagnostics: [],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 21300 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-docker',
    source: 'RunUser',
    identityKey: 'c:\\program files\\docker\\docker\\docker desktop.exe -autostart',
    name: 'Docker Desktop',
    command: '"C:\\Program Files\\Docker\\Docker\\Docker Desktop.exe" -autostart',
    resolvedPath: 'C:\\Program Files\\Docker\\Docker\\Docker Desktop.exe',
    args: ['-autostart'],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: THIRD_SIGNED('Docker Inc.'),
    risk: 'Safe',
    riskReasons: ['启动 WSL 后端，占用较多内存'],
    diagnostics: [],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 21600 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-autohotkey',
    source: 'RunUser',
    identityKey: 'c:\\program files\\autohotkey\\autohotkey.exe',
    name: 'AutoHotkey',
    command: '"C:\\Program Files\\AutoHotkey\\AutoHotkey.exe"',
    resolvedPath: 'C:\\Program Files\\AutoHotkey\\AutoHotkey.exe',
    args: [],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: UNSIGNED,
    risk: 'Medium',
    riskReasons: ['未签名'],
    diagnostics: [
      {
        code: 'UNSIGNED',
        severity: 'Medium',
        message: '未检测到有效数字签名。',
      },
    ],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 21900 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-edge',
    source: 'RunUser',
    identityKey: 'c:\\program files (x86)\\microsoft\\edge\\application\\msedge.exe --no-startup-window',
    name: 'Microsoft Edge',
    command: '"C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe" --no-startup-window',
    resolvedPath: 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
    args: ['--no-startup-window'],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: MS_SIGNED,
    risk: 'Safe',
    riskReasons: [],
    diagnostics: [],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 22200 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'run-zoom',
    source: 'RunUser',
    identityKey: 'c:\\users\\user\\appdata\\roaming\\zoom\\bin\\zoom.exe -silent',
    name: 'Zoom',
    command: '"C:\\Users\\user\\AppData\\Roaming\\Zoom\\bin\\Zoom.exe" -silent',
    resolvedPath: 'C:\\Users\\user\\AppData\\Roaming\\Zoom\\bin\\Zoom.exe',
    args: ['-silent'],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: THIRD_SIGNED('Zoom Video Communications, Inc.'),
    risk: 'Safe',
    riskReasons: [],
    diagnostics: [],
    bootPhase: 'logon',
    timing: { confidence: 'estimated', startEstimateMs: 22500 },
    raw: { hive: 'HKCU', valueType: 'REG_SZ' },
    desired: {},
  },

  // ————— 系统钩子 —————
  {
    id: 'hook-appinit',
    source: 'SystemHook',
    identityKey: 'appinit_dlls',
    name: 'AppInit_DLLs 全局注入',
    command: 'AppInit_DLLs = "C:\\Program Files\\MacType\\MacType64.dll"',
    resolvedPath: 'C:\\Program Files\\MacType\\MacType64.dll',
    args: [],
    location: 'HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Windows',
    scope: 'machine',
    enabled: true,
    signer: THIRD_SIGNED('MacType Project'),
    risk: 'High',
    riskReasons: ['对每个加载 user32.dll 的进程注入 DLL', '显著拖慢所有 GUI 程序启动'],
    diagnostics: [
      {
        code: 'GLOBAL_HOOK',
        severity: 'High',
        message:
          '注册表启用了 AppInit_DLLs 全局注入，任何加载 user32.dll 的进程都会加载该 DLL。这会拖慢系统中每一个 GUI 程序的启动，且是崩溃与兼容性问题的常见来源。',
        evidence: 'AppInit_DLLs 非空，LoadAppInit_DLLs=1',
      },
    ],
    bootPhase: 'unknown',
    timing: { confidence: 'none' },
    raw: { key: 'AppInit_DLLs', loadAppInitDlls: 1 },
    desired: {},
  },
  {
    id: 'hook-ifeo',
    source: 'SystemHook',
    identityKey: 'ifeo:obs64.exe',
    name: 'IFEO 优先级覆盖 · obs64.exe',
    command: 'PerfOptions\\CpuPriorityClass = 3 (High)',
    resolvedPath: '',
    args: [],
    location:
      'HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Image File Execution Options\\obs64.exe\\PerfOptions',
    scope: 'machine',
    enabled: true,
    signer: THIRD_SIGNED('OBS Project'),
    risk: 'Medium',
    riskReasons: ['通过 IFEO 强制指定进程优先级'],
    diagnostics: [
      {
        code: 'IFEO_OVERRIDE',
        severity: 'Medium',
        message:
          '通过 Image File Execution Options 为该程序指定了 CPU 优先级 High 与 IO 优先级 High。该设置对任何启动方式均生效。注意：RealTime(4) 级别会破坏音频栈与输入响应，本工具永久禁止设置该值。',
        evidence: 'CpuPriorityClass=3, IoPriority=3',
      },
    ],
    bootPhase: 'unknown',
    timing: { confidence: 'none' },
    raw: { cpuPriorityClass: 3, ioPriority: 3 },
    desired: {},
  },

  // ————— 卸载残留：无效启动项检测的样本 —————
  // 这类项**不会拖慢开机**（系统找不到文件就直接跳过了），
  // 但会让清单看起来比实际拥挤，也让用户高估自己的自启负担。
  {
    id: 'run-dead-mediastudio',
    source: 'RunUser',
    identityKey: 'c:\\program files (x86)\\mediastudio\\mediastudio.exe',
    name: 'MediaStudio',
    command: '"C:\\Program Files (x86)\\MediaStudio\\mediastudio.exe" -autorun',
    resolvedPath: 'C:\\Program Files (x86)\\MediaStudio\\mediastudio.exe',
    args: ['-autorun'],
    location: 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run',
    scope: 'user',
    enabled: true,
    signer: UNSIGNED,
    risk: 'Safe',
    riskReasons: [],
    diagnostics: [],
    bootPhase: 'logon',
    timing: { confidence: 'none' },
    raw: { valueName: 'MediaStudio', type: 'REG_SZ' },
    desired: {},
  },
  {
    id: 'sf-dead-oldplayer',
    source: 'StartupFolderUser',
    identityKey: 'c:\\program files (x86)\\oldmediaplayer\\player.exe',
    name: 'Player',
    command: '"C:\\Program Files (x86)\\OldMediaPlayer\\player.exe"',
    resolvedPath: 'C:\\Program Files (x86)\\OldMediaPlayer\\player.exe',
    args: [],
    location: '%APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\Player.lnk',
    scope: 'user',
    enabled: true,
    signer: UNSIGNED,
    risk: 'Safe',
    riskReasons: [],
    diagnostics: [],
    bootPhase: 'logon',
    timing: { confidence: 'none' },
    raw: { lnkTarget: 'C:\\Program Files (x86)\\OldMediaPlayer\\player.exe' },
    desired: {},
  },
]

/* ───────────────── 名称统一识别与类型补全 ───────────────── */

/**
 * 名称覆盖表：这些项在系统里注册的是机器名，需要映射成人话。
 *
 * 真实的识别逻辑在后端（读 exe 版本信息的 FileDescription、服务的 DisplayName），
 * 这里手工指定只是为了让 mock 呈现出「后端做完识别之后」的样子。
 */
const NAME_OVERRIDES: Record<string, { displayName: string; nameFrom: NameSource }> = {
  // 名字统一显示为 LocalSend，「重复」这件事交给 validity 维度去表达，
  // 不要再塞进名字里——那样既冗余，又占掉了用户最该看到的那个位置
  'run-localsend-2': { displayName: 'LocalSend', nameFrom: 'fileDescription' },
  // 系统里注册的是 exe 名，用户认的是产品名
  'run-autohotkey': { displayName: 'AutoHotkey', nameFrom: 'fileDescription' },
  'hook-appinit': { displayName: 'MacType 全局字体注入', nameFrom: 'registryValueName' },
  'hook-ifeo': { displayName: 'OBS 启动优先级', nameFrom: 'registryValueName' },
}

/**
 * 有效性与建议覆盖表。
 *
 * 真实情况下这两项由后端 `pipeline::finalize()` 产出：
 * 有效性靠检查目标文件是否存在，去重靠路径归一化，建议靠规则引擎。
 * mock 里手工指定，是为了让界面呈现出「后端跑完分析之后」的样子。
 */
const DEAD_TARGET_REASON =
  '这个程序已经不在电脑上了，它的启动记录是卸载时留下的残留。清理它不会影响任何正在使用的东西（它本来也启动不了）。'

const VALIDITY_OVERRIDES: Record<
  string,
  { validity: ValidityStatus; detail?: string; advice?: Recommendation }
> = {
  'run-dead-mediastudio': {
    validity: 'missingTarget',
    detail:
      '目标文件不存在，程序可能已被卸载：C:\\Program Files (x86)\\MediaStudio\\mediastudio.exe',
    advice: { action: 'remove', reason: DEAD_TARGET_REASON, confidence: 'measured' },
  },
  'sf-dead-oldplayer': {
    validity: 'missingTarget',
    detail:
      '目标文件不存在，程序可能已被卸载：C:\\Program Files (x86)\\OldMediaPlayer\\player.exe',
    advice: { action: 'remove', reason: DEAD_TARGET_REASON, confidence: 'measured' },
  },
  'run-localsend-2': {
    validity: 'duplicate',
    detail: '同一个程序在系统里注册了多个启动入口，此项属冗余',
    advice: {
      action: 'disable',
      reason:
        '同一个程序在系统里注册了不止一个启动入口。保留其中一个就够用了，多出来的不会让它启动得更快，反而容易在你想关掉它时漏掉一个。',
      confidence: 'measured',
    },
  },
}

/** 友好名的默认来源：服务读 DisplayName，任务读描述，其余读 exe 版本信息 */
function defaultNameSource(it: RawItem): NameSource {
  if (it.source === 'Service') return 'serviceDisplayName'
  if (it.source === 'ScheduledTask') return 'taskDescription'
  if (it.source === 'SystemHook') return 'registryValueName'
  return 'fileDescription'
}

function enrich(it: RawItem): StartupItem {
  const override = NAME_OVERRIDES[it.id]
  const assess = VALIDITY_OVERRIDES[it.id]
  return {
    ...it,
    kind: deriveKind(it),
    displayName: override?.displayName ?? it.name,
    nameFrom: override?.nameFrom ?? defaultNameSource(it),
    validity: assess?.validity ?? 'ok',
    validityDetail: assess?.detail,
    recommendation: assess?.advice,
  }
}

export const MOCK_ITEMS: StartupItem[] = RAW_ITEMS.map(enrich)

export const MOCK_SCAN_RESULT: ScanResult = {
  items: MOCK_ITEMS,
  os: { major: 10, minor: 0, build: 22631, sku: 'Windows 11 Pro' },
  elevated: false,
  scannedAt: new Date().toISOString(),
  bootTimeline: {
    totalBootMs: 31800,
    phases: [
      { name: 'kernel', startMs: 0, endMs: 1500 },
      { name: 'driver', startMs: 1500, endMs: 5400 },
      { name: 'devices', startMs: 5400, endMs: 8200 },
      { name: 'smss', startMs: 8200, endMs: 9700 },
      { name: 'userAuth', startMs: 9700, endMs: 12400 },
      { name: 'userInit', startMs: 12400, endMs: 18600 },
      { name: 'shell', startMs: 18600, endMs: 24300 },
      { name: 'logon', startMs: 24300, endMs: 31800 },
    ],
    /**
     * 慢启动记录。三处刻意贴近真实日志的形状：
     *
     * 1. `name` 是**服务名**不是显示名（Event 103 给的 `Name` 就是服务名，
     *    也正因如此才能和启动项列表对上号）。
     * 2. `friendlyName` 常常是空的——系统大多数时候不给。界面必须能接受
     *    只有服务名的情形，不能在这里假设它一定有值。
     * 3. `degradationMs` 才是"多花的"那部分，界面上用它当主指标。
     *    `ClickToRunSvc` 总耗时 8.4s，其中 6.2s 属于退化时间。
     */
    slowServices: [
      {
        name: 'ClickToRunSvc',
        friendlyName: 'Microsoft Office 即点即用服务',
        durationMs: 8420,
        degradationMs: 6200,
        eventId: 103,
      },
      { name: 'MacType', durationMs: 3180, degradationMs: 2100, eventId: 103 },
      { name: 'OneDrive.exe', durationMs: 2640, degradationMs: 1450, eventId: 101 },
    ],
    bootStartedAt: new Date(Date.now() - 3 * 3600_000).toISOString(),
    needsElevation: false,
  },
  errors: ['计划任务扫描：跳过 2 个无法读取定义的任务（拒绝访问）'],
}

/**
 * 读不到开机日志时的样子 —— 用来开发和验证「诚实分支」的界面。
 *
 * 这不是假想的边界情况：Diagnostics-Performance 这个日志通道的访问控制列表里
 * 没有普通用户的条目，**任何一台 Windows 在非管理员权限下运行都是这个结果**，
 * `EvtQuery` 直接返回拒绝访问。
 *
 * 所以必须有办法在浏览器里看到这条分支长什么样，否则容易写成"读不到就显示
 * 一张空图"，而空图在用户眼里等于"你的开机不花时间"——那是撒谎，不是降级。
 *
 * 在开发服务器地址后加 `?timeline=denied` 即可切到这份数据。
 */
export const MOCK_SCAN_RESULT_TIMELINE_DENIED: ScanResult = {
  ...MOCK_SCAN_RESULT,
  bootTimeline: {
    phases: [],
    slowServices: [],
    needsElevation: true,
    unavailableReason:
      '读取开机性能日志需要管理员权限。系统默认没有向普通用户开放这个日志，所以这次没能读到耗时数据。',
  },
}

/**
 * 「开机自记账」的记录样例 —— 开发与验证用。
 *
 * 数值刻意取**普通办公机的常见区间**（20–40 秒），不取自任何一台具体机器的实测：
 * 这里只用来验证图表与文案，带真实数据进来既没必要也容易泄露使用者的机器特征。
 *
 * 时间用相对当前的偏移生成而不是写死日期——写死的话过几天就变成"几个月前的开机"，
 * 相对时间文案（"2 小时前"）会被渲染成无意义的样子，看不出真实效果。
 */
const hoursAgo = (h: number): string =>
  new Date(Date.now() - h * 3600_000).toISOString()

export const MOCK_BOOT_RECORDS: BootRecord[] = [
  { hours: 3, totalMs: 24_100 },
  { hours: 27, totalMs: 41_800 },
  { hours: 51, totalMs: 22_600 },
  { hours: 74, totalMs: 26_400 },
  { hours: 99, totalMs: 23_900 },
  { hours: 123, totalMs: 38_200 },
  { hours: 147, totalMs: 21_500 },
].map(({ hours, totalMs }) => ({
  // 起点 = 记录时刻 − 已开机时长；样例里让它等同于"记录前几秒开始"，够真实。
  bootStartedAt: hoursAgo(hours),
  recordedAt: new Date(Date.now() - hours * 3600_000 + totalMs).toISOString(),
  totalMs,
  source: 'marker',
  // 样例数据代表"一切正常"的样子：系统日志能读到，所以是实测口径。
  basis: 'log',
}))
