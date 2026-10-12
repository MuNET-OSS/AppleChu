use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{option_override, option_target, select_option_path, set_option_override, VfsConfig};

#[test]
fn configured_option_directory_wins_when_non_empty() {
    let root = temporary_directory("configured");
    let configured = root.join("custom-option");
    let fallback = root.join("option");
    fs::create_dir_all(&configured).expect("必须能创建配置目录");
    fs::create_dir_all(&fallback).expect("必须能创建回退目录");
    fs::write(configured.join("present.dat"), b"configured").expect("必须能写入配置资源");
    fs::write(fallback.join("present.dat"), b"fallback").expect("必须能写入回退资源");

    assert_eq!(select_option_path(&root, &configured), configured);

    remove_test_tree(&root);
}

#[test]
fn empty_configured_directory_uses_candidates_in_order() {
    let root = temporary_directory("order");
    let parent = root.parent().expect("临时目录必须有父目录");
    let configured = root.join("custom-option");
    let candidates = [
        root.join("option"),
        parent.join("option"),
        root.join("options"),
        parent.join("options"),
    ];
    fs::create_dir_all(&configured).expect("必须能创建空配置目录");
    for candidate in &candidates {
        fs::create_dir_all(candidate).expect("必须能创建候选目录");
        fs::write(candidate.join("present.dat"), b"resource").expect("必须能写入候选资源");
    }

    assert_eq!(select_option_path(&root, &configured), candidates[0]);
    fs::remove_file(candidates[0].join("present.dat")).expect("必须能删除第一候选资源");
    assert_eq!(select_option_path(&root, &configured), candidates[1]);
    fs::remove_file(candidates[1].join("present.dat")).expect("必须能删除第二候选资源");
    assert_eq!(select_option_path(&root, &configured), candidates[2]);
    fs::remove_file(candidates[2].join("present.dat")).expect("必须能删除第三候选资源");
    assert_eq!(select_option_path(&root, &configured), candidates[3]);

    remove_test_tree(&root);
}

#[test]
fn missing_candidates_keep_configured_option_path() {
    let root = temporary_directory("fallback");
    let configured = root.join("custom-option");

    assert_eq!(select_option_path(&root, &configured), configured);

    remove_test_tree(&root);
}

#[test]
fn option_override_redirects_only_the_mounted_subtree() {
    // Given: 一个注册了重定向的自定义 option 根。
    let config = test_config("override-only-mounted");
    set_option_override("a9z1", "D:\\cache\\ServerResources.d\\A9Z1");

    // Then: 该子树的任何路径都指向解压目录——无论游戏走源路径
    //       (`c:\mount\option\...`) 还是 mount 根 (`<option>\...`) 拼路径。
    let mounted = |rest: &str| {
        std::path::PathBuf::from(format!("D:\\cache\\ServerResources.d\\A9Z1\\{rest}"))
    };
    assert_eq!(
        option_target(&config, "a9z1\\music\\*"),
        mounted("music\\*")
    );
    assert_eq!(
        option_target(&config, "a9z1\\data.conf"),
        mounted("data.conf")
    );
    assert_eq!(
        option_override(&config, "d:\\game\\option\\a9z1\\music"),
        Some(mounted("music"))
    );

    // 其他 option、option 根本身，以及未挂载的选项，行为完全不变。
    assert_eq!(
        option_target(&config, "a001\\music"),
        std::path::PathBuf::from("D:\\game\\option\\a001\\music")
    );
    assert_eq!(
        option_target(&config, ""),
        std::path::PathBuf::from(&config.option)
    );
    assert_eq!(option_override(&config, "d:\\game\\option\\a001"), None);
    assert_eq!(option_override(&config, "d:\\game\\option"), None);

    set_option_override("a9z1", "");
}

#[test]
fn clearing_an_override_restores_the_plain_option_root() {
    // Given: 先注册再注销同名重定向。
    let config = test_config("override-clear");
    set_option_override("a9z2", "D:\\cache\\A9Z2");
    set_option_override("a9z2", "");

    // Then: 完全回到配置的 option 根，行为与没有覆盖时一致。
    assert_eq!(
        option_target(&config, "a9z2\\*"),
        std::path::PathBuf::from("D:\\game\\option\\a9z2\\*")
    );
    assert_eq!(option_override(&config, "d:\\game\\option\\a9z2"), None);
}

/// 测试用的最小 VFS 配置（`option` 指向游戏目录下的 option）
fn test_config(_case: &str) -> VfsConfig {
    VfsConfig {
        amfs: "D:\\game\\amfs\\".to_owned(),
        option: "D:\\game\\option\\".to_owned(),
        appdata: "D:\\game\\appdata\\".to_owned(),
        nthome: "C:\\Users\\Default\\".to_owned(),
        allow_amfs_downloads: false,
    }
}

fn temporary_directory(case: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间必须有效")
        .as_nanos();
    let parent = std::env::temp_dir().join(format!(
        "applechu-vfs-{case}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&parent).expect("必须创建测试父目录");
    parent.join("bin")
}

fn remove_test_tree(root: &std::path::Path) {
    fs::remove_dir_all(root.parent().expect("测试目录必须有父目录")).expect("必须清理测试目录");
}
