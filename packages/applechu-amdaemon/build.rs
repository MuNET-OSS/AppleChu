// winmm.dll 代理会转发系统 DLL 的全部导出，同时保留 Rust DllMain hook
// AM Daemon 仍是原始可执行文件；本 DLL 只安装启动跳板，随后继续执行原入口
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86_64")
    {
        return Ok(());
    }

    let runtime_dll = r"C:\Windows\System32\winmm.dll";
    if std::env::consts::OS == "windows" {
        forward_dll::forward_dll(runtime_dll)?;
        return Ok(());
    }

    let dev_dll = std::env::var("APPLECHU_WINMM_DEV_DLL")
        .unwrap_or_else(|_| "/mnt/c/Windows/System32/winmm.dll".to_owned());
    println!("cargo:rerun-if-env-changed=APPLECHU_WINMM_DEV_DLL");
    println!("cargo:rerun-if-changed={dev_dll}");
    forward_dll::forward_dll_with_dev_path(runtime_dll, &dev_dll)?;
    Ok(())
}
