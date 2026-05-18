use crate::util::api::Api;

pub fn scan(api: &Api, pattern_str: &str) -> usize {
    scan_range(api, api.game_base(), api.game_size(), pattern_str)
}

pub fn scan_range(api: &Api, start: usize, size: u32, pattern_str: &str) -> usize {
    let Some((pattern, mask)) = parse_pattern(pattern_str) else {
        return 0;
    };
    api.aob_scan(start, size, &pattern, &mask)
}

pub fn scan_bytes(api: &Api, bytes: &[u8]) -> usize {
    let mask = "x".repeat(bytes.len());
    api.aob_scan(api.game_base(), api.game_size(), bytes, &mask)
}

fn parse_pattern(pattern_str: &str) -> Option<(Vec<u8>, String)> {
    let mut pattern = Vec::new();
    let mut mask = String::new();

    for token in pattern_str.split_whitespace() {
        if token == "??" || token == "?" {
            pattern.push(0);
            mask.push('?');
            continue;
        }

        let byte = u8::from_str_radix(token, 16).ok()?;
        pattern.push(byte);
        mask.push('x');
    }

    (!pattern.is_empty()).then_some((pattern, mask))
}
