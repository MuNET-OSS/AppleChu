// CCM schema 只在构建时读取，最终内容嵌入 PE section
pub const SOURCE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/schema.toml"));
