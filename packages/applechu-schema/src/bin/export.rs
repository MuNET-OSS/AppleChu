use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("用法: applechu-schema-export <输出目录>")?;
    fs::create_dir_all(&output)?;
    fs::write(
        output.join("acmani.bin"),
        applechu_schema::SCHEMA.encode_acmani()?,
    )?;
    Ok(())
}
