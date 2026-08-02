use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use super::select_option_path;

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
