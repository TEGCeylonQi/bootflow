//! 文件元信息：数字签名、发布者、友好名称。
//!
//! 这个模块回答三个问题，全都是界面上要展示的：
//! 1. **有没有数字签名**（`WinVerifyTrust`）—— 未签名且位于全局位置是高危信号
//! 2. **发布者是谁**（版本信息 `CompanyName`）—— "Valve Corporation" 比
//!    `steam.exe` 有用得多
//! 3. **友好名称是什么**（`FileDescription` / `ProductName`）——
//!    这是"统一识别启动项名称"的一手来源，比让用户看 `localsend_app` 强
//!
//! 两个工程上的取舍值得说明：
//!
//! - **不读证书 CN，读版本信息 `CompanyName`**。证书里取 CN 要过
//!   `CryptQueryObject` + `CertFindCertificateInStore` + `CertGetNameStringW`
//!   三跳，代码量与出错面都大一个量级；而实测两者几乎总是一致
//!   （签名时用的组织名就是写进版本信息的那一个）。属性面板只展示"发布者"，
//!   不声称"这是证书主体"，因此不存在误导。
//!
//! - **带缓存**。同一台机器上几十个启动项常常指向同一个 exe
//!   （比如三个入口都是 steam.exe），每次重新验签会让首扫多花好几秒。
//!   缓存键是小写路径，进程生命周期内有效。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::model::{NameSource, SignerInfo};

/// 单个文件的完整元信息。
#[derive(Debug, Clone, Default)]
pub struct FileMeta {
    pub signer: SignerInfo,
    /// 直接可用的友好名（`FileDescription` 优先，其次 `ProductName`）
    pub display_name: Option<String>,
    pub name_from: Option<NameSource>,
}

/// 「这个路径没东西可读」的占位值，避免调用方处理 Option 链。
pub fn unknown() -> FileMeta {
    FileMeta::default()
}

fn cache() -> &'static Mutex<HashMap<String, FileMeta>> {
    static CACHE: OnceLock<Mutex<HashMap<String, FileMeta>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 读取文件元信息。**永不失败**——读不到就返回默认值。
///
/// 这是刻意的：签名信息是"锦上添花"的判断依据，
/// 因为某个 exe 的版本信息损坏就让整个扫描失败，是不划算的。
pub fn inspect(path: &str) -> FileMeta {
    let trimmed = path.trim().trim_matches('"').trim();
    if trimmed.is_empty() {
        return unknown();
    }

    let key = trimmed.replace('/', "\\").to_ascii_lowercase();

    if let Ok(guard) = cache().lock() {
        if let Some(hit) = guard.get(&key) {
            return hit.clone();
        }
    }

    let meta = inspect_uncached(trimmed);

    if let Ok(mut guard) = cache().lock() {
        guard.insert(key, meta.clone());
    }

    meta
}

fn inspect_uncached(path: &str) -> FileMeta {
    let is_os_component = is_os_component(path);

    // 文件不存在时没必要再走验签——那只会白花时间
    if !std::path::Path::new(path).is_file() {
        return FileMeta {
            signer: SignerInfo {
                is_os_component,
                ..Default::default()
            },
            display_name: None,
            name_from: None,
        };
    }

    let strings = version_strings(path);

    // 友好名可能"存在但不可用"——见 `is_placeholder_name`。
    //
    // `name_from` 必须跟着一起清掉：如果留着 `FileDescription` 而名字是 `None`，
    // 属性面板会显示"名字来自文件说明"却看不到名字，比没有更让人困惑。
    let (file_desc, product) = match strings.as_ref() {
        Some(s) => (
            s.file_description
                .as_deref()
                .filter(|v| !is_placeholder_name(v)),
            s.product_name.as_deref().filter(|v| !is_placeholder_name(v)),
        ),
        None => (None, None),
    };

    let display_name = file_desc.or(product).map(str::to_string);

    let name_from = if file_desc.is_some() {
        Some(NameSource::FileDescription)
    } else if product.is_some() {
        Some(NameSource::ProductName)
    } else {
        None
    };

    // 公司名同样可能是占位符。本机 ASUS 的
    // `TaskSchedulerTool_ArmourySocketServer.exe` 的 CompanyName 就是
    // `TODO: <檔案說明>`——整个 PE 的版本资源被填了同一个没翻译的字符串。
    //
    // 这种值与"没有公司名"在语义上完全等价：它回答不了"这是谁做的"。
    // 留在界面上就是一个谁也看不懂的字段，所以按"取不到"处理。
    // 注意这不影响 `is_microsoft` 的判断——占位符里不会有 "microsoft"。
    let publisher = strings
        .as_ref()
        .and_then(|s| s.company_name.as_deref())
        .filter(|p| !is_placeholder_name(p))
        .map(str::to_string);

    let (is_signed, cert_valid) = verify_signature(path);

    let is_microsoft = is_os_component
        || publisher
            .as_deref()
            .is_some_and(|p| p.to_ascii_lowercase().contains("microsoft"));

    FileMeta {
        signer: SignerInfo {
            is_signed,
            publisher,
            is_microsoft,
            cert_valid,
            is_os_component,
        },
        display_name,
        name_from,
    }
}

/// 版本信息里的字符串**存在但不能当名字用**时返回 true。
///
/// 这不是理论上的洁癖，是真机数据逼出来的：本机 ASUS 的任务目标
/// `ArmourySocketServer.exe`，它的 `FileDescription` 就是
/// `TODO: <檔案說明>`——开发时留下的占位符，随程序一起发布了。
/// 直接拿去当显示名，用户会在列表里看到一条叫「TODO: <檔案說明>」的启动项，
/// 比显示文件名 `ArmourySocketServer` 还糟。
///
/// 另一类常见形态是**资源引用没展开**：`$(@%SystemRoot%\system32\spaceman.exe,-2)`、
/// `@shell32.dll,-8964`。本机任务的作者字段里实测就有这种值。
fn is_placeholder_name(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return true;
    }

    let lower = t.to_ascii_lowercase();
    if lower.starts_with("todo:")
        || lower.starts_with("fixme:")
        || lower.starts_with("tbd:")
        || lower.starts_with("<未")
    {
        return true;
    }

    // 未展开的资源引用
    if t.starts_with('@') || t.starts_with("$(") {
        return true;
    }

    // 整串被尖括号包住（`<檔案說明>` 这种本身就是"这里该有个说明"的标记）
    t.starts_with('<') && t.ends_with('>')
}

/// 路径是否位于 Windows 系统目录之内。
///
/// **判定范围刻意收窄**：只看 `SystemRoot` 下的几个系统子目录，
/// 不含 `%SystemRoot%` 根目录本身。
///
/// 原因是根目录里既放 `explorer.exe` 这类真正的系统文件，也常被安装程序
/// 当作临时落地点。漏判前者几乎没有代价（系统文件本来就不会出现在启动项里），
/// 而误判后者的代价很实在——被划进"禁改区"意味着用户**根本没法管理它**。
///
/// 另有一个已知边界：`is_os_component` 只管**路径**，不管签名。
/// 因此 Win11 上被 Store 应用接管的执行别名（`System32\cmd.exe` 这类，
/// WinVerifyTrust 对其返回"无签名"）仍会被正确判为系统组件——
/// 展示层靠这个字段避免把微软的系统文件说成"未签名"。
pub fn is_os_component(path: &str) -> bool {
    let normalized = path.replace('/', "\\").to_ascii_lowercase();

    let root = std::env::var("SystemRoot")
        .unwrap_or_else(|_| r"C:\Windows".to_string())
        .replace('/', "\\")
        .to_ascii_lowercase();

    let prefix = format!("{root}\\");
    let Some(rest) = normalized.strip_prefix(&prefix) else {
        return false;
    };

    const SYSTEM_DIRS: &[&str] = &["system32", "syswow64", "winsxs", "servicing", "assembly"];

    SYSTEM_DIRS.iter().any(|d| {
        rest == *d || rest.starts_with(&format!("{d}\\"))
    })
}

/* ───────────────────── 版本信息 ───────────────────── */

#[derive(Debug, Default, Clone)]
struct VersionStrings {
    file_description: Option<String>,
    product_name: Option<String>,
    company_name: Option<String>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 从 PE 的版本资源里读文件说明、产品名与公司名。
///
/// 整个流程是「先问大小 → 分配缓冲 → 灌进去 → 查询」四步，
/// 任意一步失败都返回 `None`，由调用方回退到文件名。
#[cfg(windows)]
fn version_strings(path: &str) -> Option<VersionStrings> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW};

    let path_w = wide(path);

    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(path_w.as_ptr()), None);
        if size == 0 {
            return None;
        }

        let mut buffer = vec![0u8; size as usize];
        GetFileVersionInfoW(
            PCWSTR(path_w.as_ptr()),
            0,
            size,
            buffer.as_mut_ptr() as *mut core::ffi::c_void,
        )
        .ok()?;

        let block = buffer.as_ptr() as *const core::ffi::c_void;

        // 先问翻译表拿到「语言 + 代码页」，没有它拼不出 StringFileInfo 的路径。
        // 绝大多数程序是 0409（英文）或 0804（简体中文），但直接写死会读不到
        // 本地化过的版本信息——所以必须动态查。
        let translation = query_translation(block)?;
        if translation.len() < 4 {
            return None;
        }

        let lang = u16::from_le_bytes([translation[0], translation[1]]);
        let codepage = u16::from_le_bytes([translation[2], translation[3]]);
        let prefix = format!(r"\StringFileInfo\{lang:04x}{codepage:04x}\");

        Some(VersionStrings {
            file_description: query_string(block, &format!("{prefix}FileDescription")),
            product_name: query_string(block, &format!("{prefix}ProductName")),
            company_name: query_string(block, &format!("{prefix}CompanyName")),
        })
    }
}

/// `VerQueryValueW` 的薄封装，返回 `(缓冲指针, 长度)`。
///
/// ⚠️ **长度的单位取决于查询内容**，这是这个 API 最容易踩的坑
/// （MSDN 原文：字符串值返回"字符数"，非字符串值返回"字节数"）：
///
/// | 查询 | `pLen` 单位 |
/// |---|---|
/// | `\VarFileInfo\Translation` | 字节 |
/// | `\StringFileInfo\...\<字段>` | 字符 |
///
/// 把后者按字节解释，只会读到一半字符。实测的表现是
/// `VB-AUDIO Virtual Audio Device` 被截成 `VB-AUDIO Virtual Audio Devic`——
/// 看起来像"这个名字本来就这么长"，很难联想到是长度单位搞错了。
/// 这个 bug 只有在真机上对着**已知内容的文件**才能发现。
#[cfg(windows)]
unsafe fn ver_query(
    block: *const core::ffi::c_void,
    sub: &str,
) -> Option<(*mut core::ffi::c_void, u32)> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::VerQueryValueW;

    let sub_w = wide(sub);
    let mut ptr: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut len: u32 = 0;

    if VerQueryValueW(block, PCWSTR(sub_w.as_ptr()), &mut ptr, &mut len).as_bool() && !ptr.is_null()
    {
        Some((ptr, len))
    } else {
        None
    }
}

/// 查翻译表（语言 + 代码页）。长度按**字节**解释。
#[cfg(windows)]
unsafe fn query_translation(block: *const core::ffi::c_void) -> Option<Vec<u8>> {
    let (ptr, len) = ver_query(block, r"\VarFileInfo\Translation")?;
    Some(std::slice::from_raw_parts(ptr as *const u8, len as usize).to_vec())
}

/// 查 `StringFileInfo` 下的字符串值。长度按**字符**解释。
#[cfg(windows)]
unsafe fn query_string(block: *const core::ffi::c_void, sub: &str) -> Option<String> {
    let (ptr, len) = ver_query(block, sub)?;

    // len 是字符数，直接用它做 u16 切片的元素个数
    let units = std::slice::from_raw_parts(ptr as *const u16, len as usize);

    let s = String::from_utf16_lossy(units)
        .trim_end_matches('\0')
        .trim()
        .to_string();

    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(not(windows))]
fn version_strings(_path: &str) -> Option<VersionStrings> {
    None
}

/* ───────────────────── 数字签名 ───────────────────── */

/// `TRUST_E_NOSIGNATURE`：文件没有数字签名。
const TRUST_E_NOSIGNATURE: i32 = 0x800B0100u32 as i32;
/// 对非 PE 文件（.bat / .ps1 / .cmd）调 WinVerifyTrust 通常返回这个，
/// 语义上等同于"没有签名"，而不是"签名坏了"。
const TRUST_E_SUBJECT_FORM_UNKNOWN: i32 = 0x800B0003u32 as i32;
const TRUST_E_PROVIDER_UNKNOWN: i32 = 0x800B0001u32 as i32;

/// 只跑 `WinVerifyTrust`，返回原始状态码。
///
/// 拆出来是为了**可诊断**：验签失败时，"到底返回了什么"是唯一能定位问题的信息，
/// 而把它压缩成 bool 之后就只能靠猜。测试里也直接用它验证真实状态码。
#[cfg(windows)]
fn verify_raw(path: &str) -> i32 {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0,
        WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
        WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };

    let path_w = wide(path);

    unsafe {
        let mut file_info = WINTRUST_FILE_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: windows::core::PCWSTR(path_w.as_ptr()),
            ..Default::default()
        };

        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            // 不查吊销列表：那需要联网，会让每次扫描卡上数秒。
            // 对"这个程序可不可信"的判断而言，签名链是否完整已经足够。
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 {
                pFile: &mut file_info,
            },
            dwStateAction: WTD_STATEACTION_VERIFY,
            ..Default::default()
        };

        // 这个 GUID 在 API 签名里是非 const 指针，所以得有可变副本。
        // 它只是个动作标识，内容不会被改写。
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;

        let status = WinVerifyTrust(
            HWND::default(),
            &mut action,
            &mut data as *mut _ as *mut core::ffi::c_void,
        );

        // 无论结果如何都必须发一次 CLOSE，否则会泄漏状态数据
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(
            HWND::default(),
            &mut action,
            &mut data as *mut _ as *mut core::ffi::c_void,
        );

        status
    }
}

/* ───────────────────── catalog 签名 ───────────────────── */

/// 超过这个大小的文件跳过 catalog 验签。
///
/// 算哈希要读整个文件——对一个几 GB 的镜像文件，这会让扫描卡上几十秒，
/// 而启动项的目标几乎不可能是那种体积。宁可对超大文件退回"未签名"，
/// 也不能让首扫变成几分钟。
const MAX_CATALOG_HASH_BYTES: u64 = 256 * 1024 * 1024;

/// catalog 签名验证。
///
/// ## 为什么必须做这一步
///
/// `WinVerifyTrust` 配 `WTD_CHOICE_FILE` 只检查**内嵌**签名。而 Windows
/// 自带的程序绝大多数**不内嵌签名**——它们的签名在 `CatRoot` 目录的
/// `.cat` 文件里（catalog 签名）。本机实测：
///
/// ```text
/// kernel32.dll   0x00000000  通过（有内嵌签名）
/// notepad.exe    0x800B0100  未通过
/// cmd.exe        0x800B0100  未通过
/// ctfmon.exe     0x800B0100  未通过
/// rundll32.exe   0x800B0100  未通过
/// ```
///
/// `0x800B0100 = TRUST_E_NOSIGNATURE`。也就是说在系统目录里 `is_signed`
/// **恒定失真**：它分不出"没签名"和"签名不内嵌"。
///
/// 后果很具体——界面上会把 `ctfmon.exe` 这类系统文件报成"无法核验发布者"。
/// 用户看到 Windows 自己的程序"没签名"，最合理的反应是不再相信这个工具
/// 的任何判断，而那正是它最不能失去的东西。
///
/// ## 流程
///
/// ```text
///   CreateFileW                            打开文件（算哈希要句柄）
///      ↓
///   CryptCATAdminAcquireContext2           取 catalog 管理器，用 SHA-256
///      ↓
///   CryptCATAdminCalcHashFromFileHandle2   算文件哈希
///      ↓
///   CryptCATAdminEnumCatalogFromHash       按哈希找包含它的 .cat
///      ↓
///   CryptCATCatalogInfoFromContext         取 .cat 的路径
///      ↓
///   WinVerifyTrust(WTD_CHOICE_CATALOG)     验证整条签名链
/// ```
///
/// ## 两个刻意的选择
///
/// **用 SHA-256 而不是默认的 SHA-1。** 旧 API（`CryptCATAdminAcquireContext`）
/// 固定用 SHA-1，而微软早已迁移到 SHA-256，新系统的 catalog 用 SHA-1
/// 去找会**找不到**——于是"有签名"被静默判成"没签名"，错得和现在
/// 一样，只是更难发现。
///
/// **任何一步失败都返回 `false`。** 这个函数只回答"有没有可验证的
/// catalog 签名"，不 panic、不返回错误码——它会被几百个启动项连续调用，
/// 一个 panic 就是整次扫描崩掉。
#[cfg(windows)]
fn verify_via_catalog(path: &str) -> bool {
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, HWND};
    use windows::Win32::Security::Cryptography::Catalog::{
        CryptCATAdminAcquireContext2, CryptCATAdminCalcHashFromFileHandle2,
        CryptCATAdminEnumCatalogFromHash, CryptCATAdminReleaseCatalogContext,
        CryptCATAdminReleaseContext, CryptCATCatalogInfoFromContext, CATALOG_INFO,
    };
    use windows::Win32::Security::Cryptography::BCRYPT_SHA256_ALGORITHM;
    use windows::Win32::Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_CATALOG_INFO, WINTRUST_DATA,
        WINTRUST_DATA_0, WTD_CHOICE_CATALOG, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
        WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, OPEN_EXISTING,
    };

    // ── 三个 RAII 守卫 ──
    //
    // 声明顺序即释放顺序的**逆序**（Rust 保证），所以：
    //   file → cat_admin → cat_info  （声明）
    //   释放时 cat_info → cat_admin → file
    // 这个顺序是必须的：释放 catalog 上下文时要用到 admin 句柄。
    struct FileGuard(HANDLE);
    impl Drop for FileGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    struct CatAdminGuard(isize);
    impl Drop for CatAdminGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CryptCATAdminReleaseContext(self.0, 0);
            }
        }
    }

    struct CatInfoGuard {
        admin: isize,
        info: isize,
    }
    impl Drop for CatInfoGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CryptCATAdminReleaseCatalogContext(self.admin, self.info, 0);
            }
        }
    }

    // 大文件直接放弃：算哈希要读全文件
    if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) > MAX_CATALOG_HASH_BYTES {
        log::debug!("文件过大，跳过 catalog 验签：{path}");
        return false;
    }

    let path_w = wide(path);

    unsafe {
        // 1. 打开文件。共享读——不能挡住正在运行的程序
        //    （explorer.exe、svchost.exe 都在跑着）。
        let Ok(file) = CreateFileW(
            windows::core::PCWSTR(path_w.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            HANDLE::default(),
        ) else {
            return false;
        };
        let file = FileGuard(file);

        // 2. catalog 管理器（SHA-256）
        let mut cat_admin: isize = 0;
        if CryptCATAdminAcquireContext2(&mut cat_admin, None, BCRYPT_SHA256_ALGORITHM, None, 0)
            .is_err()
        {
            return false;
        }
        let cat_admin = CatAdminGuard(cat_admin);

        // 3. 文件哈希：先问长度，再取内容。
        //    第一次调用必然"失败"（缓冲区不足），所以只看长度。
        let mut hash_len: u32 = 0;
        let _ =
            CryptCATAdminCalcHashFromFileHandle2(cat_admin.0, file.0, &mut hash_len, None, 0);
        if hash_len == 0 {
            return false;
        }

        let mut hash = vec![0u8; hash_len as usize];
        if CryptCATAdminCalcHashFromFileHandle2(
            cat_admin.0,
            file.0,
            &mut hash_len,
            Some(hash.as_mut_ptr()),
            0,
        )
        .is_err()
        {
            return false;
        }
        hash.truncate(hash_len as usize);

        // 4. 按哈希找 catalog。返回 0 表示没有任何 .cat 收录这个文件
        //    ——这是"确实没签名"，不是错误。
        let cat_ctx = CryptCATAdminEnumCatalogFromHash(cat_admin.0, &hash, 0, None);
        if cat_ctx == 0 {
            return false;
        }
        let cat_guard = CatInfoGuard {
            admin: cat_admin.0,
            info: cat_ctx,
        };

        // 5. catalog 文件路径
        let mut cat_info = CATALOG_INFO {
            cbStruct: std::mem::size_of::<CATALOG_INFO>() as u32,
            ..Default::default()
        };
        if CryptCATCatalogInfoFromContext(cat_guard.info, &mut cat_info, 0).is_err() {
            return false;
        }

        let cat_path: Vec<u16> = cat_info
            .wszCatalogFile
            .iter()
            .copied()
            .take_while(|&c| c != 0)
            .chain(std::iter::once(0))
            .collect();
        if cat_path.len() <= 1 {
            return false;
        }

        // 6. member tag = 文件哈希的十六进制大写字符串。
        //    这是 catalog 里标识"这个文件"的键。
        let tag: String = hash.iter().map(|b| format!("{b:02X}")).collect();
        let tag_w = wide(&tag);

        // 7. 验签。`pcCatalogContext` 传 NULL 是可以的——
        //    WinVerifyTrust 会按 `pcwszCatalogFilePath` 自己去加载 catalog。
        let mut catalog = WINTRUST_CATALOG_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_CATALOG_INFO>() as u32,
            dwCatalogVersion: 0,
            pcwszCatalogFilePath: windows::core::PCWSTR(cat_path.as_ptr()),
            pcwszMemberTag: windows::core::PCWSTR(tag_w.as_ptr()),
            pcwszMemberFilePath: windows::core::PCWSTR(path_w.as_ptr()),
            hMemberFile: file.0,
            pbCalculatedFileHash: hash.as_mut_ptr(),
            cbCalculatedFileHash: hash_len,
            pcCatalogContext: std::ptr::null_mut(),
            hCatAdmin: cat_admin.0,
        };

        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_CATALOG,
            Anonymous: WINTRUST_DATA_0 {
                pCatalog: &mut catalog,
            },
            dwStateAction: WTD_STATEACTION_VERIFY,
            ..Default::default()
        };

        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;

        let status = WinVerifyTrust(
            HWND::default(),
            &mut action,
            &mut data as *mut _ as *mut core::ffi::c_void,
        );

        // 无论结果如何都必须 CLOSE，否则会泄漏状态数据
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = WinVerifyTrust(
            HWND::default(),
            &mut action,
            &mut data as *mut _ as *mut core::ffi::c_void,
        );

        if status != 0 {
            log::debug!("catalog 验签未通过：{path} → 0x{:08X}", status as u32);
        }

        status == 0
    }
}

#[cfg(not(windows))]
fn verify_via_catalog(_path: &str) -> bool {
    false
}

/// 返回 `(is_signed, cert_valid)`。
///
/// 三态区分很重要：
/// - 验签成功 → 有签名、证书可信
/// - 明确"无签名" → 无签名、没有证书可校验
/// - 其他失败（证书过期 / 吊销 / 链不完整）→ **有**签名、证书不可信
///
/// 把第三种误报成"无签名"会让用户以为程序没签名，
/// 而实际上它只是证书过期——这是两种完全不同的处置方式。
///
/// ⚠️ 内嵌验签失败**不等于没签名**，所以还要走一次 catalog。
/// 系统目录里的文件几乎全部依赖这一步，漏掉它等于 `is_signed`
/// 在系统文件上恒定说谎。见 `verify_via_catalog` 的注释。
#[cfg(windows)]
fn verify_signature(path: &str) -> (bool, Option<bool>) {
    let status = verify_raw(path);

    if status == 0 {
        return (true, Some(true));
    }

    // 「无签名」这一族状态码才需要继续查 catalog。
    // 其他失败（证书过期等）说明签名是存在的，只是不可信——
    // 那种情况下去查 catalog 也没意义。
    let looks_unsigned = status == TRUST_E_NOSIGNATURE
        || status == TRUST_E_SUBJECT_FORM_UNKNOWN
        || status == TRUST_E_PROVIDER_UNKNOWN;

    if looks_unsigned {
        if verify_via_catalog(path) {
            log::debug!("内嵌验签失败但 catalog 验签通过：{path}");
            return (true, Some(true));
        }
        return (false, None);
    }

    // 有签名但信任链有问题（过期、吊销、根证书缺失）
    log::debug!("验签未通过：{} → 0x{:08X}", path, status as u32);
    (true, Some(false))
}

#[cfg(not(windows))]
fn verify_signature(_path: &str) -> (bool, Option<bool>) {
    (false, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system32_is_os_component() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        assert!(is_os_component(&format!(r"{root}\System32\notepad.exe")));
        assert!(is_os_component(&format!(r"{root}\SysWOW64\cmd.exe")));
    }

    #[test]
    fn program_files_is_not_os_component() {
        assert!(!is_os_component(r"C:\Program Files\SomeApp\app.exe"));
        assert!(!is_os_component(r"D:\Games\steam.exe"));
    }

    #[test]
    fn windows_root_itself_is_not_os_component() {
        // %SystemRoot% 根目录下也有第三方程序，不能一律算系统组件
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        assert!(!is_os_component(&format!(r"{root}\Temp\setup.exe")));
    }

    #[test]
    fn empty_path_yields_default_meta() {
        let meta = inspect("   ");
        assert!(!meta.signer.is_signed);
        assert!(meta.display_name.is_none());
    }

    #[test]
    fn missing_file_does_not_pretend_to_be_signed() {
        let meta = inspect(r"D:\__bootflow_missing__\nope.exe");
        assert!(!meta.signer.is_signed);
        assert!(!meta.signer.is_os_component);
    }

    /// 诊断用：把一批系统文件的真实状态码打出来。
    ///
    /// 这个测试的价值在于**失败时给得出信息**——验签这条链路的失败原因
    /// （文件不存在 / 无签名 / 证书链问题）在布尔值上完全看不出来。
    #[test]
    fn report_verify_status_of_system_binaries() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());

        // 前两个是**执行别名**（Win11 上 notepad / cmd 已被 Store 应用接管），
        // 剩下两个是稳定的传统 PE。对照组放在一起，一眼能看出差异来自文件本身
        // 而不是调用方式。
        //
        // ctfmon / rundll32 是新加的：它们出现在 Run 键里，
        // 而 ctfmon 在界面上被判成了"系统组件却未签名"——需要查清是文件的问题
        // 还是我们验签的问题。
        for rel in [
            r"System32\notepad.exe",
            r"System32\cmd.exe",
            r"System32\ctfmon.exe",
            r"System32\rundll32.exe",
            r"System32\kernel32.dll",
            r"explorer.exe",
        ] {
            let path = format!("{root}\\{rel}");
            let exists = std::path::Path::new(&path).is_file();
            let status = verify_raw(&path);
            let meta = inspect(&path);

            println!(
                "{rel:26} exists={exists:<5} status=0x{status:08X} signed={:<5} ms={:<5} os={:<5} pub={:?}",
                meta.signer.is_signed,
                meta.signer.is_microsoft,
                meta.signer.is_os_component,
                meta.signer.publisher.as_deref().unwrap_or("-"),
            );
        }

        // 传统 PE 必须能通过验签——这是"发布者可信"这条判断的地基。
        // 若这里失败，说明验签调用本身有问题，而不是文件有问题。
        let explorer = format!(r"{root}\explorer.exe");
        assert_eq!(
            verify_raw(&explorer),
            0,
            "explorer.exe 验签应返回 0（S_OK）"
        );
    }

    #[test]
    fn system_binaries_are_reported_as_signed() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());

        // ⚠️ 基准文件必须选**传统 PE**，不能用 notepad.exe / cmd.exe：
        // Win11 上这两个已被 Store 应用接管，`System32` 下留下的是执行别名
        // （reparse point）。WinVerifyTrust 对别名返回 TRUST_E_NOSIGNATURE，
        // 拿它们当基准会把"验签正确"误判成"验签坏了"。
        for rel in [r"System32\kernel32.dll", r"explorer.exe"] {
            let path = format!("{root}\\{rel}");
            let meta = inspect(&path);

            assert!(
                meta.signer.is_signed,
                "{rel} 应有有效签名（status=0x{:08X}）",
                verify_raw(&path)
            );
            assert!(meta.signer.is_microsoft, "{rel} 应识别出微软发布者");
        }
    }

    #[test]
    fn os_component_detection_is_scoped_to_system_subdirs() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());

        assert!(
            is_os_component(&format!(r"{root}\System32\kernel32.dll")),
            "System32 内应判为系统组件"
        );

        // %SystemRoot% 根目录**不在**判定范围内：那里既放 explorer.exe 这类
        // 真正的系统文件，也常被安装程序当作临时落地点（Windows\Temp 之类）。
        // 宁可漏判少数几个（它们本来也不会出现在启动项里），
        // 也不要把第三方程序误划进"禁改区"——那会让用户根本没法管理它。
        assert!(
            !is_os_component(&format!(r"{root}\explorer.exe")),
            "Windows 根目录不在系统组件判定范围内"
        );
    }

    /// 锁住 `VerQueryValueW` 的长度单位 bug。
    ///
    /// 字符串值的 `pLen` 是**字符数**而不是字节数。按字节解释会只读到一半字符，
    /// 而这个 bug 的表现极其隐蔽——读出来的半个单词看起来就像"名字本来就这么长"。
    /// 实测时就漏过了：`VB-AUDIO Virtual Audio Device` 被静默截成
    /// `VB-AUDIO Virtual Audio Devic`，直到对照真实机器输出才发现。
    #[test]
    fn placeholder_display_names_are_rejected() {
        // 真机数据：ASUS ArmourySocketServer.exe 的"文件说明"就是这一串
        assert!(is_placeholder_name("TODO: <檔案說明>"));
        assert!(is_placeholder_name("todo: <file description>"));
        assert!(is_placeholder_name("<檔案說明>"));
        // 未展开的资源引用（本机任务作者字段里真实出现过）
        assert!(is_placeholder_name(
            "$(@%SystemRoot%\\system32\\spaceman.exe,-2)"
        ));
        assert!(is_placeholder_name("@shell32.dll,-8964"));
        assert!(is_placeholder_name("   "));
        assert!(is_placeholder_name("FIXME: not localized"));

        // 正常名字不能被误杀——这是这套过滤存在的意义所在
        assert!(!is_placeholder_name("Microsoft Office Click-to-Run Client"));
        assert!(!is_placeholder_name("存储空间设置"));
        assert!(!is_placeholder_name("PowerToys.Runner"));
        assert!(!is_placeholder_name("Clash Verge"));
        // 尖括号只出现在中间就是正常名字，不该连坐
        assert!(!is_placeholder_name("Client <-> Server Bridge"));
    }

    #[test]
    fn version_description_is_not_truncated() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let meta = inspect(&format!(r"{root}\System32\kernel32.dll"));

        let desc = meta
            .display_name
            .expect("kernel32.dll 应能读到 FileDescription");

        println!("kernel32.dll FileDescription = {desc:?}（{} 字符）", desc.chars().count());

        // 这个文件在所有 Windows 上的描述都以 "DLL" 结尾。
        // 若长度单位处理错，末尾会变成 "D"——正好被这条断言抓住。
        assert!(
            desc.to_ascii_lowercase().ends_with("dll"),
            "描述被截断了，实际读到：{desc:?}"
        );
    }

    /// catalog 验签的回归锁。
    ///
    /// 修复前，系统目录里不内嵌签名的文件一律被判 `is_signed = false`，
    /// 界面上会把 Windows 自己的程序报成"无法核验发布者"——
    /// 用户看到这种结论，最合理的反应是不再相信这个工具的任何判断。
    #[test]
    fn system_binaries_are_verified_via_catalog() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());

        // 这两个是**传统 PE 且不内嵌签名**：修复前它们必然失败，
        // 因此是验证 catalog 这条链路是否真的通了的最佳样本。
        // （notepad / cmd 不能用：Win11 上它们是 Store 应用的执行别名。）
        let mut checked = 0;

        for rel in [r"System32\ctfmon.exe", r"System32\rundll32.exe"] {
            let path = format!("{root}\\{rel}");
            if !std::path::Path::new(&path).is_file() {
                continue;
            }

            let embedded = verify_raw(&path);
            let (signed, cert_valid) = verify_signature(&path);

            println!(
                "{rel:26} 内嵌验签=0x{embedded:08X} → is_signed={signed} cert_valid={cert_valid:?}"
            );

            assert!(
                signed,
                "{rel} 是微软签名的系统文件（内嵌验签 0x{embedded:08X}）。\
                 判成未签名等于对用户说了一句与事实相反的话。"
            );
            assert_eq!(
                cert_valid,
                Some(true),
                "catalog 签名有效时，证书可信度也应是肯定的"
            );
            checked += 1;
        }

        assert!(checked > 0, "本机没有可用于验证的传统 PE 样本");
    }

    /// 反面：catalog 验签不能退化成"什么都算通过"。
    ///
    /// 没有这条测试，把 `verify_via_catalog` 写成 `return true` 也能过上一关——
    /// 而那会让所有未签名的第三方程序都显示成"已签名"，
    /// 比修复前的问题更严重。
    #[test]
    fn catalog_verification_rejects_unsigned_files() {
        let tmp = std::env::temp_dir().join("bootflow_unsigned_probe.exe");
        std::fs::write(&tmp, b"MZ\x90\x00 this is not a signed binary").ok();

        let via_catalog = verify_via_catalog(tmp.to_str().unwrap());
        let (signed, _) = verify_signature(tmp.to_str().unwrap());

        let _ = std::fs::remove_file(&tmp);

        assert!(!via_catalog, "随手造的假 PE 不该通过 catalog 验签");
        assert!(!signed, "整条链路的结论也必须是「未签名」");
    }

    #[test]
    fn catalog_verification_handles_missing_files() {
        // 目标已被删除的启动项很常见，这条路径不能 panic
        assert!(!verify_via_catalog(r"D:\__bootflow_missing__\ghost.exe"));
    }
}
