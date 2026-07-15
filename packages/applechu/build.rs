// winhttp.dll proxy 构建：转发系统 winhttp.dll 全部导出
// chusanApp.exe 静态导入 winhttp → 双击即自然加载本 DLL
fn main() {
    let system_drive = std::env::var("SYSTEMDRIVE").unwrap_or_else(|_| "C:".to_string());
    let dll_path = format!("{}\\Windows\\System32\\winhttp.dll", system_drive);
    forward_dll::forward_dll(&dll_path).unwrap();
}
