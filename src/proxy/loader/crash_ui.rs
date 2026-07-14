mod theme;
mod window;

use std::sync::OnceLock;

static CRASH_DIR: OnceLock<Vec<u16>> = OnceLock::new();

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 弹出崩溃报告窗口（深色主题、等宽字体、可滚动/复制 + Copy/Open Folder/Exit 按钮）
/// 阻塞直到用户关闭，之后进程随崩溃退出
pub fn show(report: &str, crash_dir: &str, zip_path: Option<&str>) {
    let _ = CRASH_DIR.set(wide(crash_dir));

    let mut body = String::new();
    body.push_str("Chusan has crashed Nya...! (>_<)\r\n");
    body.push_str(&"-".repeat(60));
    body.push_str("\r\n\r\n");
    body.push_str(&report.replace('\n', "\r\n"));
    body.push_str("\r\n\r\nA crash report was saved to:\r\n  ");
    body.push_str(crash_dir);
    if let Some(zip) = zip_path {
        body.push_str("\r\n\r\nPlease send this zip if you need help Nya~\r\n  ");
        body.push_str(zip);
    }

    unsafe { window::run_window(&body) };
}
