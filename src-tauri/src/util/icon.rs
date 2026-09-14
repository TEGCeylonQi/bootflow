//! 图标提取：从可执行文件取图标并编码成 base64 PNG。
//!
//! 走 `IShellItemImageFactory`，而不是 `ExtractIconExW` + `GetIconInfo` + `GetDIBits`
//! 那条老路。原因：
//!
//! - Shell 返回的是**带 alpha 通道的 32bpp DIB**，直接用。老 API 拿到的是
//!   1bpp 掩码图 + 24bpp 彩图，得自己合成透明度，还经常在高 DPI 下取到糊图。
//! - Shell 会自动走到正确的图标来源（PE 资源 / 关联的 `.ico` / 文件类型图标），
//!   对 `.lnk`、`.bat` 这类没有内嵌图标的文件也能给出合理结果。
//! - 尺寸由我们指定，Sheet 会做高质量缩放，不必自己插值。
//!
//! ⚠️ 图标提取**失败绝不影响扫描**——路径失效、权限不足、Shell 内部超时都会
//! 发生。返回值是 `Option`，取不到就让前端的兜底图标顶上。

use base64::Engine as _;

use crate::util::com::ComGuard;

/// 列表里显示用的尺寸。32px 在 100% 与 125% 缩放下都够清晰，
/// 而 256px 会让几十个图标的 base64 串变成几百 KB，得不偿失。
pub const DEFAULT_SIZE: u32 = 32;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 提取图标并编码为 base64 PNG（**不带** `data:` 前缀，前端自己拼）。
///
/// 返回 `None` 的常见原因：路径不存在、无权限读取、Shell 拿不出图像。
pub fn extract_png_base64(path: &str, size: u32) -> Option<String> {
    let trimmed = path.trim().trim_matches('"').trim();
    if trimmed.is_empty() {
        return None;
    }

    extract_impl(trimmed, size)
}

#[cfg(windows)]
fn extract_impl(path: &str, size: u32) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{DeleteObject, GetObjectW, BITMAP, HGDIOBJ};
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
    };

    // Shell 组件要求当前线程初始化过 COM。放在这里而不是让调用方负责，
    // 是因为调用方（command 层）跑在线程池上，未必知道这件事。
    let _com = ComGuard::new();

    let path_w = wide(path);

    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR(path_w.as_ptr()), None).ok()?;

        let hbm = factory
            .GetImage(
                SIZE {
                    cx: size as i32,
                    cy: size as i32,
                },
                SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
            )
            .ok()?;

        // 取完立刻处理像素，随后无论如何都要释放 GDI 对象
        let result = hbitmap_to_base64_png(hbm);

        let mut bm: BITMAP = std::mem::zeroed();
        if GetObjectW(
            HGDIOBJ(hbm.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut core::ffi::c_void),
        ) != 0
        {
            let _ = DeleteObject(HGDIOBJ(hbm.0));
        }

        result
    }
}

#[cfg(windows)]
unsafe fn hbitmap_to_base64_png(
    hbm: windows::Win32::Graphics::Gdi::HBITMAP,
) -> Option<String> {
    use windows::Win32::Graphics::Gdi::{GetObjectW, BITMAP, HGDIOBJ};

    let mut bm: BITMAP = std::mem::zeroed();
    let got = GetObjectW(
        HGDIOBJ(hbm.0),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bm as *mut _ as *mut core::ffi::c_void),
    );
    if got == 0 {
        return None;
    }

    let width = bm.bmWidth;
    let raw_height = bm.bmHeight;
    let stride = bm.bmWidthBytes;
    let bits = bm.bmBits as *const u8;

    if width <= 0 || raw_height == 0 || stride <= 0 || bits.is_null() {
        return None;
    }

    let w = width as usize;
    let h = raw_height.unsigned_abs() as usize;

    // DIB 的高度为负表示"自上而下"存储；为正则是传统的"自下而上"，
    // 需要在转换时把行倒过来。搞错这一步图标会上下颠倒。
    let bottom_up = raw_height > 0;

    let stride = stride as usize;
    if stride < w * 4 {
        return None;
    }

    let src = std::slice::from_raw_parts(bits, stride * h);

    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        let y = if bottom_up { h - 1 - row } else { row };
        let line = &src[y * stride..y * stride + w * 4];
        for px in line.chunks_exact(4) {
            // DIB 是 BGRA 序，PNG 要 RGBA
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }

    // 某些来源的位图 alpha 全为 0（老的 24bpp 图标被转换过来时会这样）。
    // 若原样输出，PNG 会整张透明、界面上什么也看不到——
    // 这里补成不透明，宁可丢一点圆角，也不能让用户看到空白。
    if rgba.chunks_exact(4).all(|px| px[3] == 0) {
        for px in rgba.chunks_exact_mut(4) {
            px[3] = 0xFF;
        }
    }

    encode_png(w as u32, h as u32, &rgba)
}

#[cfg(not(windows))]
fn extract_impl(_path: &str, _size: u32) -> Option<String> {
    None
}

/// RGBA 缓冲 → base64 PNG。
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);

        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
        // writer 在此处 drop，完成 IEND 写入
    }

    Some(base64::engine::general_purpose::STANDARD.encode(&out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_png_produces_valid_base64() {
        // 2x2 全红方块
        let rgba = [255u8, 0, 0, 255].repeat(4);
        let b64 = encode_png(2, 2, &rgba).expect("应能编码");

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&b64)
            .expect("应是合法 base64");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "应为 PNG 魔数");
    }

    #[test]
    fn empty_path_returns_none() {
        assert!(extract_png_base64("   ", DEFAULT_SIZE).is_none());
    }

    #[test]
    fn missing_file_returns_none_without_panic() {
        assert!(extract_png_base64(r"D:\__bootflow_missing__\nope.exe", DEFAULT_SIZE).is_none());
    }

    #[test]
    fn system_binary_yields_a_real_png() {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let path = format!(r"{root}\System32\notepad.exe");

        let b64 = extract_png_base64(&path, DEFAULT_SIZE).expect("系统程序应能取到图标");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&b64)
            .expect("应是合法 base64");

        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        // 32x32 的 PNG 至少几百字节；太小说明只拿到一个空图
        assert!(bytes.len() > 200, "图标数据过小，可能取到空图");
    }
}
