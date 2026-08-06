use crate::util::memory::PatchMemory;
use crate::util::pattern;

const ACTIVE_RELEASE_TAG_PATTERN: &str = concat!(
    "E8 ?? ?? ?? ?? 8B 00 83 F8 15 77 08 8B 04 85 ",
    "?? ?? ?? ?? C3 B8 17 00 00 00 C3 CC CC CC CC CC"
);
const RELEASE_TAG_BUCKET_PATTERN: &str = concat!(
    "8B 44 24 04 83 F8 15 77 08 8B 04 85 ",
    "?? ?? ?? ?? C3 B8 17 00 00 00 C3 CC CC CC CC CC"
);

const ACTIVE_PREFIX_LEN: usize = 7;
const BUCKET_PREFIX_LEN: usize = 4;
const ACTIVE_TABLE_OFFSET: usize = 15;
const BUCKET_TABLE_OFFSET: usize = 12;

fn patch_mapper<M: PatchMemory>(
    api: &M,
    signature: usize,
    prefix_len: usize,
    table_offset: usize,
) -> bool {
    let patch_len = prefix_len + 24;
    let mut original = vec![0_u8; patch_len];
    if !api.mem_read(signature, &mut original) {
        return false;
    }

    let mut patch = Vec::with_capacity(patch_len);
    patch.extend_from_slice(&original[..prefix_len]);
    // 内置 ID 查表，自定义 ID 共用槽位 0，负数保留无效槽位 23
    patch.extend_from_slice(&[
        0x85, 0xC0, 0x78, 0x10, 0x83, 0xF8, 0x16, 0x72, 0x03, 0x33, 0xC0, 0xC3, 0x8B, 0x04, 0x85,
    ]);
    patch.extend_from_slice(&original[table_offset..table_offset + 4]);
    patch.extend_from_slice(&[0xC3, 0x6A, 0x17, 0x58, 0xC3]);

    patch.len() == patch_len && api.mem_write(signature, &patch)
}

pub(crate) fn apply_early<M: PatchMemory>(api: &M) {
    let active_tag = pattern::scan(api, ACTIVE_RELEASE_TAG_PATTERN);
    let bucket_map = pattern::scan(api, RELEASE_TAG_BUCKET_PATTERN);

    if active_tag == 0 || bucket_map == 0 {
        api.log_warn("custom release tag compatibility: target code not found");
        return;
    }

    let active_patched = patch_mapper(api, active_tag, ACTIVE_PREFIX_LEN, ACTIVE_TABLE_OFFSET);
    let bucket_patched = patch_mapper(api, bucket_map, BUCKET_PREFIX_LEN, BUCKET_TABLE_OFFSET);
    if active_patched && bucket_patched {
        api.log_info("Patch applied: custom release tag compatibility");
    } else {
        api.log_warn("custom release tag compatibility: target code mismatch");
    }
}
