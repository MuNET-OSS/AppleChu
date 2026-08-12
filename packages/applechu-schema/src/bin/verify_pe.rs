use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let image = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("用法: verify_pe <winhttp.dll> [示例配置输出路径]")?;
    let example = arguments.next().map(PathBuf::from);
    if arguments.next().is_some() {
        return Err("用法: verify_pe <winhttp.dll> [示例配置输出路径]".into());
    }
    let bytes = fs::read(&image)?;
    let schema = applechu_schema::decode_pe_acmani(&bytes)?;
    if let Some(example) = example {
        fs::write(example, schema.default_config)?;
    }
    println!(
        ".acmani V1 有效: manifest={} bytes, default_config={} bytes",
        schema.manifest.len(),
        schema.default_config.len()
    );
    Ok(())
}
