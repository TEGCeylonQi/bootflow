//! 计划任务扫描器 —— 只收**开机时**与**登录时**触发的任务。
//!
//! 为什么单独一个来源：任务计划程序是"第三个壳"。同一件事既可以写成
//! Run 键值，也可以写成启动文件夹快捷方式，还可以写成计划任务——而
//! **计划任务是三者里唯一能表达"登录后延迟 30 秒再跑"的**。用户抱怨
//! 开机慢时，真正拖后腿的常常在这里，因为它在启动项列表里看不见。
//!
//! | 坑 | 如果不管会怎样 | 这里怎么处理 |
//! |---|---|---|
//! | 隐藏任务默认不返回 | 微软自带任务大量标记为隐藏，界面上凭空少一批 | `GetTasks(TASK_ENUM_HIDDEN)` |
//! | 触发器禁用 ≠ 任务禁用 | 优化软件常只关触发器，读任务级状态会报"启用中" | 两级状态分别读，都记进 `raw` |
//! | 动作用的不一定是 exe | 无路径的 COM 处理器被按空路径判存在性 → 误报"程序已卸载" | 非 Exec 动作不产生路径，交给有效检测判 `Unknown` |
//! | 任务名的可读性差异极大 | `OfficeTelemetryAgentLogOn` 这类名字直接给用户看等于没说 | 目标是通用宿主程序时改用任务名，其余走 exe 版本信息 |
//! | 触发器有 13 种 | 把定时任务也收进来，列表会被上百个"每天凌晨更新"淹没 | 只收 Boot / Logon，其余记日志 |
//! | 部分任务拒绝访问 | 直接放弃整个来源，界面一片空白 | 单任务失败只跳过该任务，计数后如实汇报 |
//!
//! ⚠️ 只读。本模块不出现任何 `RegisterTaskDefinition` / `SetEnabled` 调用。
//!
//! **刻意不收的触发器**及理由（写在这里避免以后有人以为是漏了）：
//! - `RegistrationTrigger`：它在任务被注册时触发一次，属于安装流程的一部分，
//!   重启电脑不会再跑，不构成开机负担。
//! - `EventTrigger`：它挂在事件日志上，触发时机不可预测。归 T8 的诊断引擎
//!   （事件驱动的启动行为）而不是启动项清单。
//! - `TimeTrigger` / `Daily` / `Weekly` / `Monthly*` / `Idle` / `SessionStateChange`：
//!   完全不参与开机。

#[cfg(windows)]
pub use imp::collect;

#[cfg(not(windows))]
pub fn collect() -> Result<Vec<crate::model::StartupItem>, String> {
    Err("计划任务扫描仅支持 Windows".to_string())
}

#[cfg(windows)]
mod imp {
    use windows::core::{BSTR, Interface, IUnknown, VARIANT};
    use windows::Win32::Foundation::VARIANT_BOOL;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows::Win32::System::TaskScheduler::{
        IBootTrigger, ILogonTrigger, IRegisteredTask, ITaskFolder, ITaskService, ITrigger,
        TASK_ACTION_EXEC, TASK_ENUM_HIDDEN, TASK_TRIGGER_BOOT, TASK_TRIGGER_LOGON,
        TASK_TRIGGER_TYPE2, TaskScheduler,
    };

    use crate::model::{
        BootPhase, DiagnosticInfo, ItemKind, NameSource, RiskLevel, SourceKind, StartupItem,
    };
    use crate::diag::{codes, known};
    use crate::scanners::builder::ItemBuilder;
    use crate::util::{cmdline, com::ComGuard};

    /// 任务文件夹递归上限。真实系统里最深的也就是 `\Microsoft\Windows\Xxx\`，
    /// 设上限纯粹是防御"文件夹互相包含"这种不该存在但没人能保证不存在的情况。
    const MAX_FOLDER_DEPTH: usize = 8;

    /// 这些程序是「通用宿主」：它们本身说明不了这个任务要干什么，
    /// `cmd.exe /c "..."` 的版本信息是 "Windows 命令处理程序"，
    /// 对用户毫无信息量。遇到它们改用任务自己的名字。
    ///
    /// `explorer.exe` 也在内：任务用 explorer 拉起一个文件很常见。
    const HOST_PROGRAMS: &[&str] = &[
        "cmd.exe",
        "powershell.exe",
        "pwsh.exe",
        "wscript.exe",
        "cscript.exe",
        "rundll32.exe",
        "mshta.exe",
        "msiexec.exe",
        "explorer.exe",
    ];

    #[derive(Default)]
    struct Ctx {
        items: Vec<StartupItem>,
        /// 因权限不足读不到定义的任务数。非空时必须让用户知道——
        /// 「界面上少了一批」比「多报一条」危险得多。
        denied: usize,
        /// 枚举出来的任务总数（含不被收的类型），用于日志与自检
        total_tasks: usize,
        /// 没有 Boot/Logon 触发器而跳过
        skipped_by_trigger: usize,
        /// 枚举本身出错的位置
        errors: Vec<String>,
        /// 权限不足导致整块读不到的任务文件夹
        denied_folders: usize,
    }

    /// 扫描所有含开机/登录触发器的计划任务。
    ///
    /// 返回 `Err` 只在**连接不上任务计划程序服务**时发生。单个任务或
    /// 单个文件夹失败只跳过它、计数后如实汇报——一个来源的部分失败
    /// 不能让整个界面空掉。
    pub fn collect() -> Result<Vec<StartupItem>, String> {
        // COM 是按线程初始化的，而调用方跑在线程池上——必须在这里初始化
        let _com = ComGuard::new();

        let service: ITaskService = unsafe {
            CoCreateInstance(
                &TaskScheduler,
                Option::<&IUnknown>::None,
                CLSCTX_INPROC_SERVER,
            )
        }
        .map_err(|e| format!("连接任务计划程序服务失败：{}", hr(e)))?;

        // 四个空 VARIANT 表示"本机、当前用户"。传用户名密码可以连远程机器，
        // 但启动项管理是本地操作，没有理由接受远程凭据。
        let empty = VARIANT::default();
        unsafe { service.Connect(&empty, &empty, &empty, &empty) }
            .map_err(|e| format!("初始化任务计划程序连接失败：{}", hr(e)))?;

        let root = unsafe { service.GetFolder(&BSTR::from("\\")) }
            .map_err(|e| format!("打开根任务文件夹失败：{}", hr(e)))?;

        let mut ctx = Ctx::default();
        walk(&root, 0, &mut ctx);

        for e in &ctx.errors {
            log::warn!("计划任务部分失败：{e}");
        }

        log::info!(
            "计划任务：枚举 {} 个，收下 {} 个（{} 个无开机/登录触发器，{} 个因权限跳过{}）",
            ctx.total_tasks,
            ctx.items.len(),
            ctx.skipped_by_trigger,
            ctx.denied,
            if ctx.denied_folders > 0 {
                format!("，{} 个文件夹不可读", ctx.denied_folders)
            } else {
                String::new()
            }
        );

        // ── 一个都没收到，但原因是权限 ──
        // 这时候必须报错而不是返回空列表：返回空列表会被界面渲染成
        // "你的电脑很干净，没有任何计划任务"，而事实正好相反。
        if ctx.items.is_empty() && ctx.total_tasks > 0 && ctx.denied >= ctx.total_tasks {
            return Err(format!(
                "{} 个计划任务全部因权限不足未能读取，请以管理员身份重试",
                ctx.denied
            ));
        }

        Ok(ctx.items)
    }

    /// 递归遍历任务文件夹。
    fn walk(folder: &ITaskFolder, depth: usize, ctx: &mut Ctx) {
        if depth > MAX_FOLDER_DEPTH {
            ctx.errors
                .push(format!("任务文件夹层级超过 {MAX_FOLDER_DEPTH} 层，未继续深入"));
            return;
        }

        let folder_path = read_bstr(|| unsafe { folder.Path().ok() }).unwrap_or_default();

        // ── 本层的任务 ──
        // 必须带 TASK_ENUM_HIDDEN：微软自带任务绝大多数是隐藏的，
        // 默认枚举会漏掉它们，而"漏"是这类工具最不该犯的错。
        match unsafe { folder.GetTasks(TASK_ENUM_HIDDEN.0) } {
            Ok(tasks) => {
                let count = unsafe { tasks.Count() }.unwrap_or(0);
                for i in 1..=count {
                    match unsafe { tasks.get_Item(&VARIANT::from(i)) } {
                        Ok(task) => {
                            ctx.total_tasks += 1;
                            consider(&task, &folder_path, ctx);
                        }
                        Err(e) => ctx
                            .errors
                            .push(format!("读取「{folder_path}」下的第 {i} 个任务失败：{}", hr(e))),
                    }
                }
            }
            Err(e) => {
                // 拒绝访问在这里是"整块文件夹看不到"，比单个任务更严重
                if is_access_denied(&e) {
                    ctx.denied_folders += 1;
                }
                ctx.errors
                    .push(format!("枚举「{folder_path}」下的任务失败：{}", hr(e)));
            }
        }

        // ── 子文件夹 ──
        // Office / 各家软件的任务都放在自己的子文件夹里，不递归等于漏掉大半。
        match unsafe { folder.GetFolders(0) } {
            Ok(subs) => {
                let count = unsafe { subs.Count() }.unwrap_or(0);
                for i in 1..=count {
                    match unsafe { subs.get_Item(&VARIANT::from(i)) } {
                        Ok(sub) => walk(&sub, depth + 1, ctx),
                        Err(e) => ctx.errors.push(format!(
                            "读取「{folder_path}」下的第 {i} 个子文件夹失败：{}",
                            hr(e)
                        )),
                    }
                }
            }
            Err(e) => ctx
                .errors
                .push(format!("枚举「{folder_path}」的子文件夹失败：{}", hr(e))),
        }
    }

    /// 判断一个任务是否算启动项，算就构造出来。
    fn consider(task: &IRegisteredTask, folder_path: &str, ctx: &mut Ctx) {
        let full_path = match read_bstr(|| unsafe { task.Path().ok() }) {
            Some(p) if !p.is_empty() => p,
            _ => format!(
                "{folder_path}{}",
                read_bstr(|| unsafe { task.Name().ok() }).unwrap_or_default()
            ),
        };

        // 定义读不到：非管理员下少数系统任务会拒绝访问。
        // 跳过而不是伪造一条空记录——伪记录会让用户以为这个任务没配置。
        let definition = match unsafe { task.Definition() } {
            Ok(d) => d,
            Err(e) => {
                ctx.denied += 1;
                log::info!("任务「{full_path}」的定义无法读取，已跳过：{}", hr(e));
                return;
            }
        };

        // ── 触发器筛选 ──
        let triggers = match unsafe { definition.Triggers() } {
            Ok(t) => t,
            Err(e) => {
                log::info!("任务「{full_path}」的触发器无法读取，已跳过：{}", hr(e));
                ctx.denied += 1;
                return;
            }
        };

        let mut trigger_count = 0i32;
        if unsafe { triggers.Count(&mut trigger_count) }.is_err() {
            ctx.denied += 1;
            return;
        }

        let mut matched: Vec<TriggerInfo> = Vec::new();

        for i in 1..=trigger_count {
            let Ok(trigger) = (unsafe { triggers.get_Item(i) }) else {
                continue;
            };

            let mut ttype = TASK_TRIGGER_TYPE2::default();
            if unsafe { trigger.Type(&mut ttype) }.is_err() {
                continue;
            }

            let phase = if ttype == TASK_TRIGGER_BOOT {
                // BootTrigger 在服务/驱动初始化之后、用户登录之前被拉起，
                // 归到会话管理器（Smss）之后的启动阶段。
                BootPhase::Smss
            } else if ttype == TASK_TRIGGER_LOGON {
                BootPhase::Logon
            } else {
                continue;
            };

            matched.push(TriggerInfo {
                phase,
                kind: if ttype == TASK_TRIGGER_BOOT {
                    "boot"
                } else {
                    "logon"
                },
                enabled: read_trigger_enabled(&trigger),
                delay_sec: read_trigger_delay(&trigger, ttype),
                detail: read_trigger_detail(&trigger, ttype),
            });
        }

        if matched.is_empty() {
            ctx.skipped_by_trigger += 1;
            return;
        }

        // ── 动作 ──
        let actions = read_actions(&definition);

        // ── 启用状态：任务级 与 触发器级 是两回事 ──
        //
        // 优化软件的手法通常是**只关触发器**，任务本身仍是"已启用"。
        // 只看任务级状态会把这类项报成启用中，用户按界面去关反而关错了地方。
        let task_enabled = unsafe { task.Enabled() }
            .map(|b| b.as_bool())
            .unwrap_or(true);
        let any_trigger_enabled = matched.iter().any(|t| t.enabled);
        let enabled = task_enabled && any_trigger_enabled;

        // ── 名称 ──
        // 目标是通用宿主程序（cmd / powershell / rundll32…）时，exe 的版本信息
        // 说明不了任何事，改用任务名；其余情况**不设** display_name，
        // 让 builder 走 exe 版本信息——这样同一个程序在 Run 键与计划任务里
        // 会显示成同一个名字，"统一识别"才成立。
        let name = read_bstr(|| unsafe { task.Name().ok() }).unwrap_or_default();
        let primary_target = actions.first().map(|a| a.path.clone()).unwrap_or_default();
        // 裸程序名在 `normalize_target` 里已经补全过了，所以这里只剩两种
        // 需要改名的情形：没有可执行路径，或者它是通用宿主程序。
        let needs_task_name = primary_target.is_empty() || is_host_program(&primary_target);

        // 命令行：Exec 动作的 Path 与 Arguments 本来就是分开存的，
        // 比 Run 键的字符串干净，但为了和其他来源一致仍拼成一条。
        let command = match actions.first() {
            Some(a) if !a.arguments.is_empty() => format!("{} {}", a.path, a.arguments),
            Some(a) => a.path.clone(),
            None => String::new(),
        };
        let args = match actions.first() {
            Some(a) if !a.arguments.is_empty() => cmdline::parse_args_only(&a.arguments),
            _ => Vec::new(),
        };

        // ── 刻意**不**把禁用项标成 `DisabledRemnant` ──
        //
        // 那个状态在 T5 里专指"重复且已停用的自启入口"，它带出的建议是
        // "清理掉不会影响那个在用的入口"——对计划任务完全不适用。
        // 一个被禁用的登录任务不是残渣，它是**用户或优化软件的有意配置**，
        // 而且很可能正是用户想恢复的东西（Office 打不开的根因就在这一类）。
        // 所以这里只如实给出 `enabled = false`，让界面显示"已停用"，
        // 至于"该不该恢复它"归 T8 的诊断引擎判断。
        let principal = unsafe { definition.Principal() }.ok();
        let user_id = principal
            .as_ref()
            .and_then(|p| take_bstr(|b| unsafe { p.UserId(b) }))
            .unwrap_or_default();
        let run_level = read_run_level(&definition).unwrap_or_default();

        let reg_info = unsafe { definition.RegistrationInfo() }.ok();
        let author = reg_info
            .as_ref()
            .and_then(|i| take_bstr(|b| unsafe { i.Author(b) }));
        let description = reg_info
            .as_ref()
            .and_then(|i| take_bstr(|b| unsafe { i.Description(b) }));
        let hidden = unsafe { definition.Settings() }
            .ok()
            .and_then(|s| take_bool(|b| unsafe { s.Hidden(b) }))
            .unwrap_or(false);

        let raw = serde_json::json!({
            "kind": "scheduledTask",
            "taskPath": full_path.clone(),
            "taskName": name.clone(),
            "taskEnabled": task_enabled,
            "triggerEnabled": any_trigger_enabled,
            "hidden": hidden,
            "author": author,
            "description": description,
            "triggers": matched.iter().map(|t| serde_json::json!({
                "type": t.kind,
                "enabled": t.enabled,
                "delay": t.delay_sec,
                "detail": t.detail,
            })).collect::<Vec<_>>(),
            "actions": actions.iter().map(|a| serde_json::json!({
                "type": "exec",
                "path": a.path,
                "arguments": a.arguments,
                "workingDirectory": a.working_directory,
            })).collect::<Vec<_>>(),
            "principal": {
                "userId": user_id,
                "runLevel": run_level,
            },
        });

        let mut builder = ItemBuilder::new(SourceKind::ScheduledTask, name.clone())
            .scope(scope_of(&user_id))
            .location(full_path.clone())
            .command(command)
            .target(primary_target.clone())
            .args(args)
            .enabled(enabled)
            // 一个任务可能有 Boot+Logon 两个触发器，取"更早的那个"作为相位。
            // 用户看到"开机时"比看到"登录时"更能解释为什么它会拖慢开机。
            .boot_phase(
                matched
                    .iter()
                    .map(|t| t.phase)
                    .min_by_key(|p| phase_rank(*p))
                    .unwrap_or(BootPhase::Logon),
            )
            .raw(raw);

        if needs_task_name {
            builder = builder.display_name(name.clone(), NameSource::TaskDescription);
        }

        // Windows 自己注册的任务：任务路径在 `\Microsoft\Windows\` 下。
        // 必须显式标出来，因为其中相当一部分的动作是 COM 处理器、
        // 没有目标文件，光靠"目标在不在 System32"判不出系统组件，
        // 它们会和第三方任务混成一片（见 builder::os_component）。
        if is_windows_component_task(&full_path) {
            builder = builder.os_component();
        }

        // 一个任务可以挂多个动作。这里只把第一个 Exec 动作当"这条启动项
        // 启动了什么"，但要在属性面板里说清还有别的——否则用户以为
        // 处理掉一个程序就完事了。
        let extra = actions.len().saturating_sub(1);
        if extra > 0 {
            builder = builder.diagnostic(DiagnosticInfo {
                code: codes::MULTI_ACTION_TASK.to_string(),
                severity: RiskLevel::Safe,
                message: format!(
                    "这个任务除了上面那个程序以外，还会一并启动 {extra} 个程序——\
                     它们同属一个任务，停用或启用是整体生效的。"
                ),
                evidence: Some(
                    actions
                        .iter()
                        .skip(1)
                        .map(|a| a.path.clone())
                        .collect::<Vec<_>>()
                        .join("；"),
                ),
            });
        }

        let mut item = builder.build();

        // 禁用态要交代清楚"是什么被禁用了"：「任务本身被禁用」和
        // 「只是触发器被关掉」在任务计划程序里是两个不同位置的开关，
        // 后者甚至会让任务显示成「准备就绪」——不说明白，用户按界面上
        // 的"已停用"去任务计划程序里翻，会完全找不到该改哪里。
        //
        // **只对非系统任务说**。Windows 自己有一批任务出厂就是禁用的
        // （本机实测 55 项里有 26 项禁用，绝大多数是系统维护任务），
        // 对它们逐条解释"这个任务被禁用了、如需使用请重新启用"，
        // 既会把属性面板塞满无用说明，又在暗示用户去改那些本来就这样配置的
        // 系统任务——那是这类工具最不该鼓励的行为。
        // Office 的后台任务单独处理，**不走下面那条通用说明**。
        // 原因见 `diag::known::OFFICE_BACKGROUND_TASKS` 的长注释：
        // 这些任务被关掉很可能是用户有意为之，把它渲染成"该修复的问题"
        // 等于在纠正用户的个人选择。
        let is_office_task = known::lookup_office_task(&full_path).is_some();

        if !item.enabled && item.kind != ItemKind::System && !is_office_task {
            item.diagnostics.push(DiagnosticInfo {
                code: if task_enabled {
                    codes::TASK_TRIGGER_DISABLED
                } else {
                    codes::TASK_DISABLED
                }
                .to_string(),
                severity: RiskLevel::Safe,
                message: if task_enabled {
                    "这个任务本身是启用状态，但它的开机/登录触发器被单独关掉了，\
                     所以不会运行。恢复时要展开任务的「触发器」选项卡才会看到那个开关。"
                        .to_string()
                } else {
                    "这个任务被整体禁用了，不会在开机或登录时运行。\
                     如果它属于你正在使用的软件，可以在任务计划程序里重新启用它。"
                        .to_string()
                },
                evidence: None,
            });
        }

        // ── Office 后台任务被关闭 ──
        //
        // 这条诊断的存在理由，是**用户往往不知道是自己关的**：某次装了
        // 一个"系统优化大师"、或者点过一次"一键加速"，这几个任务就没了。
        // 等到某天发现 Office 不再自动更新、反复提示版本过旧时，
        // 没人会把这两件事联系起来。
        //
        // 所以措辞必须**不评判**：不说"你该把它打开"，只说清代价，
        // 把决定权留给用户——这是唯一诚实的位置。
        if known::is_office_task_disabled(&full_path, task_enabled) {
            let purpose = known::lookup_office_task(&full_path)
                .map(|k| k.purpose)
                .unwrap_or("支撑 Office 的后台功能");

            item.diagnostics.push(DiagnosticInfo {
                code: codes::OFFICE_TASKS_DISABLED.to_string(),
                severity: RiskLevel::Medium,
                message: format!(
                    "这是 Office 自带的后台任务，用途是{purpose}。它现在被关闭了——\
                     好处是登录时少一点后台活动，代价是 Office 的自动更新与\
                     部分功能会一并停掉。如果这是你自己（或某个优化工具）关掉的，\
                     不用处理；如果不知道有这回事，可以在这里重新启用它。"
                ),
                evidence: Some(format!(
                    "任务 {full_path}：任务本身 Enabled={task_enabled}"
                )),
            });
        }

        ctx.items.push(item);
    }

    /// 这个任务是不是 Windows 自己注册的组件任务。
    ///
    /// 判据用**任务路径前缀** `\Microsoft\Windows\`——Windows 组件把任务
    /// 注册在这个固定位置，本机 55 项里有 35 项来自这里。
    ///
    /// 为什么不用"作者是不是 Microsoft"：
    /// `\Microsoft\Office\` 下的任务作者也是 Microsoft，但那些是 Office
    /// 这个应用自己的任务，用户**需要**看到它们——本机上 Office 的 5 个
    /// 登录任务全部被关掉了，那正是用户最该知道的一件事。
    /// 也不能用"目标在不在 System32"：COM 处理器类型的任务没有目标文件。
    fn is_windows_component_task(task_path: &str) -> bool {
        task_path
            .to_ascii_lowercase()
            .starts_with(r"\microsoft\windows\")
    }

    // ─────────────────────── 读取辅助 ───────────────────────

    struct TriggerInfo {
        phase: BootPhase,
        kind: &'static str,
        enabled: bool,
        delay_sec: Option<u64>,
        detail: Option<String>,
    }

    struct ActionInfo {
        path: String,
        arguments: String,
        working_directory: String,
    }

    /// 相位排序：Boot 早于 Logon。
    ///
    /// 直接用 `BootPhase` 的 `ORDER` 下标会引入对数组顺序的依赖，
    /// 而且那个数组只覆盖到 Logon，这里只要比大小，写死两个值更稳。
    fn phase_rank(p: BootPhase) -> u8 {
        match p {
            BootPhase::Smss => 0,
            _ => 1,
        }
    }

    /// `ITrigger::Enabled` 读不到时**默认为启用**。
    ///
    /// 这个默认方向是刻意的：读不到说明触发器是"未设置"状态（接口允许
    /// 返回失败表示未设置），而未设置的触发器按系统约定就是启用的。
    /// 反过来默认禁用会把大量正常任务标成"已停用"，让用户以为自己的电脑
    /// 有一堆东西被关掉了。
    fn read_trigger_enabled(trigger: &ITrigger) -> bool {
        let mut value = VARIANT_BOOL::default();
        if unsafe { trigger.Enabled(&mut value) }.is_err() {
            return true;
        }
        value.as_bool()
    }

    /// 触发器的延迟（ISO8601 时长，如 `PT30S`）。
    ///
    /// 只对 Boot / Logon 触发器有意义——这两类才有 `Delay` 属性。
    fn read_trigger_delay(trigger: &ITrigger, ttype: TASK_TRIGGER_TYPE2) -> Option<u64> {
        let mut delay = BSTR::default();

        // 注意 `cast` 本身是安全方法，只有底下的 `Delay` 调用是 unsafe。
        // 把 cast 包进 unsafe 块骗过编译器只会掩盖真正的边界在哪。
        let ok = if ttype == TASK_TRIGGER_BOOT {
            match trigger.cast::<IBootTrigger>() {
                Ok(t) => unsafe { t.Delay(&mut delay) }.is_ok(),
                Err(_) => false,
            }
        } else {
            match trigger.cast::<ILogonTrigger>() {
                Ok(t) => unsafe { t.Delay(&mut delay) }.is_ok(),
                Err(_) => false,
            }
        };

        if !ok {
            return None;
        }
        parse_iso_duration_sec(&delay.to_string())
    }

    /// 触发器的补充信息：登录触发器会限定用户，开机触发器没有。
    fn read_trigger_detail(trigger: &ITrigger, ttype: TASK_TRIGGER_TYPE2) -> Option<String> {
        if ttype != TASK_TRIGGER_LOGON {
            return None;
        }
        let logon = trigger.cast::<ILogonTrigger>().ok()?;

        let mut user = BSTR::default();
        if unsafe { logon.UserId(&mut user) }.is_err() {
            return None;
        }

        let user = user.to_string();
        (!user.trim().is_empty()).then_some(format!("仅限用户：{user}"))
    }

    /// 读取所有 Exec 动作；非 Exec 动作只跳过不产出路径。
    fn read_actions(
        definition: &windows::Win32::System::TaskScheduler::ITaskDefinition,
    ) -> Vec<ActionInfo> {
        let mut out = Vec::new();

        let Ok(collection) = (unsafe { definition.Actions() }) else {
            return out;
        };

        let mut count = 0i32;
        if unsafe { collection.Count(&mut count) }.is_err() {
            return out;
        }

        for i in 1..=count {
            let Ok(action) = (unsafe { collection.get_Item(i) }) else {
                continue;
            };

            let mut atype = windows::Win32::System::TaskScheduler::TASK_ACTION_TYPE::default();
            if unsafe { action.Type(&mut atype) }.is_err() || atype != TASK_ACTION_EXEC {
                // COM 处理器 / 发邮件 / 显示消息：它们确实会"启动"
                // 某样东西，但没有可执行文件路径可言。
                // 这里跳过路径的读取，该任务照样会作为启动项出现，
                // 只是有效性检测会给它 `Unknown`（信息不足）而不是
                // "目标不存在"——后者是纯粹的假警报。
                continue;
            }

            let Ok(exec) = action.cast::<windows::Win32::System::TaskScheduler::IExecAction>() else {
                continue;
            };

            let mut path = BSTR::default();
            if unsafe { exec.Path(&mut path) }.is_err() {
                continue;
            }

            let path = normalize_target(&path.to_string());
            if path.is_empty() {
                continue;
            }

            let mut arguments = BSTR::default();
            let arguments = if unsafe { exec.Arguments(&mut arguments) }.is_ok() {
                arguments.to_string()
            } else {
                String::new()
            };

            let mut workdir = BSTR::default();
            let working_directory = if unsafe { exec.WorkingDirectory(&mut workdir) }.is_ok() {
                workdir.to_string()
            } else {
                String::new()
            };

            out.push(ActionInfo {
                path,
                arguments,
                working_directory,
            });
        }

        out
    }

    /// 把任务动作里的路径整理成"可以拿去判存在性"的形式。
    ///
    /// 和 Run 键相比这里干净得多——Path 与 Arguments 分开存放，不会出现
    /// `"C:\a b\x.exe" -flag` 这种需要猜的字符串。但两件事仍要做：
    /// 1. 展开 `%windir%` 这类变量（任务里非常常见）
    /// 2. 裸程序名按系统搜索顺序补全（否则 `rundll32.exe` 会被误判成已卸载）
    fn normalize_target(raw: &str) -> String {
        let expanded = cmdline::expand_env(raw.trim());
        let expanded = expanded.trim().trim_matches('"').trim().to_string();

        if expanded.is_empty() {
            return expanded;
        }

        if !cmdline::has_path_separator(&expanded) {
            if let Some(found) = cmdline::resolve_via_search_path(&expanded) {
                return found;
            }
        }

        expanded
    }

    fn is_host_program(path: &str) -> bool {
        let base = path
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(path)
            .to_ascii_lowercase();
        HOST_PROGRAMS.contains(&base.as_str())
    }

    /// 任务的作用域：以系统账户运行的是全机级，其余归用户级。
    ///
    /// 判据用**账户**而不是"有没有勾最高权限"——普通用户为自己的软件建一个
    /// 勾了"以最高权限运行"的任务，它照样只在这个用户登录时运行，性质是用户级。
    /// 反过来，用文件夹位置判断也不可靠：第三方软件把全机级任务放在自己名字
    /// 的文件夹下，靠路径前缀认会漏。
    fn scope_of(user_id: &str) -> crate::model::Scope {
        /// 系统账户的各种写法。任务定义里既可能是显示名，也可能是 SID。
        const SYSTEM_ACCOUNTS: &[&str] = &[
            "system",
            "localsystem",
            "local system",
            "local service",
            "networkservice",
            "network service",
            "s-1-5-18", // SYSTEM
            "s-1-5-19", // LOCAL SERVICE
            "s-1-5-20", // NETWORK SERVICE
        ];

        let u = user_id.trim().to_ascii_lowercase();
        let is_system = SYSTEM_ACCOUNTS.contains(&u.as_str()) || u.starts_with("nt authority\\");

        if is_system {
            crate::model::Scope::Machine
        } else {
            crate::model::Scope::User
        }
    }

    fn read_run_level(
        definition: &windows::Win32::System::TaskScheduler::ITaskDefinition,
    ) -> Option<String> {
        use windows::Win32::System::TaskScheduler::{TASK_RUNLEVEL_HIGHEST, TASK_RUNLEVEL_TYPE};

        let principal = unsafe { definition.Principal() }.ok()?;
        let mut level = TASK_RUNLEVEL_TYPE::default();
        unsafe { principal.RunLevel(&mut level) }.ok()?;

        // 存成字符串而不是枚举的 Debug 输出：`TASK_RUNLEVEL_TYPE(1)` 这种
        // 东西写进 raw 里，将来做回滚审计时没人看得懂。
        Some(if level == TASK_RUNLEVEL_HIGHEST {
            "highest"
        } else {
            "limited"
        }
        .to_string())
    }

    /// 调用一个读接口并取回可读字符串，失败或空值时返回 `None`。
    ///
    /// 闭包返回 `Option<BSTR>` 而不是 `Result<BSTR>`：任务接口里
    /// "属性没设置"是常态（绝大多数任务没有描述），`Option` 表达它更直接，
    /// 调用点也不用为每个属性编一个不会有人看的 HRESULT。
    fn read_bstr(f: impl FnOnce() -> Option<BSTR>) -> Option<String> {
        let s = f()?.to_string();
        (!s.trim().is_empty()).then_some(s)
    }

    /// 读一个**以出参形式**返回字符串的属性（`fn(&mut BSTR) -> Result<()>`）。
    ///
    /// 任务接口里两种风格混着用：`IRegisteredTask::Name` 直接返回 `BSTR`，
    /// 而 `IPrincipal::UserId` / `IRegistrationInfo::Author` 是出参形式。
    /// 这不是我们能选的，是 win32 元数据里就这么标的——
    /// 所以这里备两个入口，而不是在调用点用 `zeroed()` 硬凑。
    fn take_bstr(f: impl FnOnce(&mut BSTR) -> windows::core::Result<()>) -> Option<String> {
        let mut buf = BSTR::default();
        f(&mut buf).ok()?;
        let s = buf.to_string();
        (!s.trim().is_empty()).then_some(s)
    }

    /// 读一个以出参形式返回布尔值的属性。
    fn take_bool(f: impl FnOnce(&mut VARIANT_BOOL) -> windows::core::Result<()>) -> Option<bool> {
        let mut v = VARIANT_BOOL::default();
        f(&mut v).ok()?;
        Some(v.as_bool())
    }

    /// HRESULT → 用户能看懂的话。
    fn hr(e: windows::core::Error) -> String {
        const E_ACCESSDENIED: i32 = 0x8007_0005u32 as i32;
        const SCHED_E_SERVICE_NOT_RUNNING: i32 = 0x8004_1315u32 as i32;
        const SCHED_E_MALFORMEDXML: i32 = 0x8004_1318u32 as i32;

        match e.code().0 {
            E_ACCESSDENIED => "拒绝访问（需要管理员权限）".to_string(),
            SCHED_E_SERVICE_NOT_RUNNING => {
                "任务计划程序服务没有运行（它的启动类型可能被改成了禁用）".to_string()
            }
            SCHED_E_MALFORMEDXML => "任务定义已损坏，任务计划程序自己也读不了它".to_string(),
            _ => e.to_string(),
        }
    }

    fn is_access_denied(e: &windows::core::Error) -> bool {
        e.code().0 == 0x8007_0005u32 as i32
    }

    /// 解析 ISO8601 时长（`PT30S` / `PT1M30S` / `P1DT2H`），返回秒数。
    ///
    /// 只支持任务计划程序实际会产出的形式。看不懂就返回 `None`——
    /// 猜一个延迟值会直接误导用户对"这个任务到底拖不拖开机"的判断。
    fn parse_iso_duration_sec(input: &str) -> Option<u64> {
        let s = input.trim();
        let body = s.strip_prefix('P')?.trim();
        if body.is_empty() {
            return None;
        }

        let mut total = 0u64;
        let mut digits = String::new();
        let mut in_time = false;

        for c in body.chars() {
            match c {
                'T' => in_time = true,
                'D' if !in_time => {
                    total += digits.parse::<u64>().ok()? * 86_400;
                    digits.clear();
                }
                'H' if in_time => {
                    total += digits.parse::<u64>().ok()? * 3_600;
                    digits.clear();
                }
                // 日期段的 `M` 是"月"，长度不定，不猜
                'M' if in_time => {
                    total += digits.parse::<u64>().ok()? * 60;
                    digits.clear();
                }
                'S' if in_time => {
                    total += digits.parse::<u64>().ok()?;
                    digits.clear();
                }
                d if d.is_ascii_digit() => digits.push(d),
                _ => return None,
            }
        }

        Some(total)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::model::{SourceKind, ValidityStatus};

    #[test]
    fn iso_duration_parsing_covers_task_scheduler_forms() {
        assert_eq!(parse_iso_duration_sec("PT30S"), Some(30));
        assert_eq!(parse_iso_duration_sec("PT1M"), Some(60));
        assert_eq!(parse_iso_duration_sec("PT1M30S"), Some(90));
        assert_eq!(parse_iso_duration_sec("PT2H"), Some(7_200));
        assert_eq!(parse_iso_duration_sec("P1DT2H"), Some(93_600));
        assert_eq!(parse_iso_duration_sec("PT0S"), Some(0));
    }

    #[test]
    fn unknown_duration_is_not_guessed() {
        // 月份长度不定，宁可返回 None 也不要编一个数字出来——
        // 延迟值会直接进用户对"这任务拖不拖开机"的判断
        assert_eq!(parse_iso_duration_sec("P1M"), None);
        assert_eq!(parse_iso_duration_sec("30S"), None);
        assert_eq!(parse_iso_duration_sec(""), None);
        assert_eq!(parse_iso_duration_sec("nonsense"), None);
    }

    #[test]
    fn host_programs_are_detected_by_filename() {
        assert!(is_host_program(r"C:\Windows\System32\cmd.exe"));
        assert!(is_host_program("powershell.exe"));
        assert!(is_host_program(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"));
        assert!(is_host_program(r"C:\Windows\System32\RUNDLL32.EXE"));
        assert!(!is_host_program(r"C:\Program Files\App\app.exe"));
    }

    #[test]
    fn target_normalization_expands_variables_and_unquotes() {
        let got = normalize_target("\"%WINDIR%\\System32\\notepad.exe\"");
        assert!(
            got.to_ascii_lowercase().ends_with("notepad.exe"),
            "应展开变量并去掉引号，实际：{got}"
        );
        assert!(!got.contains('%'), "不该留下未展开的变量：{got}");
    }

    #[test]
    fn bare_program_name_is_completed_to_a_real_path() {
        // 任务里写成裸程序名同样存在，不补全会得到"程序已卸载"的假阳性
        let got = normalize_target("rundll32.exe");
        assert!(
            std::path::Path::new(&got).is_file(),
            "应补全为真实文件路径，实际：{got}"
        );
    }

    #[test]
    fn missing_target_keeps_its_original_form() {
        // 补全不了就原样返回，交由有效性检测如实报"目标不存在"，
        // 而不是编一个路径出来
        let got = normalize_target(r"D:\__bootflow_missing__\x.exe");
        assert_eq!(got, r"D:\__bootflow_missing__\x.exe");
    }

    /// 真机诊断：把这台机器上所有任务的触发器类型与筛选结果打出来。
    ///
    /// 存在的意义是**能看出"没扫到"和"扫到但没收"的区别**——
    /// 只收 Boot/Logon 是设计选择，但用户看到界面数量变少时无从判断
    /// 那是设计还是 bug。这个测试让两者可区分。
    #[test]
    fn report_scheduled_tasks() {
        let items = collect().unwrap_or_default();

        println!("\n计划任务：收下 {} 项（仅 Boot / Logon 触发器）", items.len());

        // 先给总览。收下几十项时，逐项读之前需要先知道"大头在哪一档"——
        // 否则 55 条明细里找不出该关注哪 10 条。
        let mut by_kind: std::collections::BTreeMap<String, (usize, usize)> =
            std::collections::BTreeMap::new();
        for it in &items {
            let entry = by_kind.entry(format!("{:?}", it.kind)).or_insert((0, 0));
            entry.0 += 1;
            if it.enabled {
                entry.1 += 1;
            }
        }
        println!("\n  按类型（总数 / 其中启用）：");
        for (kind, (total, on)) in &by_kind {
            println!("    {kind:<8} {total:>3} / {on:>3}");
        }

        println!("\n  需要关注的（非系统组件、或已停用）：");
        for it in items
            .iter()
            .filter(|i| i.kind != ItemKind::System || !i.enabled)
        {
            println!(
                "    [{:?}] {} — {} ({})",
                it.kind,
                it.display_name.as_deref().unwrap_or(&it.name),
                if it.enabled { "启用" } else { "已停用" },
                it.location
            );
        }

        for it in &items {
            println!("\n  [{}] {}", it.name, it.location);
            println!(
                "      显示名 = {}（来源 {:?}）",
                it.display_name.as_deref().unwrap_or("(回退到原始名)"),
                it.name_from
            );
            println!("      目标   = {}", it.resolved_path);
            println!("      命令行 = {}", it.command);
            println!(
                "      启用   = {}  相位 = {:?}  范围 = {:?}",
                it.enabled, it.boot_phase, it.scope
            );
            println!("      有效性 = {:?}", it.validity);
            if let Some(t) = it.raw.get("triggers").and_then(|v| v.as_array()) {
                for tr in t {
                    println!(
                        "      触发器 = {} 启用={} 延迟={:?}",
                        tr.get("type").and_then(|v| v.as_str()).unwrap_or("?"),
                        tr.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
                        tr.get("delay").and_then(|v| v.as_u64()),
                    );
                }
            }
            println!(
                "      任务启用 = {:?}  触发器启用 = {:?}",
                it.raw.get("taskEnabled"),
                it.raw.get("triggerEnabled")
            );
            println!(
                "      作者 = {}",
                it.raw
                    .get("author")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(无)")
            );
            println!(
                "      发布者 = {}（签名 {}）",
                it.signer.publisher.as_deref().unwrap_or("(无)"),
                if it.signer.is_signed { "通过" } else { "未通过" }
            );
        }
    }

    /// 基本契约：不会 panic、来源正确、禁用的项被如实标为停用。
    #[test]
    fn collect_is_consistent() {
        let items = collect().unwrap_or_default();

        for it in &items {
            assert_eq!(it.source, SourceKind::ScheduledTask);
            assert!(!it.name.is_empty(), "任务名不该为空：{it:?}");
            assert!(
                it.location.starts_with('\\'),
                "任务位置应是全路径（\\开头），实际：{}",
                it.location
            );
            assert!(it.raw.is_object(), "raw 必须是对象，供写操作回滚定位");

            // 禁用项**不能**被标成 `DisabledRemnant`：那个状态的建议文案是
            // "清理掉不会影响那个在用的入口"，只对重复项成立。用在这里会让
            // 用户把一条自己需要的、被优化软件关掉的登录任务当成垃圾清掉。
            if !it.enabled {
                assert_ne!(
                    it.validity,
                    ValidityStatus::DisabledRemnant,
                    "禁用任务不是「残余记录」：{}",
                    it.location
                );
            }

            // 系统自带任务被禁用是常态（出厂就有一批是关着的），
            // 不对它们产生诊断——否则属性面板会被无用的说明塞满，
            // 那份说明还等于在鼓励用户去动这些任务。
            if it.kind == ItemKind::System {
                assert!(
                    !it.diagnostics
                        .iter()
                        .any(|d| {
                            d.code == crate::diag::codes::TASK_DISABLED
                                || d.code == crate::diag::codes::TASK_TRIGGER_DISABLED
                        }),
                    "系统自带任务的禁用状态不该产生诊断：{}",
                    it.location
                );
            }
        }
    }

    /// 真机数据逼出来的规则：`\Microsoft\Windows\` 下的任务必须是系统组件。
    ///
    /// 本机 55 项里有 23 项动作是 COM 处理器、**没有目标文件**，
    /// 靠"目标在不在 System32"判不出系统组件，它们会作为普通「计划任务」
    /// 混进第三方那一堆里——用户看不出哪个是 Windows 自己的维护任务。
    #[test]
    fn windows_tasks_are_classified_as_system_components() {
        let items = collect().unwrap_or_default();
        let mut windows_tasks = 0;
        let mut office_tasks = 0;

        for it in &items {
            let lower = it.location.to_ascii_lowercase();

            if is_windows_component_task(&it.location) {
                windows_tasks += 1;
                assert_eq!(
                    it.kind,
                    ItemKind::System,
                    "Windows 自带任务应归为系统组件：{}",
                    it.location
                );
            }

            // 反向守卫：Office 的任务作者也是 Microsoft，但它属于**应用**。
            // 一旦被误判成系统组件，用户就看不到"Office 的登录任务被关掉了"
            // 这条最该知道的信息。
            if lower.starts_with(r"\microsoft\office\") {
                office_tasks += 1;
                assert_ne!(
                    it.kind,
                    ItemKind::System,
                    "Office 的任务不是 Windows 组件：{}",
                    it.location
                );
            }
        }

        println!("\nWindows 自带任务 {windows_tasks} 项，Office 任务 {office_tasks} 项");
    }

    #[test]
    fn windows_component_prefix_is_recognised_case_insensitively() {
        assert!(is_windows_component_task(
            r"\Microsoft\Windows\Wininet\CacheTask"
        ));
        assert!(is_windows_component_task(
            r"\microsoft\windows\UpdateOrchestrator\Reboot"
        ));
        // Office 是应用而不是 Windows 组件，这条边界最容易被误伤
        assert!(!is_windows_component_task(
            r"\Microsoft\Office\Office Feature Updates Logon"
        ));
        // PowerToys 会把用户名拼进任务名——真实机器上长这样，必须不误判为系统组件
        assert!(!is_windows_component_task(r"\PowerToys\Autorun for <username>"));
        assert!(!is_windows_component_task(r"\Clash Verge"));
        // 根目录下叫 Microsoft 的任务不是 Windows 组件
        assert!(!is_windows_component_task(r"\Microsoft"));
    }

    #[test]
    fn scope_follows_the_running_account() {
        use crate::model::Scope;

        assert_eq!(scope_of("SYSTEM"), Scope::Machine);
        assert_eq!(scope_of("NT AUTHORITY\\SYSTEM"), Scope::Machine);
        assert_eq!(scope_of("S-1-5-18"), Scope::Machine);
        assert_eq!(scope_of("LOCAL SERVICE"), Scope::Machine);
        assert_eq!(scope_of("Network Service"), Scope::Machine);
        // 普通用户自己的任务，即使勾了最高权限也是用户级——
        // 它只在这个用户登录时运行
        assert_eq!(scope_of("DESKTOP-1\\alice"), Scope::User);
        assert_eq!(scope_of("alice@example.com"), Scope::User);
        // 读不到账户信息时保守归用户级：宁可少报"影响全机"，
        // 也不要让一个只影响自己的项被说成全局风险
        assert_eq!(scope_of(""), Scope::User);
    }
}
}
