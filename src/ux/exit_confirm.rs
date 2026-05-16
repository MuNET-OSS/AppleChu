use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::config::Config;
use crate::util::api::Api;

type WndProc = unsafe extern "system" fn(usize, u32, usize, isize) -> isize;
type TaskDialogIndirectFn = unsafe extern "system" fn(*const TaskDialogConfig, *mut i32, *mut i32, *mut i32) -> i32;

static ORIGINAL_WNDPROC: AtomicUsize = AtomicUsize::new(0);
static EXIT_CONFIRMED: AtomicBool = AtomicBool::new(false);
static DIALOG_OPEN: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);

const WM_CLOSE: u32 = 0x0010;
const WM_SYSCOMMAND: u32 = 0x0112;
const SC_CLOSE: usize = 0xF060;
const MB_ICONQUESTION: u32 = 0x0000_0020;
const MB_OKCANCEL: u32 = 0x0000_0001;
const MB_DEFBUTTON2: u32 = 0x0000_0100;
const IDOK: i32 = 1;
const TDF_ALLOW_DIALOG_CANCELLATION: u32 = 0x0008;
const TDCBF_CANCEL_BUTTON: u32 = 0x0008;
const TASKDIALOG_BUTTON_EXIT: i32 = 1001;
const GWL_WNDPROC: i32 = -4;

#[link(name = "kernel32")]
extern "system" {
    fn CreateThread(
        attrs: *const c_void,
        stack_size: usize,
        start: Option<unsafe extern "system" fn(*mut c_void) -> u32>,
        param: *const c_void,
        flags: u32,
        id: *mut u32,
    ) -> usize;
    fn GetModuleHandleA(module_name: *const u8) -> usize;
    fn GetProcAddress(module: usize, proc_name: *const u8) -> *const c_void;
    fn Sleep(ms: u32);
    fn CreateActCtxA(ctx: *const ActCtxA) -> usize;
    fn ActivateActCtx(ctx: usize, cookie: *mut usize) -> i32;
    fn DeactivateActCtx(flags: u32, cookie: usize) -> i32;
    fn ReleaseActCtx(ctx: usize) -> i32;
    fn LoadLibraryA(name: *const u8) -> usize;
}

#[repr(C)]
struct ActCtxA {
    cb_size: u32,
    dw_flags: u32,
    lp_source: *const u8,
    w_processor_architecture: u16,
    w_lang_id: u16,
    lp_assembly_directory: *const u8,
    lp_resource_name: *const u8,
    lp_application_name: *const u8,
    h_module: usize,
}

const ACTCTX_FLAG_ASSEMBLY_DIRECTORY_VALID: u32 = 0x004;
const INVALID_HANDLE_VALUE: usize = usize::MAX;

#[link(name = "user32")]
extern "system" {
    fn FindWindowA(class_name: *const u8, window_name: *const u8) -> usize;
    fn CallWindowProcA(prev_wnd_func: usize, hwnd: usize, msg: u32, wp: usize, lp: isize) -> isize;
    fn MessageBoxW(hwnd: usize, text: *const u16, caption: *const u16, flags: u32) -> i32;
    fn PostMessageA(hwnd: usize, msg: u32, wp: usize, lp: isize) -> i32;
    fn SetWindowLongA(hwnd: usize, index: i32, new_long: i32) -> i32;
    fn EnumWindows(callback: unsafe extern "system" fn(usize, isize) -> i32, lparam: isize) -> i32;
    fn GetWindowThreadProcessId(hwnd: usize, process_id: *mut u32) -> u32;
    fn IsWindowVisible(hwnd: usize) -> i32;
    fn GetCurrentProcessId() -> u32;
}

#[repr(C)]
struct TaskDialogButton {
    button_id: i32,
    button_text: *const u16,
}

#[repr(C)]
struct TaskDialogConfig {
    cb_size: u32,
    hwnd_parent: usize,
    instance: usize,
    flags: u32,
    common_buttons: u32,
    window_title: *const u16,
    main_icon: usize,
    main_instruction: *const u16,
    content: *const u16,
    button_count: u32,
    buttons: *const TaskDialogButton,
    default_button: i32,
    radio_button_count: u32,
    radio_buttons: *const c_void,
    default_radio_button: i32,
    verification_text: *const u16,
    expanded_information: *const u16,
    expanded_control_text: *const u16,
    collapsed_control_text: *const u16,
    footer_icon: usize,
    footer: *const u16,
    callback: *const c_void,
    callback_data: isize,
    width: u32,
}

struct DialogParam {
    hwnd: usize,
}

pub fn init(api: &Api, config: &Config) {
    if !config.is_enabled("ExitConfirm") {
        return;
    }

    ENABLED.store(true, Ordering::SeqCst);

    unsafe {
        CreateThread(
            ptr::null(),
            0,
            Some(wndproc_hook_thread),
            ptr::null(),
            0,
            ptr::null_mut(),
        );
    }
    api.log_info("退出确认已启用: 等待游戏窗口创建后替换 WndProc");
}

unsafe extern "system" fn wndproc_hook_thread(_param: *mut c_void) -> u32 {
    if !ENABLED.load(Ordering::SeqCst) {
        return 0;
    }

    let pid = GetCurrentProcessId();
    let mut hwnd = 0usize;
    for _ in 0..300 {
        hwnd = find_visible_window_for_pid(pid);
        if hwnd != 0 {
            break;
        }
        Sleep(100);
    }

    if hwnd == 0 {
        return 0;
    }

    let old = SetWindowLongA(hwnd, GWL_WNDPROC, confirming_wndproc as i32);
    if old != 0 {
        ORIGINAL_WNDPROC.store(old as usize, Ordering::SeqCst);
    }
    0
}

static mut G_ENUM_HWND: usize = 0;
static mut G_ENUM_PID: u32 = 0;

unsafe fn find_visible_window_for_pid(pid: u32) -> usize {
    G_ENUM_HWND = 0;
    G_ENUM_PID = pid;
    EnumWindows(enum_window_cb, 0);
    G_ENUM_HWND
}

unsafe extern "system" fn enum_window_cb(hwnd: usize, _lparam: isize) -> i32 {
    let mut wnd_pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut wnd_pid);
    if wnd_pid == G_ENUM_PID && IsWindowVisible(hwnd) != 0 {
        G_ENUM_HWND = hwnd;
        return 0;
    }
    1
}

unsafe extern "system" fn confirming_wndproc(hwnd: usize, msg: u32, wp: usize, lp: isize) -> isize {
    if should_confirm_close(msg, wp) {
        if EXIT_CONFIRMED.swap(false, Ordering::SeqCst) {
            return call_original_wndproc(hwnd, msg, wp, lp);
        }
        show_confirm_dialog_async(hwnd);
        return 0;
    }
    call_original_wndproc(hwnd, msg, wp, lp)
}

fn should_confirm_close(msg: u32, wp: usize) -> bool {
    msg == WM_CLOSE || (msg == WM_SYSCOMMAND && (wp & 0xFFF0) == SC_CLOSE)
}

unsafe fn show_confirm_dialog_async(hwnd: usize) {
    if DIALOG_OPEN.swap(true, Ordering::SeqCst) {
        return;
    }
    let param = Box::into_raw(Box::new(DialogParam { hwnd }));
    let thread = CreateThread(
        ptr::null(),
        0,
        Some(confirm_dialog_thread),
        param.cast(),
        0,
        ptr::null_mut(),
    );
    if thread == 0 {
        let _ = Box::from_raw(param);
        DIALOG_OPEN.store(false, Ordering::SeqCst);
    }
}

unsafe extern "system" fn confirm_dialog_thread(param: *mut c_void) -> u32 {
    let param = Box::from_raw(param.cast::<DialogParam>());
    let hwnd = param.hwnd;
    let confirmed = match show_task_dialog(hwnd) {
        Some(result) => result,
        None => show_message_box(hwnd),
    };
    if confirmed {
        EXIT_CONFIRMED.store(true, Ordering::SeqCst);
        let _ = PostMessageA(hwnd, WM_CLOSE, 0, 0);
    }
    DIALOG_OPEN.store(false, Ordering::SeqCst);
    0
}

unsafe fn show_task_dialog(hwnd: usize) -> Option<bool> {
    let (_ctx, cookie) = activate_comctl32_v6();

    let comctl32 = LoadLibraryA(b"comctl32.dll\0".as_ptr());
    if comctl32 == 0 {
        deactivate_ctx(cookie);
        return None;
    }
    let proc = GetProcAddress(comctl32, b"TaskDialogIndirect\0".as_ptr());
    if proc.is_null() {
        deactivate_ctx(cookie);
        return None;
    }

    let title = wide_null("AppleChu");
    let instruction = wide_null("\u{786e}\u{8ba4}\u{9000}\u{51fa}\u{6e38}\u{620f}\u{5417}\u{ff1f}");
    let exit_text = wide_null("\u{9000}\u{51fa}");
    let cancel_text = wide_null("\u{53d6}\u{6d88}");
    let buttons = [
        TaskDialogButton {
            button_id: TASKDIALOG_BUTTON_EXIT,
            button_text: exit_text.as_ptr(),
        },
        TaskDialogButton {
            button_id: 2,
            button_text: cancel_text.as_ptr(),
        },
    ];
    let config = TaskDialogConfig {
        cb_size: std::mem::size_of::<TaskDialogConfig>() as u32,
        hwnd_parent: hwnd,
        instance: 0,
        flags: TDF_ALLOW_DIALOG_CANCELLATION,
        common_buttons: 0,
        window_title: title.as_ptr(),
        main_icon: 0,
        main_instruction: instruction.as_ptr(),
        content: ptr::null(),
        button_count: buttons.len() as u32,
        buttons: buttons.as_ptr(),
        default_button: 2,
        radio_button_count: 0,
        radio_buttons: ptr::null(),
        default_radio_button: 0,
        verification_text: ptr::null(),
        expanded_information: ptr::null(),
        expanded_control_text: ptr::null(),
        collapsed_control_text: ptr::null(),
        footer_icon: 0,
        footer: ptr::null(),
        callback: ptr::null(),
        callback_data: 0,
        width: 0,
    };

    let task_dialog_indirect: TaskDialogIndirectFn = std::mem::transmute(proc);
    let mut button = 0;
    let hr = task_dialog_indirect(&config, &mut button, ptr::null_mut(), ptr::null_mut());
    deactivate_ctx(cookie);

    if hr < 0 {
        return None;
    }
    Some(button == TASKDIALOG_BUTTON_EXIT)
}

unsafe fn activate_comctl32_v6() -> (usize, usize) {
    static MANIFEST: &[u8] = b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
        <assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\" manifestVersion=\"1.0\">\r\n\
        <dependency><dependentAssembly>\r\n\
        <assemblyIdentity type=\"win32\" name=\"Microsoft.Windows.Common-Controls\" \
        version=\"6.0.0.0\" processorArchitecture=\"*\" publicKeyToken=\"6595b64144ccf1df\" language=\"*\"/>\r\n\
        </dependentAssembly></dependency>\r\n\
        </assembly>\0";

    let game_dir = std::env::current_dir()
        .map(|p| p.join("AppleChu_comctl6.manifest"))
        .unwrap_or_default();
    let manifest_path_str = format!("{}\0", game_dir.display());

    let _ = std::fs::write(&game_dir, &MANIFEST[..MANIFEST.len() - 1]);

    let act_ctx = ActCtxA {
        cb_size: std::mem::size_of::<ActCtxA>() as u32,
        dw_flags: 0,
        lp_source: manifest_path_str.as_ptr(),
        w_processor_architecture: 0,
        w_lang_id: 0,
        lp_assembly_directory: ptr::null(),
        lp_resource_name: ptr::null(),
        lp_application_name: ptr::null(),
        h_module: 0,
    };

    let ctx = CreateActCtxA(&act_ctx);
    let _ = std::fs::remove_file(&game_dir);

    if ctx == INVALID_HANDLE_VALUE || ctx == 0 {
        return (0, 0);
    }

    let mut cookie = 0usize;
    if ActivateActCtx(ctx, &mut cookie) == 0 {
        ReleaseActCtx(ctx);
        return (0, 0);
    }

    (ctx, cookie)
}

unsafe fn deactivate_ctx(cookie: usize) {
    if cookie != 0 {
        DeactivateActCtx(0, cookie);
    }
}

unsafe fn show_message_box(hwnd: usize) -> bool {
    let text = wide_null("\u{786e}\u{8ba4}\u{9000}\u{51fa}\u{6e38}\u{620f}\u{5417}\u{ff1f}");
    let caption = wide_null("AppleChu");
    MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_ICONQUESTION | MB_OKCANCEL | MB_DEFBUTTON2) == IDOK
}

unsafe fn call_original_wndproc(hwnd: usize, msg: u32, wp: usize, lp: isize) -> isize {
    let original = ORIGINAL_WNDPROC.load(Ordering::SeqCst);
    if original == 0 {
        return 0;
    }
    CallWindowProcA(original, hwnd, msg, wp, lp)
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
