// CCM 从最终 DLL 的 PE section 提取 schema；运行时不会把它当作 AppleChu.toml 读取
#[used]
#[link_section = ".acmani"]
static ACMANI: [u8; include_bytes!(concat!(env!("OUT_DIR"), "/acmani.bin")).len()] =
    *include_bytes!(concat!(env!("OUT_DIR"), "/acmani.bin"));
