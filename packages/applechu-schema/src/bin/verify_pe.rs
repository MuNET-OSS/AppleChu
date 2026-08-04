use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let image = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("用法: verify_pe <winhttp.dll>")?;
    let bytes = fs::read(&image)?;
    let schema = applechu_schema::decode_pe_acmani(&bytes)?;
    println!(
        ".acmani V1 有效: manifest={} bytes, default_config={} bytes",
        schema.manifest.len(),
        schema.default_config.len()
    );
    Ok(())
}
