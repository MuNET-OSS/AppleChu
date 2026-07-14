use windows_sys_loader::Win32::Foundation::COLORREF;

use super::wide;

#[derive(Clone, Copy)]
pub(super) struct Theme {
    pub(super) bg: COLORREF,
    pub(super) text_bg: COLORREF,
    pub(super) text_fg: COLORREF,
    pub(super) dark: bool,
}

const DARK: Theme = Theme {
    bg: 0x002B2B2B,
    text_bg: 0x001E1E1E,
    text_fg: 0x00DCDCDC,
    dark: true,
};

pub(super) const LIGHT: Theme = Theme {
    bg: 0x00F3F3F3,
    text_bg: 0x00FFFFFF,
    text_fg: 0x00202020,
    dark: false,
};

/// 读系统 AppsUseLightTheme：0=深色 非0=浅色（默认浅色）
pub(super) fn current_theme() -> &'static Theme {
    if system_uses_dark() {
        &DARK
    } else {
        &LIGHT
    }
}

fn system_uses_dark() -> bool {
    use windows_sys_loader::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD,
    };
    unsafe {
        let subkey = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
        let value = wide("AppsUseLightTheme");
        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return false;
        }
        let mut data: u32 = 1;
        let mut size = std::mem::size_of::<u32>() as u32;
        let mut value_type = REG_DWORD;
        let ok = RegQueryValueExW(
            hkey,
            value.as_ptr(),
            std::ptr::null_mut(),
            &mut value_type,
            &mut data as *mut u32 as *mut u8,
            &mut size,
        );
        RegCloseKey(hkey);
        ok == 0 && data == 0
    }
}
