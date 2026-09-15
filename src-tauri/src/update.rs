//! 更新检测 —— 问一问 GitHub：「有没有比我手上这个更新的版本」。
//!
//! 【为什么只做"检测"，不做"自动更新"】
//! Tauri 官方的 updater 插件能自己下载并替换安装包，代价是要长期保管一对
//! 签名密钥：私钥丢了，老版本的用户就再也收不到更新；私钥泄露，任何人都能
//! 往这条更新链路上塞东西。而它真正省下的，只是"用户点一下下载"这一步。
//!
//! 现阶段把"有没有新版、新在哪、去哪儿下载"讲清楚就够用了，链路更短、更可控。
//! 等发布节奏稳定下来，再评估是否升级成自动更新。
//!
//! 【诚实原则的延伸】
//! **检查失败绝不等于"已是最新"。** 网络不通、被墙、接口限流都会让请求失败；
//! 这种情况必须如实回报"没能检查成功"并说明原因，而不是悄悄显示"最新版"——
//! 那会让人以为功能正常，实际上他永远收不到更新提示。

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::util::http;

/// 项目仓库。检查更新只认这一个源头。
pub const REPO: &str = "TEGCeylonQi/bootflow";

/// 更新说明最多带回多少字符。Release 正文可能很长，完整内容留给发布页。
const NOTES_LIMIT: usize = 1500;

/// 当前编译进程序的版本号，源头是 `src-tauri/Cargo.toml`。
///
/// 用编译期常量而不是运行时读文件：程序跑起来之后，它就是"这个二进制自己的版本"，
/// 不会因为同目录下换了别的文件而说谎。
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ─────────────────────────────────────────────────────────────
// 版本号
// ─────────────────────────────────────────────────────────────

/// 语义化版本号。
///
/// 只需要比较大小，所以只解析 semver 里影响排序的部分：
/// 主次修订三段数字 + 预发布标识（`1.0.0-beta.2` 里的 `beta.2`）。
/// 构建元数据（`+build.5`）不影响排序，直接丢掉。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// 有预发布标识 = 比同号段的正式版旧（`1.0.0-beta` < `1.0.0`）
    pub pre: Option<String>,
}

impl Version {
    /// 解析 `v0.1.1` / `0.1.1` / `1.0.0-beta.2` 这类写法。
    ///
    /// 解析不了就返回 `None` —— 调用方据此判定"没法比较"，
    /// 而不是硬凑一个结果出来。缺省段按 0 处理（`v1` 等价于 `1.0.0`）。
    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let s = s.split('+').next().unwrap_or(s);

        let (numbers, pre) = match s.split_once('-') {
            Some((n, p)) => (n, Some(p.trim().to_string())),
            None => (s, None),
        };

        let mut parts = numbers.split('.');
        let major: u64 = parts.next()?.trim().parse().ok()?;
        let minor: u64 = parts.next().unwrap_or("0").trim().parse().ok()?;
        let patch: u64 = parts.next().unwrap_or("0").trim().parse().ok()?;

        Some(Self {
            major,
            minor,
            patch,
            pre: pre.filter(|p| !p.is_empty()),
        })
    }

    /// `self` 是否比 `other` 新。
    pub fn is_newer_than(&self, other: &Self) -> bool {
        use std::cmp::Ordering;

        match (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch)) {
            Ordering::Equal => {}
            ord => return ord == Ordering::Greater,
        }

        // 数字段相同时：没有预发布标识的更大
        match (&self.pre, &other.pre) {
            (None, None) => false,
            (None, Some(_)) => true,
            (Some(_), None) => false,
            (Some(a), Some(b)) => cmp_pre(a, b) == Ordering::Greater,
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

/// 预发布标识之间的比较（semver 第 11 条）：
/// 按 `.` 分段，纯数字段按数值比，其它按字典序；数字段比字母段小；
/// 段数少的更小（`beta` < `beta.1`）。
fn cmp_pre(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let mut left = a.split('.');
    let mut right = b.split('.');

    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(nx), Ok(ny)) => nx.cmp(&ny),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────
// 对外契约
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateStatus {
    /// 已是最新
    UpToDate,
    /// 有新版本
    Available,
    /// 没能检查成功（网络、限流、数据格式……）
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub status: UpdateStatus,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub release_name: Option<String>,
    pub release_url: Option<String>,
    pub published_at: Option<String>,
    /// Release 正文（Markdown），已截断
    pub notes: Option<String>,
    pub assets: Vec<ReleaseAsset>,
    /// 检查失败的原因，人话；`status == Failed` 时一定有
    pub reason: Option<String>,
    pub checked_at: String,
}

impl UpdateCheck {
    fn failed(reason: impl Into<String>) -> Self {
        Self {
            status: UpdateStatus::Failed,
            current_version: current_version().to_string(),
            latest_version: None,
            release_name: None,
            release_url: None,
            published_at: None,
            notes: None,
            assets: Vec::new(),
            reason: Some(reason.into()),
            checked_at: now(),
        }
    }
}

/// 检查更新。**任何失败都不返回 `Err`**，而是变成 `status = Failed` 的结果——
/// 检查更新是后台顺手做的事，不该以异常的形式砸到界面上。
pub fn check() -> UpdateCheck {
    match fetch_latest_release() {
        Ok(release) => classify(release),
        Err(e) => UpdateCheck::failed(e.to_string()),
    }
}

// ─────────────────────────────────────────────────────────────
// 打开下载页：域名白名单
// ─────────────────────────────────────────────────────────────

/// 放行 `url` 交给系统浏览器，前提是它指向 GitHub 自家。
///
/// 这些地址来自 GitHub 接口的返回值。正常情况下它们一定在 github.com 下，
/// 但"正常情况下"不足以作为"把任意字符串交给 shell"的理由——
/// 接口返回值理论上也可能被改道，而这一步的代价只是几行判断。
pub fn ensure_trusted_url(url: &str) -> Result<()> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| AppError::Other(format!("只允许打开 https 地址：{url}")))?;

    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // 去掉可能的 `user@` 与端口
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();

    let trusted = host == "github.com"
        || host.ends_with(".github.com")
        || host == "githubusercontent.com"
        || host.ends_with(".githubusercontent.com");

    if !trusted {
        return Err(AppError::Other(format!(
            "拒绝打开非 GitHub 的地址：{host}"
        )));
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────
// 与 GitHub 打交道
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

fn fetch_latest_release() -> Result<GhRelease> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = http::get_text(&url, &user_agent())?;

    match resp.status {
        200..=299 => {}
        404 => {
            return Err(AppError::Other(
                "GitHub 上还没有可用的发布版本".to_string(),
            ))
        }
        // 未登录调用 GitHub 接口每小时只有 60 次配额，打满就是这个码
        403 | 429 => {
            return Err(AppError::Other(
                "GitHub 接口的访问次数已超限，请过一会儿再试".to_string(),
            ))
        }
        500..=599 => {
            return Err(AppError::Other(format!(
                "GitHub 服务暂时不可用（HTTP {}）",
                resp.status
            )))
        }
        other => {
            return Err(AppError::Other(format!(
                "GitHub 返回了意外的状态码 HTTP {other}"
            )))
        }
    }

    serde_json::from_str::<GhRelease>(&resp.body)
        .map_err(|e| AppError::Other(format!("无法解析 GitHub 返回的内容：{e}")))
}

/// GitHub 要求请求方声明身份，否则直接拒绝。
fn user_agent() -> String {
    format!("BootFlow/{} (+https://github.com/{REPO})", current_version())
}

fn classify(release: GhRelease) -> UpdateCheck {
    let current = current_version().to_string();

    let Some(latest) = Version::parse(&release.tag_name) else {
        return UpdateCheck::failed(format!(
            "发布标签「{}」不是可比较的版本号，无法判断新旧",
            release.tag_name
        ));
    };

    let Some(mine) = Version::parse(&current) else {
        return UpdateCheck::failed(format!("当前版本号「{current}」无法解析"));
    };

    let status = if latest.is_newer_than(&mine) {
        UpdateStatus::Available
    } else {
        UpdateStatus::UpToDate
    };

    UpdateCheck {
        status,
        current_version: current,
        latest_version: Some(latest.to_string()),
        release_name: release.name.filter(|n| !n.trim().is_empty()),
        release_url: Some(release.html_url),
        published_at: release.published_at,
        notes: trim_notes(release.body),
        assets: sorted_assets(release.assets),
        reason: None,
        checked_at: now(),
    }
}

/// 截断更新说明。
///
/// 按**字符**而不是字节切：Release 正文里有中文和各种符号，
/// 按字节切会把一个汉字劈成两半，产生乱码。
fn trim_notes(body: Option<String>) -> Option<String> {
    let body = body?;
    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    let mut out: String = body.chars().take(NOTES_LIMIT).collect();
    if body.chars().count() > NOTES_LIMIT {
        out.push_str("\n\n（说明过长，完整内容见发布页）");
    }
    Some(out)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// 排列发布产物：安装包在前，便携包在后，其余保持 GitHub 给的原顺序。
///
/// 用户扫这一眼要回答的是「我该下哪个」，而不是「一共有哪些文件」。
/// `sort_by_key` 是稳定排序，同类内部仍按原顺序，不会打乱。
fn sorted_assets(assets: Vec<GhAsset>) -> Vec<ReleaseAsset> {
    let mut out: Vec<ReleaseAsset> = assets
        .into_iter()
        .map(|a| ReleaseAsset {
            name: a.name,
            url: a.browser_download_url,
            size: a.size,
        })
        .collect();

    out.sort_by_key(|a| asset_rank(&a.name));
    out
}

fn asset_rank(name: &str) -> u8 {
    let n = name.to_ascii_lowercase();
    if n.contains("setup") || n.contains("installer") || n.ends_with(".msi") {
        0
    } else if n.contains("portable") {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 发布产物排序 ────────────────────────────────────────

    #[test]
    fn puts_installer_before_portable() {
        let assets = vec![
            GhAsset {
                name: "bootflow-portable-x64.zip".into(),
                browser_download_url: "https://github.com/x/p".into(),
                size: 10,
            },
            GhAsset {
                name: "BootFlow_0.1.1_x64-setup.exe".into(),
                browser_download_url: "https://github.com/x/s".into(),
                size: 20,
            },
        ];

        let sorted = sorted_assets(assets);
        assert_eq!(sorted[0].name, "BootFlow_0.1.1_x64-setup.exe");
        assert_eq!(sorted[1].name, "bootflow-portable-x64.zip");
    }

    #[test]
    fn keeps_unrecognized_assets_last_without_reordering_them() {
        let mk = |n: &str| GhAsset {
            name: n.into(),
            browser_download_url: "https://github.com/x/y".into(),
            size: 1,
        };

        let sorted = sorted_assets(vec![mk("checksums.txt"), mk("bootflow.msi"), mk("extra.txt")]);
        assert_eq!(sorted[0].name, "bootflow.msi");
        // 不认识的两个保持原顺序
        assert_eq!(sorted[1].name, "checksums.txt");
        assert_eq!(sorted[2].name, "extra.txt");
    }

    // ── 版本号解析 ──────────────────────────────────────────

    #[test]
    fn parses_plain_and_prefixed() {
        assert_eq!(Version::parse("0.1.1").unwrap().to_string(), "0.1.1");
        assert_eq!(Version::parse("v0.1.1").unwrap().to_string(), "0.1.1");
        assert_eq!(Version::parse(" V1.2.3 ").unwrap().to_string(), "1.2.3");
    }

    #[test]
    fn missing_segments_default_to_zero() {
        let v = Version::parse("v2").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (2, 0, 0));
    }

    #[test]
    fn build_metadata_is_ignored() {
        assert_eq!(Version::parse("1.2.3+build.7").unwrap().to_string(), "1.2.3");
    }

    #[test]
    fn rejects_non_semver_tags() {
        // 这类标签没法比较新旧，必须被识别出来而不是硬凑一个结果
        assert!(Version::parse("nightly").is_none());
        assert!(Version::parse("release-2026").is_none());
        assert!(Version::parse("").is_none());
    }

    // ── 版本号比较 ──────────────────────────────────────────

    fn newer(a: &str, b: &str) -> bool {
        Version::parse(a).unwrap().is_newer_than(&Version::parse(b).unwrap())
    }

    #[test]
    fn compares_each_segment() {
        assert!(newer("0.1.1", "0.1.0"));
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("1.0.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.1.1"));
    }

    #[test]
    fn stable_beats_prerelease_of_same_number() {
        assert!(newer("1.0.0", "1.0.0-beta"));
        assert!(!newer("1.0.0-beta", "1.0.0"));
        // 关键：预发布版不应该被当成"有更新"，否则正式版用户会被反复提示
        assert!(!newer("0.2.0-rc.1", "0.2.0"));
    }

    #[test]
    fn compares_prerelease_identifiers() {
        assert!(newer("1.0.0-beta.2", "1.0.0-beta.1"));
        assert!(newer("1.0.0-beta.1", "1.0.0-beta"));
        assert!(newer("1.0.0-rc", "1.0.0-beta"));
        // 数字段比字母段小
        assert!(newer("1.0.0-alpha", "1.0.0-1"));
    }

    #[test]
    fn ignores_leading_v_when_comparing() {
        assert!(newer("v0.2.0", "0.1.0"));
        assert!(!newer("v0.1.0", "0.1.0"));
    }

    // ── 说明截断 ────────────────────────────────────────────

    #[test]
    fn trims_notes_by_chars_not_bytes() {
        let long = "中文说明".repeat(600); // 2400 字，超过上限
        let out = trim_notes(Some(long)).unwrap();
        assert!(out.ends_with("（说明过长，完整内容见发布页）"));
        // 截断处不能出现乱码替换符
        assert!(!out.contains('\u{FFFD}'));
    }

    #[test]
    fn keeps_short_notes_intact() {
        assert_eq!(trim_notes(Some("小改动".into())).unwrap(), "小改动");
    }

    #[test]
    fn empty_notes_become_none() {
        assert!(trim_notes(Some("   ".into())).is_none());
        assert!(trim_notes(None).is_none());
    }

    // ── 下载地址白名单 ──────────────────────────────────────

    #[test]
    fn accepts_github_hosts() {
        assert!(ensure_trusted_url("https://github.com/TEGCeylonQi/bootflow/releases/tag/v0.1.1").is_ok());
        assert!(ensure_trusted_url("https://api.github.com/repos/a/b").is_ok());
        assert!(ensure_trusted_url("https://objects.githubusercontent.com/x/y").is_ok());
        // 大小写与端口都不该影响判断
        assert!(ensure_trusted_url("https://GitHub.com:443/x").is_ok());
    }

    #[test]
    fn rejects_lookalike_and_other_hosts() {
        // 典型伪装：把 github.com 放在子域里，真正的域名是后一段
        assert!(ensure_trusted_url("https://github.com.evil.test/x").is_err());
        assert!(ensure_trusted_url("https://evil.test/github.com").is_err());
        assert!(ensure_trusted_url("http://github.com/x").is_err());
        assert!(ensure_trusted_url("file:///C:/windows/system32/calc.exe").is_err());
    }

    // ── 失败永远是"没检查成功"，不是"已是最新" ────────────

    #[test]
    fn failed_result_never_claims_up_to_date() {
        let r = UpdateCheck::failed("连接超时");
        assert_eq!(r.status, UpdateStatus::Failed);
        assert_eq!(r.reason.as_deref(), Some("连接超时"));
        assert!(r.latest_version.is_none());
    }

    #[test]
    fn failed_result_serializes_status_as_camel_case() {
        let json = serde_json::to_string(&UpdateCheck::failed("x")).unwrap();
        assert!(json.contains("\"status\":\"failed\""));
        assert!(json.contains("\"currentVersion\""));
    }

    #[test]
    fn status_variants_match_frontend_contract() {
        assert_eq!(serde_json::to_string(&UpdateStatus::UpToDate).unwrap(), "\"upToDate\"");
        assert_eq!(serde_json::to_string(&UpdateStatus::Available).unwrap(), "\"available\"");
        assert_eq!(serde_json::to_string(&UpdateStatus::Failed).unwrap(), "\"failed\"");
    }

    // ── 归类：把一次成功的响应变成结论 ──────────────────────

    fn release(tag: &str) -> GhRelease {
        GhRelease {
            tag_name: tag.to_string(),
            name: Some(format!("BootFlow {tag}")),
            body: Some("修了几个问题".to_string()),
            html_url: format!("https://github.com/{REPO}/releases/tag/{tag}"),
            published_at: Some("2026-09-15T00:00:00Z".to_string()),
            assets: vec![GhAsset {
                name: "BootFlow_0.1.1_x64-setup.exe".to_string(),
                browser_download_url: "https://github.com/x/y".to_string(),
                size: 1234,
            }],
        }
    }

    #[test]
    fn newer_tag_is_reported_as_available() {
        // 当前编译版本是 0.1.1（Cargo.toml），所以 0.2.0 应当被判为"有新版本"
        let r = classify(release("v0.2.0"));
        assert_eq!(r.status, UpdateStatus::Available);
        assert_eq!(r.latest_version.as_deref(), Some("0.2.0"));
        assert_eq!(r.current_version, current_version());
        assert_eq!(r.assets.len(), 1);
        assert!(r.reason.is_none());
    }

    #[test]
    fn same_or_older_tag_is_reported_as_up_to_date() {
        assert_eq!(classify(release("v0.1.1")).status, UpdateStatus::UpToDate);
        assert_eq!(classify(release("v0.1.0")).status, UpdateStatus::UpToDate);
    }

    /// 真实联网测试，默认不跑：`cargo test -- --ignored`。
    ///
    /// 留在仓库里是有必要的——「检查更新」的失败模式几乎全在网络这一侧
    /// （代理、证书、限流），而单元测试只能覆盖解析与比较。
    /// 发版前手动跑一次，能确认整条链路真的通。
    #[test]
    #[ignore = "需要联网，手动执行：cargo test -- --ignored"]
    fn live_github_api_is_reachable() {
        let r = check();
        println!("{}", serde_json::to_string_pretty(&r).unwrap());
        assert_ne!(
            r.status,
            UpdateStatus::Failed,
            "联网检查未成功：{}",
            r.reason.unwrap_or_default()
        );
    }

    #[test]
    fn unparsable_tag_fails_instead_of_guessing() {
        let r = classify(release("latest"));
        assert_eq!(r.status, UpdateStatus::Failed);
        assert!(r.reason.unwrap().contains("latest"));
    }
}
