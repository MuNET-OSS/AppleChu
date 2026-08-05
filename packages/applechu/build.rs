use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../applechu-schema/src");
    println!("cargo:rerun-if-changed=../applechu-schema/src/schema.toml");
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86")
    {
        return Ok(());
    }

    let output = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR must be available")?);
    let blob = applechu_schema::SCHEMA.encode_acmani()?;
    fs::write(output.join("acmani.bin"), blob)?;

    let runtime_dll = r"C:\Windows\System32\winhttp.dll";
    if std::env::consts::OS == "windows" {
        forward_dll::forward_dll(runtime_dll)?;
        return Ok(());
    }

    let dev_dll = std::env::var("APPLECHU_WINHTTP_DEV_DLL")
        .unwrap_or_else(|_| "/mnt/c/Windows/SysWOW64/winhttp.dll".to_owned());
    println!("cargo:rerun-if-env-changed=APPLECHU_WINHTTP_DEV_DLL");
    println!("cargo:rerun-if-changed={dev_dll}");
    forward_dll::forward_dll_with_dev_path(runtime_dll, &dev_dll)?;
    Ok(())
}
