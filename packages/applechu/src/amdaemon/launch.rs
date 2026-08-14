use crate::iohook::hook_table::{hook_table_apply, null_module, HookSymbol};

const CONFIG_FILE_PATTERN: &str = "config_*.json";
const STANDARD_CONFIG_ORDER: [&str; 6] = [
    "config_common.json",
    "config_server.json",
    "config_client.json",
    "config_cvt.json",
    "config_sp.json",
    "config_hook.json",
];

pub const INHERIT_CONSOLE_ENV: &str = "APPLECHU_AMDAEMON_INHERIT_CONSOLE";

crate::config_section! {
    pub(crate) struct AmdaemonConfig => AMDAEMON_CONFIG_SECTION {
        section: "Amdaemon",
        order: 20,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "AM Daemon 自动启动及命令行管理",
        fields: {
            pub auto_start: bool = true,
            comment: "游戏自动拉起 AM Daemon";
            pub executable: String = String::from("amdaemon.exe"),
            advanced: true,
            comment: "AM Daemon 可执行文件名";
            pub hide_window: bool = false,
            comment: "手动启动 AM Daemon 时隐藏控制台窗口";
            pub terminate_on_exit: bool = true,
            advanced: true,
            comment: "ChusanApp 退出时终止 AM Daemon";
            pub append_config_args: bool = true,
            advanced: true,
            comment: "无完整 -c 参数时补充 JSON 配置";
            pub config_files: Vec<String> = vec![CONFIG_FILE_PATTERN.to_owned()],
            advanced: true,
            schema_default: ["config_*.json"],
            comment: "JSON 配置文件列表";
        }
    }
}

pub fn config_files(base_dir: &str) -> Vec<String> {
    let config = crate::config::Config::global(base_dir);
    let configured = config
        .section::<AmdaemonConfig>()
        .filter(|section| section.enabled)
        .map(|section| section.config_files.clone())
        .unwrap_or_default();
    resolve_config_files(std::path::Path::new(base_dir), &configured)
}

fn resolve_config_files(base_dir: &std::path::Path, configured: &[String]) -> Vec<String> {
    if configured.is_empty() {
        return discover_config_files(base_dir);
    }

    let mut resolved = Vec::new();
    for file in configured {
        if file.eq_ignore_ascii_case(CONFIG_FILE_PATTERN) {
            for discovered in discover_config_files(base_dir) {
                if !resolved
                    .iter()
                    .any(|current: &String| current.eq_ignore_ascii_case(&discovered))
                {
                    resolved.push(discovered);
                }
            }
        } else if !resolved
            .iter()
            .any(|current| current.eq_ignore_ascii_case(file))
        {
            resolved.push(file.clone());
        }
    }
    resolved
}

fn discover_config_files(base_dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(base_dir) else {
        return Vec::new();
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|file_type| file_type.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            let name = name.to_ascii_lowercase();
            name.starts_with("config_") && name.ends_with(".json")
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| {
        config_file_rank(left)
            .cmp(&config_file_rank(right))
            .then_with(|| left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase()))
            .then_with(|| left.cmp(right))
    });
    files
}

fn config_file_rank(name: &str) -> usize {
    STANDARD_CONFIG_ORDER
        .iter()
        .position(|standard| name.eq_ignore_ascii_case(standard))
        .unwrap_or(STANDARD_CONFIG_ORDER.len())
}

pub fn append_config_args(base_dir: &str) -> bool {
    crate::config::Config::global(base_dir)
        .section::<AmdaemonConfig>()
        .is_some_and(|section| section.enabled && section.append_config_args)
}

pub fn hide_window(base_dir: &str) -> bool {
    crate::config::Config::global(base_dir)
        .section::<AmdaemonConfig>()
        .is_some_and(|section| section.enabled && section.hide_window)
}

pub unsafe fn install_command_line_hooks(
    get_command_line_a: *const (),
    get_command_line_w: *const (),
) -> usize {
    let symbols = [
        HookSymbol {
            name: "GetCommandLineA",
            patch: get_command_line_a,
            original: std::ptr::null_mut(),
        },
        HookSymbol {
            name: "GetCommandLineW",
            patch: get_command_line_w,
            original: std::ptr::null_mut(),
        },
    ];
    hook_table_apply(null_module(), "kernel32.dll", &symbols)
}

pub unsafe fn install_wgetmainargs_hook(replacement: *const (), original: *mut *const ()) -> usize {
    let symbols = [HookSymbol {
        name: "__wgetmainargs",
        patch: replacement,
        original,
    }];
    hook_table_apply(null_module(), "msvcr110.dll", &symbols)
}

#[cfg(test)]
mod tests {
    use super::{discover_config_files, resolve_config_files, CONFIG_FILE_PATTERN};

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "applechu-amdaemon-config-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("系统时间必须有效")
                    .as_nanos()
            ));
            std::fs::create_dir(&path).expect("测试目录必须可创建");
            Self(path)
        }

        fn create(&self, name: &str) {
            std::fs::write(self.0.join(name), []).expect("测试文件必须可创建");
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovers_all_json_configs_in_standard_order() {
        let directory = TestDirectory::new();
        for name in [
            "config_extra.json",
            "config_sp.json",
            "config_common.json",
            "config_server.json.bak",
            "other.json",
        ] {
            directory.create(name);
        }

        assert_eq!(
            discover_config_files(&directory.0),
            ["config_common.json", "config_sp.json", "config_extra.json"]
        );
        assert_eq!(
            resolve_config_files(&directory.0, &[CONFIG_FILE_PATTERN.to_owned()]),
            ["config_common.json", "config_sp.json", "config_extra.json"]
        );
    }

    #[test]
    fn explicit_config_files_keep_their_order() {
        let configured = vec!["second.json".to_owned(), "first.json".to_owned()];
        assert_eq!(
            resolve_config_files(std::path::Path::new("."), &configured),
            configured
        );
    }
}
