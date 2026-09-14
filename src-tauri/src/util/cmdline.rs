//! 命令行解析：把 `"C:\Program Files\App\app.exe" --min --x=1` 拆成
//! 路径与参数两部分。
//!
//! 为什么不能简单 `split(' ')`：Windows 允许路径含空格，
//! **加引号**和**不加引号**两种写法都真实存在，注册表里两种都能见到。
//! 拆分错了就会出现"这个程序明明装了却报找不到"的假阳性。
//!
//! 这里分两路处理：
//! 1. 以引号开头 → 引号内即路径，语义明确，直接切。
//! 2. 其余情况 → 逐段扩展试探，取第一个**真实存在的文件**作为路径。
//!    若都不存在则退回第一段，交由有效性检测报"目标不存在"。

use std::path::Path;

/// 展开 `%VAR%` 形式的环境变量。
///
/// 为什么不调 `ExpandEnvironmentStringsW`：进程启动时已继承完整环境块，
/// `std::env::var` 在 Windows 上底层走 `GetEnvironmentVariableW`，
/// **本身就是大小写不敏感的**，行为与系统 API 一致。
/// 少一个 Win32 调用就少一处 API 版本差异。
///
/// 变量不存在时**原样保留**（`%NOSUCH%`），这与系统 API 的行为一致——
/// 悄悄删掉会让用户以为命令本身就是这样，反而查不出问题。
pub fn expand_env(input: &str) -> String {
    if !input.contains('%') {
        return input.to_string();
    }

    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '%' {
            // 找配对的结束 %
            if let Some(offset) = chars[i + 1..].iter().position(|&c| c == '%') {
                let name: String = chars[i + 1..i + 1 + offset].iter().collect();

                if name.is_empty() {
                    // `%%` —— 字面量百分号
                    out.push('%');
                    i += 2;
                    continue;
                }

                if let Ok(value) = std::env::var(&name) {
                    out.push_str(&value);
                    i += offset + 2;
                    continue;
                }
            }
            // 不成对 / 变量不存在 —— 保留原字符继续
            out.push('%');
            i += 1;
            continue;
        }

        out.push(chars[i]);
        i += 1;
    }

    out
}

/// 引号感知分词。引号本身被剥离，引号内的空格不切分。
///
/// `--x="a b"` → `["--x=a b"]`，这是刻意的：参数值里的空格是内容，
/// 不是分隔符。拆错了会影响 `identity_key`，进而影响去重判断。
fn tokenize(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut has_content = false;

    for ch in s.chars() {
        match ch {
            '"' => {
                in_quote = !in_quote;
                // 空串 `""` 也是一个显式参数，用标记记住"这里有过东西"
                has_content = true;
            }
            c if c.is_whitespace() && !in_quote => {
                if has_content {
                    tokens.push(std::mem::take(&mut current));
                    has_content = false;
                }
            }
            c => {
                current.push(c);
                has_content = true;
            }
        }
    }

    if has_content {
        tokens.push(current);
    }

    tokens
}

/// 这个字符串里有没有路径分隔符。
///
/// 用来区分两种完全不同的东西：
/// - `C:\App\a.exe` / `..\a.exe` —— 已经是路径，直接判断存在性即可
/// - `rundll32.exe` —— **只是程序名**，它不在当前目录也不在 CWD，
///   靠系统的搜索顺序才能找到。按路径判会得到"程序已卸载"的假阳性。
pub fn has_path_separator(s: &str) -> bool {
    s.contains('\\') || s.contains('/')
}

/// 对「裸程序名」按 Windows 的搜索顺序查一遍，返回找到的完整路径。
///
/// Run 键里这种写法很常见，例如
/// `rundll32.exe iernonce.dll,RunOnceExProcess`、
/// `MsiExec.exe /X{90120000-...}`（卸载残留）。
/// 这些程序名都能在 `System32` 里找到——**不查这一步的话，界面上会把它们
/// 全部标成"目标文件不存在，程序可能已被卸载"**，而用户明明还装着 Office。
///
/// 搜索顺序按 `CreateProcess` 的规则简化：`System32` → `SysWOW64` → Windows 目录 → `PATH`。
/// 省略了"应用目录"和"当前目录"——启动项的这两项对整个系统而言没有稳定含义。
///
/// 拿不准时（例如 `System32` 与 `SysWOW64` 同名文件）**优先返回 System32**：
/// 这里的结果只用于展示与验签，不参与实际执行，选主流架构更不容易让人困惑。
pub fn resolve_via_search_path(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || has_path_separator(name) {
        return None;
    }

    let mut dirs: Vec<std::path::PathBuf> = Vec::new();

    if let Some(root) = std::env::var_os("SystemRoot") {
        let root = std::path::PathBuf::from(root);
        dirs.push(root.join("System32"));
        dirs.push(root.join("SysWOW64"));
        dirs.push(root.clone());
    }

    if let Some(path) = std::env::var_os("PATH") {
        // split_paths 按 `;` 切分且正确处理带引号的路径项
        dirs.extend(std::env::split_paths(&path));
    }

    for dir in dirs {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }

    None
}

/// 把命令行拆成「可执行文件路径 + 参数」。
///
/// 返回的路径已展开环境变量、已去引号。**不保证存在**——
/// 存在性判断是 `valid.rs` 的职责，这里只负责"拆对"。
///
/// 主要消费者是 `scanners::registry`（Run 键的值全是这种「路径 + 参数」命令行）。
pub fn split_command(command: &str) -> (String, Vec<String>) {
    let cmd = expand_env(command);
    let cmd = cmd.trim();

    if cmd.is_empty() {
        return (String::new(), Vec::new());
    }

    // ── 路径 1：整体被引号包裹 ──
    // `"C:\Program Files\App\app.exe" --min`
    // 这是最常见也最规范的写法，不需要试探。
    if let Some(rest) = cmd.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let path = rest[..end].to_string();
            let args = tokenize(&rest[end + 1..]);
            return (path, args);
        }
        // 引号没闭合（本身就是个故障，valid 会报出来）——
        // 去掉开头引号后当作无引号处理，至少能把路径试出来
        return split_unquoted(rest.trim_start_matches('"'));
    }

    split_unquoted(cmd)
}

/// 无引号路径：从左往右逐步合并 token，取第一个真实存在的文件。
fn split_unquoted(cmd: &str) -> (String, Vec<String>) {
    let tokens = tokenize(cmd);

    if tokens.is_empty() {
        return (String::new(), Vec::new());
    }

    // 合并上限 8 段。真实路径再长也不会超过这个数，
    // 设上限是为了避免在"整条命令都不存在"时做无意义的 O(n²) 试探。
    let limit = tokens.len().min(8);

    for take in 1..=limit {
        let candidate = tokens[..take].join(" ");
        if Path::new(&candidate).is_file() {
            return (candidate, tokens[take..].to_vec());
        }
    }

    // 都不存在：第一段当路径，其余当参数。
    // 这样用户至少能在界面上看到"它想启动哪个程序"。
    let path = tokens[0].clone();
    (path, tokens[1..].to_vec())
}

/// 解析快捷方式里独立存放的参数串（`IShellLinkW::GetArguments` 的返回值）。
///
/// 与 `split_command` 分开，是因为这里的输入**确定只含参数**，
/// 不需要也无法做路径试探。
pub fn parse_args_only(raw: &str) -> Vec<String> {
    let expanded = expand_env(raw);
    tokenize(&expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_known_variable() {
        std::env::set_var("BOOTFLOW_TEST_VAR", "foo");
        assert_eq!(expand_env("a%BOOTFLOW_TEST_VAR%b"), "afoob");
    }

    #[test]
    fn keeps_unknown_variable_intact() {
        // 保留原样而非删除，这样用户能看到"这里有个我没见过的变量"
        assert_eq!(expand_env("%__NO_SUCH_VAR__%x"), "%__NO_SUCH_VAR__%x");
    }

    #[test]
    fn quoted_path_with_spaces_is_split_correctly() {
        let (path, args) = split_command("\"C:\\Program Files\\A B\\a.exe\" --min --x=1");
        assert_eq!(path, "C:\\Program Files\\A B\\a.exe");
        assert_eq!(args, vec!["--min", "--x=1"]);
    }

    #[test]
    fn quoted_argument_keeps_inner_spaces() {
        let (_, args) = split_command("C:\\a.exe --name=\"hello world\"");
        assert_eq!(args, vec!["--name=hello world"]);
    }

    #[test]
    fn unquoted_path_falls_back_to_first_token() {
        // 路径不存在时不应把参数吞进路径里
        let (path, args) = split_command("D:\\__bootflow_missing__\\x.exe --flag");
        assert_eq!(path, "D:\\__bootflow_missing__\\x.exe");
        assert_eq!(args, vec!["--flag"]);
    }

    #[test]
    fn real_file_is_picked_out_of_unquoted_command() {
        // %SystemRoot%\System32\notepad.exe 在所有 Windows 上都存在。
        // 用 %WINDIR% 顺带验证环境变量展开与"无引号含空格路径"的合并试探。
        let cmd = "%WINDIR%\\System32\\notepad.exe /x";
        let (path, args) = split_command(cmd);
        assert!(
            path.to_ascii_lowercase().ends_with("notepad.exe"),
            "应识别出完整路径，实际得到：{path}"
        );
        assert_eq!(args, vec!["/x"]);
    }

    #[test]
    fn empty_command_is_safe() {
        let (path, args) = split_command("   ");
        assert!(path.is_empty());
        assert!(args.is_empty());
    }

    #[test]
    fn separator_detection_distinguishes_paths_from_bare_names() {
        assert!(has_path_separator(r"C:\App\a.exe"));
        assert!(has_path_separator("sub/dir/a.exe"));
        // 裸程序名是关键区分点：它靠系统搜索顺序才能找到
        assert!(!has_path_separator("rundll32.exe"));
        assert!(!has_path_separator("msiexec.exe"));
    }

    #[test]
    fn bare_command_name_is_resolved_via_search_path() {
        // 每个 Windows 上都有它。Run 键里 `rundll32.exe xxx.dll,Entry`
        // 这种写法极常见，不补全就会全被误报成"程序已卸载"。
        let found = resolve_via_search_path("rundll32.exe")
            .expect("rundll32.exe 应能在 System32 或搜索路径里找到");
        assert!(
            Path::new(&found).is_file(),
            "补全结果必须是真实存在的文件：{found}"
        );
    }

    #[test]
    fn already_a_path_is_left_alone_by_search() {
        // 已经是完整路径的，不该再去搜索路径里找一遍——
        // 那样可能把一个存在的路径替换成另一个同名文件
        assert!(resolve_via_search_path(r"C:\Windows\notepad.exe").is_none());
        assert!(resolve_via_search_path("").is_none());
    }

    #[test]
    fn nonexistent_bare_name_resolves_to_none() {
        // 找不到就如实返回 None，交给有效性检测报"目标不存在"，
        // 而不是编一个路径出来
        assert!(resolve_via_search_path("__bootflow_no_such_tool__.exe").is_none());
    }
}
