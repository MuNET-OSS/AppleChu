use once_cell::sync::Lazy;

pub static SCHEMA: Lazy<applechu_schema::Schema> = Lazy::new(|| {
    applechu_schema::Schema::parse(include_str!(concat!(env!("OUT_DIR"), "/schema.toml")))
        .expect("构建生成的 schema 必须有效")
});

pub fn section(id: &str) -> Option<&'static applechu_schema::SectionSpec> {
    SCHEMA.section(id)
}

// CCM 从最终 DLL 的 PE section 提取 schema；运行时不会把它当作 AppleChu.toml 读取
#[cfg(target_arch = "x86")]
#[used]
#[link_section = ".acmani"]
static ACMANI: [u8; include_bytes!(concat!(env!("OUT_DIR"), "/acmani.bin")).len()] =
    *include_bytes!(concat!(env!("OUT_DIR"), "/acmani.bin"));
