//! 服务器下发条目与条件筛选
//!
//! 对应 `<field>.entries[]` 中的单个条目，以及 AquaMai
//! `ConditionalMessage.ShouldShow()` 的等价判定。
//!
//! 字段名与 AquaMai 保持一致，只替换品牌前缀：`_appleChuResources` /
//! `minimumAppleChuVersion` / `maximumAppleChuVersion`，其余（`url` / `sign` /
//! `locales` / `minimumGameVersion` / `maximumGameVersion`）完全相同。

use crate::server_resources::json::JsonValue;

/// 单个下发条目
#[derive(Clone, Debug, Default)]
pub struct ServerResourcesEntry {
    pub url: String,
    pub sign: String,
    pub locales: Vec<String>,
    pub minimum_apple_chu_version: Option<String>,
    pub maximum_apple_chu_version: Option<String>,
    pub minimum_game_version: i64,
    pub maximum_game_version: i64,
}

impl ServerResourcesEntry {
    /// 从 JSON 对象解析；缺少 `url` 或 `sign` 视为无效条目
    ///
    /// 字段名按「AppleChu 优先、AquaMai 兼容」读取：同一份响应里两种命名
    /// 都能生效，方便服务端一次性同时服务 AquaMai 与 AppleChu。
    pub fn from_json(value: &JsonValue) -> Option<Self> {
        let url = value.get("url").and_then(JsonValue::as_str)?;
        let sign = value.get("sign").and_then(JsonValue::as_str)?;
        if url.is_empty() || sign.is_empty() {
            return None;
        }

        Some(Self {
            url: url.to_owned(),
            sign: sign.to_owned(),
            locales: value
                .get("locales")
                .map(JsonValue::string_array)
                .unwrap_or_default(),
            minimum_apple_chu_version: optional_string_any(
                value,
                &["minimumAppleChuVersion", "minimumAquaMaiVersion"],
            ),
            maximum_apple_chu_version: optional_string_any(
                value,
                &["maximumAppleChuVersion", "maximumAquaMaiVersion"],
            ),
            minimum_game_version: value
                .get("minimumGameVersion")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0),
            maximum_game_version: value
                .get("maximumGameVersion")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0),
        })
    }

    /// 当前环境是否满足该条目的条件
    ///
    /// - `locales`：本期不筛（AppleChu 尚无语言来源），字段保留待后续接入；
    /// - AppleChu 版本：按 semver 数值比较；
    /// - 游戏版本：按整数比较，0 表示不限制。
    pub fn should_show(&self, locale: Option<&str>, game_version: i64) -> bool {
        if !locale_matches(&self.locales, locale) {
            return false;
        }

        let apple_chu_version = env!("CARGO_PKG_VERSION");
        if let Some(minimum) = &self.minimum_apple_chu_version {
            if !version_at_least(apple_chu_version, minimum) {
                return false;
            }
        }
        if let Some(maximum) = &self.maximum_apple_chu_version {
            if version_greater(apple_chu_version, maximum) {
                return false;
            }
        }

        if self.minimum_game_version != 0 && game_version < self.minimum_game_version {
            return false;
        }
        if self.maximum_game_version != 0 && game_version > self.maximum_game_version {
            return false;
        }

        true
    }
}

/// 从 `_appleChuResources` 根对象取出全部条目
pub fn parse_entries(resources: &JsonValue) -> Vec<ServerResourcesEntry> {
    resources
        .get("entries")
        .and_then(JsonValue::as_array)
        .map(|items| items.iter().filter_map(ServerResourcesEntry::from_json).collect())
        .unwrap_or_default()
}

/// 选出第一个满足条件的条目
pub fn select_entry<'a>(
    entries: &'a [ServerResourcesEntry],
    locale: Option<&str>,
    game_version: i64,
) -> Option<&'a ServerResourcesEntry> {
    entries
        .iter()
        .find(|entry| entry.should_show(locale, game_version))
}

/// 按候选键顺序取第一个非空字符串（AppleChu 键优先，AquaMai 键兜底）
fn optional_string_any(value: &JsonValue, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(key)
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
    })
}

/// 语言筛选：空列表表示不限制
///
/// 当前客户端尚未解析机台语言，`locale` 恒为 `None`，此时不因语言排除条目。
fn locale_matches(locales: &[String], locale: Option<&str>) -> bool {
    if locales.is_empty() {
        return true;
    }
    match locale {
        Some(current) => locales.iter().any(|item| item.eq_ignore_ascii_case(current)),
        // 无语言来源时不做排除，避免误杀全部条目
        None => true,
    }
}

/// 解析 `major.minor.patch` 为可比较的三元组
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.trim().split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u32>().unwrap_or(0);
    let patch = parts.next().unwrap_or("0").parse::<u32>().unwrap_or(0);
    Some((major, minor, patch))
}

/// `current >= minimum`；任一侧无法解析时视为满足，避免因版本串怪异而误杀
fn version_at_least(current: &str, minimum: &str) -> bool {
    match (parse_version(current), parse_version(minimum)) {
        (Some(current), Some(minimum)) => current >= minimum,
        _ => true,
    }
}

/// `current > maximum`；任一侧无法解析时视为不超限
fn version_greater(current: &str, maximum: &str) -> bool {
    match (parse_version(current), parse_version(maximum)) {
        (Some(current), Some(maximum)) => current > maximum,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_resources::json;

    fn entry_from(text: &str) -> ServerResourcesEntry {
        let value = json::parse(text).expect("测试 JSON 必须有效");
        ServerResourcesEntry::from_json(&value).expect("测试条目必须有效")
    }

    #[test]
    fn parses_full_entry() {
        // Given: 服务器返回的完整条目。
        let entry = entry_from(
            r#"{"url":"https://example/option.bin","sign":"AAAA","locales":["zh"],
                "minimumAppleChuVersion":"2.0.0","maximumAppleChuVersion":null,
                "minimumGameVersion":25000,"maximumGameVersion":0}"#,
        );

        // Then: 字段逐项落入结构体。
        assert_eq!(entry.url, "https://example/option.bin");
        assert_eq!(entry.locales, vec!["zh".to_owned()]);
        assert_eq!(entry.minimum_game_version, 25000);
        assert_eq!(entry.maximum_game_version, 0);
        assert!(entry.maximum_apple_chu_version.is_none());
    }

    #[test]
    fn entry_without_signature_is_rejected() {
        // Given: 缺 sign 的条目。
        // Then: 解析失败，避免无签名内容进入下载流程。
        let value = json::parse(r#"{"url":"https://example/option.bin"}"#).unwrap();
        assert!(ServerResourcesEntry::from_json(&value).is_none());
    }

    #[test]
    fn accepts_aquamai_version_field_names() {
        // Given: 服务端按 AquaMai 命名下发版本范围（同一份响应同时服务两个客户端）。
        let entry = entry_from(
            r#"{"url":"u","sign":"s","minimumAquaMaiVersion":"2.0.0","maximumAquaMaiVersion":"9.9.9"}"#,
        );

        // Then: 兼容读取，语义与 AppleChu 命名一致。
        assert_eq!(entry.minimum_apple_chu_version.as_deref(), Some("2.0.0"));
        assert_eq!(entry.maximum_apple_chu_version.as_deref(), Some("9.9.9"));
        assert!(entry.should_show(None, 0));
    }

    #[test]
    fn apple_chu_field_names_take_priority() {
        // Given: 两种命名同时出现。
        let entry = entry_from(
            r#"{"url":"u","sign":"s","minimumAppleChuVersion":"2.0.0","minimumAquaMaiVersion":"9.0.0"}"#,
        );

        // Then: 取 AppleChu 命名，不因兼容读取而放宽条件。
        assert_eq!(entry.minimum_apple_chu_version.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn game_version_range_filters_entries() {
        // Given: 限制游戏版本 25000 及以上的条目。
        let entry = entry_from(
            r#"{"url":"u","sign":"s","minimumGameVersion":25000,"maximumGameVersion":25099}"#,
        );

        // Then: 只有落在区间内的版本命中。
        assert!(entry.should_show(None, 25000));
        assert!(entry.should_show(None, 25099));
        assert!(!entry.should_show(None, 24999));
        assert!(!entry.should_show(None, 25100));
    }

    #[test]
    fn zero_game_version_means_unrestricted() {
        // Given: 未限制游戏版本的条目。
        let entry = entry_from(r#"{"url":"u","sign":"s"}"#);

        // Then: 任意游戏版本都命中。
        assert!(entry.should_show(None, 0));
        assert!(entry.should_show(None, 999999));
    }

    #[test]
    fn apple_chu_version_range_is_compared_semantically() {
        // Given: 上限低于当前版本（2.1.0）的条目，以及下限高于当前版本的条目。
        let too_new = entry_from(r#"{"url":"u","sign":"s","maximumAppleChuVersion":"1.9.9"}"#);
        let too_old = entry_from(r#"{"url":"u","sign":"s","minimumAppleChuVersion":"9.0.0"}"#);

        // Then: 两者都被排除；宽区间则命中。
        assert!(!too_new.should_show(None, 0));
        assert!(!too_old.should_show(None, 0));
        assert!(
            entry_from(r#"{"url":"u","sign":"s","minimumAppleChuVersion":"2.0.0"}"#)
                .should_show(None, 0)
        );
    }

    #[test]
    fn empty_locales_do_not_restrict() {
        // Given: 未声明语言的条目。
        let entry = entry_from(r#"{"url":"u","sign":"s"}"#);

        // Then: 有无语言信息都命中。
        assert!(entry.should_show(None, 0));
        assert!(entry.should_show(Some("zh"), 0));
    }

    #[test]
    fn select_entry_skips_non_matching_entries() {
        // Given: 首条被版本排除，次条可用。
        let value = json::parse(
            r#"{"entries":[
                {"url":"a","sign":"s","minimumGameVersion":999999},
                {"url":"b","sign":"s"}
            ]}"#,
        )
        .unwrap();
        let entries = parse_entries(&value);

        // When: 选出唯一条目。
        let selected = select_entry(&entries, None, 25000).expect("应选中第二条");

        // Then: 得到第一条可用条目。
        assert_eq!(selected.url, "b");
    }
}
