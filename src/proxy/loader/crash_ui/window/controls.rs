use super::*;

pub(super) unsafe fn create_text_box(
    parent: HWND,
    hinstance: windows_sys_loader::Win32::Foundation::HMODULE,
    body: &str,
) -> HWND {
    let class = wide("EDIT");
    let hwnd = CreateWindowExW(
        0,
        class.as_ptr(),
        std::ptr::null(),
        WS_CHILD | WS_VISIBLE | WS_VSCROLL | ES_MULTILINE | ES_AUTOVSCROLL | ES_READONLY,
        0,
        0,
        100,
        100,
        parent,
        ID_TEXT as _,
        hinstance,
        std::ptr::null(),
    );
    let text = wide(body);
    SetWindowTextW(hwnd, text.as_ptr());
    if !UI_FONT.is_null() {
        SendMessageW(hwnd, WM_SETFONT, UI_FONT as WPARAM, 1);
    }
    hwnd
}

pub(super) unsafe fn create_button(
    parent: HWND,
    hinstance: windows_sys_loader::Win32::Foundation::HMODULE,
    label: &str,
    id: isize,
) {
    let class = wide("BUTTON");
    let text = wide(label);
    let hwnd = CreateWindowExW(
        0,
        class.as_ptr(),
        text.as_ptr(),
        WS_CHILD | WS_VISIBLE,
        0,
        0,
        100,
        BTN_H,
        parent,
        id as _,
        hinstance,
        std::ptr::null(),
    );
    if !UI_FONT.is_null() {
        SendMessageW(hwnd, WM_SETFONT, UI_FONT as WPARAM, 1);
    }
}

pub(super) unsafe fn layout(hwnd: HWND) {
    let mut rect: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;

    let text_height = height - BTN_H - MARGIN * 3;
    if !TEXT_HWND.is_null() {
        MoveWindow(
            TEXT_HWND,
            MARGIN,
            MARGIN,
            width - MARGIN * 2,
            text_height,
            1,
        );
    }

    let button_width = 130;
    let button_y = height - MARGIN - BTN_H;
    let mut x = MARGIN;
    for id in [ID_COPY, ID_OPEN, ID_EXIT] {
        if let Some(button) = find_child(hwnd, id) {
            MoveWindow(button, x, button_y, button_width, BTN_H, 1);
        }
        x += button_width + BTN_GAP;
    }
}

unsafe fn find_child(parent: HWND, id: isize) -> Option<HWND> {
    let child = windows_sys_loader::Win32::UI::WindowsAndMessaging::GetDlgItem(parent, id as i32);
    (!child.is_null()).then_some(child)
}
