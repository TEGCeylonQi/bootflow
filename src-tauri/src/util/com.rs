//! COM 初始化 RAII 守卫。
//!
//! 为什么需要它：COM 是**按线程**初始化的，而 Tauri 的 `spawn_blocking`
//! 会让任务落在**线程池里的任意线程**上——同一个扫描任务这次跑在 A 线程、
//! 下次跑在 B 线程。所以不能"在程序启动时初始化一次就完事"，
//! 必须在每个可能用到 COM 的阻塞任务内部自己初始化。
//!
//! 这也是为什么把这个守卫做成 RAII：中途 `?` 提前返回时能自动 `CoUninitialize`，
//! 不会因为错误路径漏掉清理。
//!
//! ## ⚠️ 使用它的函数有一条硬规矩：COM 对象必须活不过守卫
//!
//! `ComGuard` 析构时会调 `CoUninitialize()`；**那会把 COM 的引用计数清零，
//! 并卸载本进程加载过的进程内 COM 服务器 DLL**（例如计划任务的 `taskschd.dll`）。
//! 此后任何残留的 COM 接口指针再去调 `Release()`，就是跳进一段**已卸载的代码**，
//! 直接 `0xC0000005` 段错误。
//!
//! 最容易踩的写法是**把 COM 调用写成函数的尾表达式**：
//!
//! ```ignore
//! fn exists(path: &str) -> bool {
//!     let _com = ComGuard::new();          // 先声明 → 最后析构
//!     let folder = get_folder(/* … */);
//!     unsafe { folder.GetTask(&path.into()) }.is_ok()   // ✗ 危险
//! }
//! ```
//!
//! 尾表达式里的临时 `Result<IRegisteredTask>` **不**在语句结束处析构，而是活到
//! **函数作用域末尾**——排在 `_com` 之后。于是 `Release()` 落在 `CoUninitialize()`
//! 之后，进程**启动即崩且没有任何输出**（发布版连控制台都没有，表现为"双击没反应"）。
//!
//! 而且它对代码布局敏感：随手加一行日志就可能让崩溃消失，看着像"已经修好了"。
//! **判别与正确写法**：凡是会在返回路径上携带 COM 对象的表达式，一律先落地成
//! 普通值（`bool` / `String` / 自己的结构体），并**显式 `drop`**：
//!
//! ```ignore
//! let task = unsafe { folder.GetTask(&path.into()) };
//! let found = task.is_ok();
//! drop(task);      // 必须先于 _com
//! drop(folder);
//! found
//! ```
//!
//! 语句形式（`let x = unsafe { … }?;`）是安全的——临时值在**语句**结束就释放了。

#[cfg(windows)]
mod imp {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

    /// COM 已初始化的凭证。离开作用域时自动反初始化。
    pub struct ComGuard {
        /// `false` 表示本线程在此之前已被别的模式初始化过。
        /// 这种情况下 COM **可以正常使用**，但**不允许**由我们调用 `CoUninitialize`
        /// （那会把调用方的初始化计数减掉，破坏它的假设）。
        owns_init: bool,
    }

    impl ComGuard {
        /// 在当前线程初始化 COM。
        ///
        /// 用 STA（`COINIT_APARTMENTTHREADED`）而不是 MTA，是因为
        /// `IShellLinkW` / `IShellItemImageFactory` 都是 Shell 组件，
        /// 在 STA 下行为最稳。如果当前线程已经是 MTA，`CoInitializeEx`
        /// 会返回 `RPC_E_CHANGED_MODE`——这不是错误，此时 COM 照样可用。
        pub fn new() -> Self {
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            let owns_init = hr.is_ok();

            if !owns_init && hr != windows::Win32::Foundation::RPC_E_CHANGED_MODE {
                // 真正罕见的失败（如 E_OUTOFMEMORY）。不 panic：
                // 调用方的 COM 调用会自己失败并降级，比整个扫描挂掉好。
                log::warn!("COM 初始化失败：{hr:?}，相关来源可能扫不到");
            }

            Self { owns_init }
        }
    }

    impl Default for ComGuard {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.owns_init {
                unsafe { CoUninitialize() };
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    /// 非 Windows 平台的空实现，让上层代码不需要 `cfg` 分支。
    #[derive(Default)]
    pub struct ComGuard;

    impl ComGuard {
        pub fn new() -> Self {
            Self
        }
    }
}

pub use imp::ComGuard;
