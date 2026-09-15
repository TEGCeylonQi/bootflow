fn main() {
    // 写入 exe 的 application manifest。**刻意用 asInvoker**：
    // 进程以启动它的那个用户令牌运行，不请求提权、不弹 UAC。
    //
    // 【为什么不是 highestAvailable / requireAdministrator】
    // 这两个都让 exe 自带「我要提权」的声明，代价很重：
    // 1. 管理员用户**每次双击都要点一次 UAC**——一个启动项体检工具不该这样；
    // 2. 受限令牌环境（脚本、CI、被沙箱限制的 shell）**根本启动不了**它，
    //    报 `Permission denied` / `os error 126`——本机实测踩过；
    // 3. 放进 `HKCU\...\Run` 做开机自启时，登录过程会弹 UAC，
    //    用户必然去关掉自启，开机自记账就废了。
    //
    // 【不提权，功能上够用吗？——够】
    // 开机耗时是两层数据，权限要求完全不同：
    // - 「这次开机用时」自记账：读 System 通道的 Event 12 / 6005 / 6009，
    //   **普通用户就能读**（本机在受限令牌下实测通过），核心功能不依赖提权；
    // - 「分段耗时」（到底哪些服务/启动项拖慢的）：读
    //   `Microsoft-Windows-Diagnostics-Performance/Operational`，
    //   该通道 ACL 只给了管理员。读不到时程序会**如实说明并给出
    //   「以管理员身份重开」入口**，而不是假装没数据。
    // 所以这是刻意的取舍：默认永不弹 UAC，想看分段耗时由用户主动提权一次。
    //
    // 【必须保留 Common-Controls v6 依赖】
    // 缺少它时系统 dialog/通用控件样式会回退到 Win95 时代。
    #[cfg(all(target_os = "windows", not(debug_assertions)))]
    {
        use tauri_build::WindowsAttributes;

        let manifest = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>"#;

        tauri_build::try_build(
            tauri_build::Attributes::new().windows_attributes(
                WindowsAttributes::new().app_manifest(manifest),
            ),
        )
        .expect("failed to run tauri-build with app manifest");
    }

    #[cfg(not(all(target_os = "windows", not(debug_assertions))))]
    tauri_build::build();
}