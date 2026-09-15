//! 一次性 HTTPS 读取 —— 基于系统自带的 WinHTTP。
//!
//! 【为什么不用 reqwest / ureq 这类 HTTP 客户端】
//! 检查更新要做的事只有一件：发一个 GET、拿回一段文本。为这一件事引入
//! hyper + TLS 那一整套，会让发布包凭空胖一圈，也多出一串需要长期跟着升级的依赖。
//!
//! WinHTTP 是系统自带组件，而且有两个第三方客户端给不了的好处：
//!
//! 1. **默认吃系统代理设置**（`WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY`）。
//!    对需要走代理才能访问 GitHub 的用户，这一点最省心——用户已经在系统里
//!    配好的代理，我们自动就用上了，不用他再教一遍这个程序。
//! 2. **默认用系统根证书**。第三方客户端通常自带一份根证书快照，
//!    在证书链有差异的机器上会直接握手失败。
//!
//! 【只支持 https，且不跟随重定向】
//! 检查更新只访问一个固定的 API 地址，不需要 http 降级，也不需要跟随跳转——
//! 拒绝这两件事，等于免费砍掉一整类"被中间人改道"的可能。

use windows::core::PCWSTR;
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE,
};

use crate::error::{AppError, Result};

/// 单次响应体上限。
///
/// 检查更新只拿几十 KB 的 JSON，超过这个量级说明对面返回的不是我们预期的东西，
/// 与其闷头读进内存，不如当场停下。
const MAX_BODY: usize = 512 * 1024;

/// 一次读取的缓冲区大小
const CHUNK: usize = 8 * 1024;

pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// 读取一个 https 地址的文本内容。
///
/// 网络层失败与 HTTP 层失败都会返回 `Err`，且文案已经是人话——
/// 这类错误最终会原样展示给用户，不适合把 HRESULT 丢出去。
pub fn get_text(url: &str, user_agent: &str) -> Result<HttpResponse> {
    let target = HttpsUrl::parse(url)?;
    unsafe { fetch(&target, user_agent) }
}

// ─────────────────────────────────────────────────────────────
// URL 解析（只认 https）
// ─────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
struct HttpsUrl {
    host: String,
    port: u16,
    /// 含开头的 `/`，默认 "/"
    path: String,
}

impl HttpsUrl {
    fn parse(raw: &str) -> Result<Self> {
        let rest = raw
            .strip_prefix("https://")
            .ok_or_else(|| AppError::Other(format!("只允许 https 地址，收到：{raw}")))?;

        // 主机名到第一个 `/` 为止；`/` 之后（含 `?`、`#`）整段都是路径，
        // 交给 WinHTTP 原样发出，我们不去理解它
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };

        if authority.is_empty() {
            return Err(AppError::Other(format!("地址里没有主机名：{raw}")));
        }

        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() => {
                let port: u16 = p
                    .parse()
                    .map_err(|_| AppError::Other(format!("端口号不合法：{p}")))?;
                (h.to_string(), port)
            }
            _ => (authority.to_string(), 443),
        };

        if host.is_empty() {
            return Err(AppError::Other(format!("地址里没有主机名：{raw}")));
        }

        Ok(Self {
            host,
            port,
            path: path.to_string(),
        })
    }
}

// ─────────────────────────────────────────────────────────────
// WinHTTP 调用
// ─────────────────────────────────────────────────────────────

/// 包一层句柄生命周期管理。
///
/// WinHTTP 的句柄必须逐个 `WinHttpCloseHandle`，中间任何一步失败都会提前返回；
/// 靠手写 `goto cleanup` 那种写法迟早会漏。用 RAII 让 Drop 兜底，
/// 中途无论从哪一行退出都不会泄漏内核对象。
struct Handle(*mut core::ffi::c_void);

impl Handle {
    fn new(raw: *mut core::ffi::c_void, what: &str) -> Result<Self> {
        if raw.is_null() {
            Err(AppError::Other(format!("{what}失败")))
        } else {
            Ok(Self(raw))
        }
    }

    fn raw(&self) -> *mut core::ffi::c_void {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 把 `windows` crate 的 `Result` 换成带中文上下文的 `AppError`
fn ctx<T>(r: windows::core::Result<T>, what: &str) -> Result<T> {
    r.map_err(|e| AppError::Win32(format!("{what}：{e}")))
}

unsafe fn fetch(target: &HttpsUrl, user_agent: &str) -> Result<HttpResponse> {
    let ua = wide(user_agent);
    let host = wide(&target.host);
    let path = wide(&target.path);
    let verb = wide("GET");

    // ⚠️ WinHttpOpen / WinHttpConnect / WinHttpOpenRequest 这一族在绑定里
    // 直接返回句柄指针，失败时是空指针而不是 Result，判空统一交给 Handle::new。
    let session = Handle::new(
        WinHttpOpen(
            PCWSTR(ua.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ),
        "初始化 WinHTTP",
    )?;

    // 超时全部压到十秒内：检查更新是后台的顺手动作，绝不能让界面等它
    ctx(
        WinHttpSetTimeouts(session.raw(), 5_000, 5_000, 8_000, 10_000),
        "设置网络超时",
    )?;

    let connect = Handle::new(
        WinHttpConnect(session.raw(), PCWSTR(host.as_ptr()), target.port, 0),
        "连接服务器",
    )?;

    let request = Handle::new(
        WinHttpOpenRequest(
            connect.raw(),
            PCWSTR(verb.as_ptr()),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ),
        "创建请求",
    )?;

    ctx(
        WinHttpSendRequest(request.raw(), None, None, 0, 0, 0),
        "发送请求",
    )?;

    ctx(
        WinHttpReceiveResponse(request.raw(), std::ptr::null_mut()),
        "等待服务器响应",
    )?;

    let status = read_status(request.raw())?;
    let body = read_body(request.raw())?;

    Ok(HttpResponse { status, body })
}

unsafe fn read_status(request: *mut core::ffi::c_void) -> Result<u16> {
    let mut code: u32 = 0;
    let mut len = std::mem::size_of::<u32>() as u32;

    ctx(
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut code as *mut u32 as *mut core::ffi::c_void),
            &mut len,
            std::ptr::null_mut(),
        ),
        "读取响应状态码",
    )?;

    Ok(code as u16)
}

unsafe fn read_body(request: *mut core::ffi::c_void) -> Result<String> {
    let mut buf = vec![0u8; CHUNK];
    let mut raw: Vec<u8> = Vec::with_capacity(CHUNK);

    loop {
        let mut read: u32 = 0;
        ctx(
            WinHttpReadData(
                request,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len() as u32,
                &mut read,
            ),
            "读取响应内容",
        )?;

        // 读到 0 字节 = 服务器已经把内容发完了
        if read == 0 {
            break;
        }

        raw.extend_from_slice(&buf[..read as usize]);

        if raw.len() > MAX_BODY {
            return Err(AppError::Other(format!(
                "响应内容超过 {} KB 上限，已中止读取",
                MAX_BODY / 1024
            )));
        }
    }

    // GitHub 的响应都是 UTF-8；真出现坏字节也只是说明内容不对，
    // 让后续的 JSON 解析去报错，比在这里直接失败更有信息量
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_and_default_port() {
        let u = HttpsUrl::parse("https://api.github.com/repos/a/b/releases/latest").unwrap();
        assert_eq!(u.host, "api.github.com");
        assert_eq!(u.port, 443);
        assert_eq!(u.path, "/repos/a/b/releases/latest");
    }

    #[test]
    fn parses_explicit_port_and_missing_path() {
        let u = HttpsUrl::parse("https://example.com:8443").unwrap();
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, 8443);
        assert_eq!(u.path, "/");
    }

    #[test]
    fn keeps_query_string_in_path() {
        let u = HttpsUrl::parse("https://example.com/a?b=c").unwrap();
        assert_eq!(u.path, "/a?b=c");
    }

    #[test]
    fn rejects_plain_http() {
        // 明文 http 会让"下载地址"有机会被改道，直接不接受
        assert!(HttpsUrl::parse("http://api.github.com/x").is_err());
    }

    #[test]
    fn rejects_empty_host() {
        assert!(HttpsUrl::parse("https:///path").is_err());
    }

    #[test]
    fn rejects_bad_port() {
        assert!(HttpsUrl::parse("https://example.com:abc/").is_err());
    }
}
