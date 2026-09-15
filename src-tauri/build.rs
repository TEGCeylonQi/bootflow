fn main() {
    // 默认以管理员权限运行（写入 exe 的 application manifest，level=requireAdministrator）。
    //
    // 【为什么默认提权】
    // 开机性能日志（Microsoft-Windows-Diagnostics-Performance/Operational）的通道
    // ACL 里没有普通用户——只有管理员能读。而「读不到就说没数据」会让用户误以为
    // 开机很快，等于伪造结论。程序作为诊断工具，启动时就要有读取能力。
    //
    // 【代价与取舍】
    // 每次冷启动都会弹一次 UAC。在「多一次点击」与「核心数据永远读不到」之间
    // 选前者。安装包仍用用户级安装（不需要管理员安装），只有运行时要提权。
    //
    // 【只在 release 注入】
    // debug / test 构建保持普通权限：`cargo test` 的测试 exe 如果也要求提权，
    // 普通的 CI/test shell 根本启动不了它（os error 740）。manifest 只写进
    // 发布包，开发与测试不受影响。
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
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>"#;

        tauri_build::try_build(
            tauri_build::Attributes::new().windows_attributes(
                WindowsAttributes::new().app_manifest(manifest),
            ),
        )
        .expect("failed to run tauri-build with admin manifest");
    }

    #[cfg(not(all(target_os = "windows", not(debug_assertions))))]
    tauri_build::build();
}