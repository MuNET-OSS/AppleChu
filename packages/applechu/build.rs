use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");

    let output = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR must be available")?);
    let schema = applechu_schema::generate_from_rust_dir("src")?;
    fs::write(output.join("schema.toml"), schema.manifest_toml()?)?;
    let artifact_directory = output
        .ancestors()
        .find(|directory| directory.file_name().is_some_and(|name| name == "build"))
        .and_then(|directory| directory.parent())
        .ok_or("OUT_DIR must contain Cargo's build directory")?;
    schema.write_example_config(artifact_directory)?;

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86")
    {
        return Ok(());
    }

    let blob = schema.encode_acmani()?;
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
