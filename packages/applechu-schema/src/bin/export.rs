use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let source = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("用法: applechu-schema-export <Rust 源目录> <输出目录>")?;
    let output = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("用法: applechu-schema-export <Rust 源目录> <输出目录>")?;
    fs::create_dir_all(&output)?;
    let schema = applechu_schema::generate_from_rust_dir(source)?;
    fs::write(output.join("acmani.bin"), schema.encode_acmani()?)?;
    Ok(())
}
