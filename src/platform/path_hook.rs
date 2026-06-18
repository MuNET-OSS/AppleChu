use std::ffi::{CStr, c_char, c_void};
use std::ptr;
use std::sync::Mutex;

use crate::iohook::hook_table::{HookSymbol, hook_table_apply, null_module};
use crate::platform::winapi;

const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const INVALID_HANDLE_VALUE: usize = usize::MAX;
const KERNEL32_DLL: &str = "kernel32.dll";
const SHLWAPI_DLL: &str = "shlwapi.dll";

type PathTransform = fn(&str) -> Option<String>;

type CreateFileAFn =
    unsafe extern "system" fn(*const c_char, u32, u32, *const c_void, u32, u32, usize) -> usize;
type CreateFileWFn =
    unsafe extern "system" fn(*const u16, u32, u32, *const c_void, u32, u32, usize) -> usize;
type GetFileAttributesAFn = unsafe extern "system" fn(*const c_char) -> u32;
type GetFileAttributesWFn = unsafe extern "system" fn(*const u16) -> u32;
type GetFileAttributesExAFn = unsafe extern "system" fn(*const c_char, u32, *mut c_void) -> i32;
type GetFileAttributesExWFn = unsafe extern "system" fn(*const u16, u32, *mut c_void) -> i32;
type FindFirstFileAFn = unsafe extern "system" fn(*const c_char, *mut c_void) -> usize;
type FindFirstFileWFn = unsafe extern "system" fn(*const u16, *mut c_void) -> usize;
type FindFirstFileExAFn =
    unsafe extern "system" fn(*const c_char, u32, *mut c_void, u32, *mut c_void, u32) -> usize;
type FindFirstFileExWFn =
    unsafe extern "system" fn(*const u16, u32, *mut c_void, u32, *mut c_void, u32) -> usize;
type PathFileExistsAFn = unsafe extern "system" fn(*const c_char) -> i32;
type PathFileExistsWFn = unsafe extern "system" fn(*const u16) -> i32;
type CreateDirectoryAFn = unsafe extern "system" fn(*const c_char, *const c_void) -> i32;
type CreateDirectoryWFn = unsafe extern "system" fn(*const u16, *const c_void) -> i32;
type GetDriveTypeAFn = unsafe extern "system" fn(*const c_char) -> u32;
type GetDriveTypeWFn = unsafe extern "system" fn(*const u16) -> u32;
type DeleteFileAFn = unsafe extern "system" fn(*const c_char) -> i32;
type DeleteFileWFn = unsafe extern "system" fn(*const u16) -> i32;
type GetPrivateProfileStringAFn = unsafe extern "system" fn(
    *const c_char,
    *const c_char,
    *const c_char,
    *mut c_char,
    u32,
    *const c_char,
) -> u32;
type GetPrivateProfileStringWFn =
    unsafe extern "system" fn(*const u16, *const u16, *const u16, *mut u16, u32, *const u16) -> u32;

static CALLBACKS: Mutex<Vec<PathTransform>> = Mutex::new(Vec::new());
static INIT_LOCK: Mutex<bool> = Mutex::new(false);

static mut NEXT_CREATE_FILE_A: *const () = ptr::null();
static mut NEXT_CREATE_FILE_W: *const () = ptr::null();
static mut NEXT_GET_FILE_ATTRIBUTES_A: *const () = ptr::null();
static mut NEXT_GET_FILE_ATTRIBUTES_W: *const () = ptr::null();
static mut NEXT_GET_FILE_ATTRIBUTES_EX_A: *const () = ptr::null();
static mut NEXT_GET_FILE_ATTRIBUTES_EX_W: *const () = ptr::null();
static mut NEXT_FIND_FIRST_FILE_A: *const () = ptr::null();
static mut NEXT_FIND_FIRST_FILE_W: *const () = ptr::null();
static mut NEXT_FIND_FIRST_FILE_EX_A: *const () = ptr::null();
static mut NEXT_FIND_FIRST_FILE_EX_W: *const () = ptr::null();
static mut NEXT_PATH_FILE_EXISTS_A: *const () = ptr::null();
static mut NEXT_PATH_FILE_EXISTS_W: *const () = ptr::null();
static mut NEXT_CREATE_DIRECTORY_A: *const () = ptr::null();
static mut NEXT_CREATE_DIRECTORY_W: *const () = ptr::null();
static mut NEXT_GET_DRIVE_TYPE_A: *const () = ptr::null();
static mut NEXT_GET_DRIVE_TYPE_W: *const () = ptr::null();
static mut NEXT_DELETE_FILE_A: *const () = ptr::null();
static mut NEXT_DELETE_FILE_W: *const () = ptr::null();
static mut NEXT_GET_PRIVATE_PROFILE_STRING_A: *const () = ptr::null();
static mut NEXT_GET_PRIVATE_PROFILE_STRING_W: *const () = ptr::null();

pub fn push(callback: PathTransform) {
    init();
    if let Ok(mut callbacks) = CALLBACKS.lock() {
        callbacks.push(callback);
    }
}

pub fn init() {
    let Ok(mut initialized) = INIT_LOCK.lock() else {
        return;
    };
    if *initialized {
        return;
    }

    unsafe {
        let symbols = [
            HookSymbol {
                name: "CreateFileA",
                patch: hooked_create_file_a as *const (),
                original: ptr::addr_of_mut!(NEXT_CREATE_FILE_A),
            },
            HookSymbol {
                name: "CreateFileW",
                patch: hooked_create_file_w as *const (),
                original: ptr::addr_of_mut!(NEXT_CREATE_FILE_W),
            },
            HookSymbol {
                name: "GetFileAttributesA",
                patch: hooked_get_file_attributes_a as *const (),
                original: ptr::addr_of_mut!(NEXT_GET_FILE_ATTRIBUTES_A),
            },
            HookSymbol {
                name: "GetFileAttributesW",
                patch: hooked_get_file_attributes_w as *const (),
                original: ptr::addr_of_mut!(NEXT_GET_FILE_ATTRIBUTES_W),
            },
            HookSymbol {
                name: "GetFileAttributesExA",
                patch: hooked_get_file_attributes_ex_a as *const (),
                original: ptr::addr_of_mut!(NEXT_GET_FILE_ATTRIBUTES_EX_A),
            },
            HookSymbol {
                name: "GetFileAttributesExW",
                patch: hooked_get_file_attributes_ex_w as *const (),
                original: ptr::addr_of_mut!(NEXT_GET_FILE_ATTRIBUTES_EX_W),
            },
            HookSymbol {
                name: "FindFirstFileA",
                patch: hooked_find_first_file_a as *const (),
                original: ptr::addr_of_mut!(NEXT_FIND_FIRST_FILE_A),
            },
            HookSymbol {
                name: "FindFirstFileW",
                patch: hooked_find_first_file_w as *const (),
                original: ptr::addr_of_mut!(NEXT_FIND_FIRST_FILE_W),
            },
            HookSymbol {
                name: "FindFirstFileExA",
                patch: hooked_find_first_file_ex_a as *const (),
                original: ptr::addr_of_mut!(NEXT_FIND_FIRST_FILE_EX_A),
            },
            HookSymbol {
                name: "FindFirstFileExW",
                patch: hooked_find_first_file_ex_w as *const (),
                original: ptr::addr_of_mut!(NEXT_FIND_FIRST_FILE_EX_W),
            },
            HookSymbol {
                name: "CreateDirectoryA",
                patch: hooked_create_directory_a as *const (),
                original: ptr::addr_of_mut!(NEXT_CREATE_DIRECTORY_A),
            },
            HookSymbol {
                name: "CreateDirectoryW",
                patch: hooked_create_directory_w as *const (),
                original: ptr::addr_of_mut!(NEXT_CREATE_DIRECTORY_W),
            },
            HookSymbol {
                name: "GetDriveTypeA",
                patch: hooked_get_drive_type_a as *const (),
                original: ptr::addr_of_mut!(NEXT_GET_DRIVE_TYPE_A),
            },
            HookSymbol {
                name: "GetDriveTypeW",
                patch: hooked_get_drive_type_w as *const (),
                original: ptr::addr_of_mut!(NEXT_GET_DRIVE_TYPE_W),
            },
            HookSymbol {
                name: "DeleteFileA",
                patch: hooked_delete_file_a as *const (),
                original: ptr::addr_of_mut!(NEXT_DELETE_FILE_A),
            },
            HookSymbol {
                name: "DeleteFileW",
                patch: hooked_delete_file_w as *const (),
                original: ptr::addr_of_mut!(NEXT_DELETE_FILE_W),
            },
            HookSymbol {
                name: "GetPrivateProfileStringA",
                patch: hooked_get_private_profile_string_a as *const (),
                original: ptr::addr_of_mut!(NEXT_GET_PRIVATE_PROFILE_STRING_A),
            },
            HookSymbol {
                name: "GetPrivateProfileStringW",
                patch: hooked_get_private_profile_string_w as *const (),
                original: ptr::addr_of_mut!(NEXT_GET_PRIVATE_PROFILE_STRING_W),
            },
        ];
        hook_table_apply(null_module(), KERNEL32_DLL, &symbols);
        let shlwapi_symbols = [
            HookSymbol {
                name: "PathFileExistsA",
                patch: hooked_path_file_exists_a as *const (),
                original: ptr::addr_of_mut!(NEXT_PATH_FILE_EXISTS_A),
            },
            HookSymbol {
                name: "PathFileExistsW",
                patch: hooked_path_file_exists_w as *const (),
                original: ptr::addr_of_mut!(NEXT_PATH_FILE_EXISTS_W),
            },
        ];
        hook_table_apply(null_module(), SHLWAPI_DLL, &shlwapi_symbols);
        if let Some(api) = crate::util::api::API.get() {
            api.log_info(&format!(
                "path_hook: NEXT_CreateFileA={:#x}, hooked_create_file_a={:#x}",
                NEXT_CREATE_FILE_A as usize, hooked_create_file_a as *const () as usize
            ));
        }
    }

    *initialized = true;
}

fn transform_path(path: &str) -> Option<String> {
    let Ok(callbacks) = CALLBACKS.lock() else {
        return None;
    };
    callbacks.iter().find_map(|callback| callback(path))
}

unsafe fn transform_path_a(path: *const c_char) -> Option<String> {
    if path.is_null() {
        return None;
    }
    let path = CStr::from_ptr(path).to_string_lossy();
    transform_path(&path)
}

unsafe fn transform_path_w(path: *const u16) -> Option<String> {
    winapi::wide_to_string(path).and_then(|path| transform_path(&path))
}

unsafe extern "system" fn hooked_create_file_a(
    path: *const c_char,
    access: u32,
    share: u32,
    security: *const c_void,
    creation: u32,
    flags: u32,
    template: usize,
) -> usize {
    let Some(next) = cast_next::<CreateFileAFn>(NEXT_CREATE_FILE_A) else {
        return INVALID_HANDLE_VALUE;
    };
    if let Some(transformed) = transform_path_a(path) {
        let path = winapi::to_cstring_lossy(&transformed);
        return next(
            path.as_ptr(),
            access,
            share,
            security,
            creation,
            flags,
            template,
        );
    }
    next(path, access, share, security, creation, flags, template)
}

unsafe extern "system" fn hooked_create_file_w(
    path: *const u16,
    access: u32,
    share: u32,
    security: *const c_void,
    creation: u32,
    flags: u32,
    template: usize,
) -> usize {
    let Some(next) = cast_next::<CreateFileWFn>(NEXT_CREATE_FILE_W) else {
        return INVALID_HANDLE_VALUE;
    };
    if let Some(transformed) = transform_path_w(path) {
        let path = winapi::string_to_wide(&transformed);
        return next(
            path.as_ptr(),
            access,
            share,
            security,
            creation,
            flags,
            template,
        );
    }
    next(path, access, share, security, creation, flags, template)
}

unsafe extern "system" fn hooked_get_file_attributes_a(path: *const c_char) -> u32 {
    let Some(next) = cast_next::<GetFileAttributesAFn>(NEXT_GET_FILE_ATTRIBUTES_A) else {
        return INVALID_FILE_ATTRIBUTES;
    };
    if let Some(transformed) = transform_path_a(path) {
        let path = winapi::to_cstring_lossy(&transformed);
        return next(path.as_ptr());
    }
    next(path)
}

unsafe extern "system" fn hooked_get_file_attributes_w(path: *const u16) -> u32 {
    let Some(next) = cast_next::<GetFileAttributesWFn>(NEXT_GET_FILE_ATTRIBUTES_W) else {
        return INVALID_FILE_ATTRIBUTES;
    };
    if let Some(transformed) = transform_path_w(path) {
        let path = winapi::string_to_wide(&transformed);
        return next(path.as_ptr());
    }
    next(path)
}

unsafe extern "system" fn hooked_get_file_attributes_ex_a(
    path: *const c_char,
    level: u32,
    data: *mut c_void,
) -> i32 {
    let Some(next) = cast_next::<GetFileAttributesExAFn>(NEXT_GET_FILE_ATTRIBUTES_EX_A) else {
        return 0;
    };
    if let Some(transformed) = transform_path_a(path) {
        let path = winapi::to_cstring_lossy(&transformed);
        return next(path.as_ptr(), level, data);
    }
    next(path, level, data)
}

unsafe extern "system" fn hooked_get_file_attributes_ex_w(
    path: *const u16,
    level: u32,
    data: *mut c_void,
) -> i32 {
    let Some(next) = cast_next::<GetFileAttributesExWFn>(NEXT_GET_FILE_ATTRIBUTES_EX_W) else {
        return 0;
    };
    if let Some(transformed) = transform_path_w(path) {
        let path = winapi::string_to_wide(&transformed);
        return next(path.as_ptr(), level, data);
    }
    next(path, level, data)
}

unsafe extern "system" fn hooked_find_first_file_a(
    path: *const c_char,
    data: *mut c_void,
) -> usize {
    let Some(next) = cast_next::<FindFirstFileAFn>(NEXT_FIND_FIRST_FILE_A) else {
        return INVALID_HANDLE_VALUE;
    };
    if let Some(transformed) = transform_path_a(path) {
        let path = winapi::to_cstring_lossy(&transformed);
        return next(path.as_ptr(), data);
    }
    next(path, data)
}

unsafe extern "system" fn hooked_find_first_file_w(path: *const u16, data: *mut c_void) -> usize {
    let Some(next) = cast_next::<FindFirstFileWFn>(NEXT_FIND_FIRST_FILE_W) else {
        return INVALID_HANDLE_VALUE;
    };
    if let Some(transformed) = transform_path_w(path) {
        let path = winapi::string_to_wide(&transformed);
        return next(path.as_ptr(), data);
    }
    next(path, data)
}

unsafe extern "system" fn hooked_find_first_file_ex_a(
    path: *const c_char,
    info_level: u32,
    found: *mut c_void,
    search_op: u32,
    filter: *mut c_void,
    flags: u32,
) -> usize {
    let Some(next) = cast_next::<FindFirstFileExAFn>(NEXT_FIND_FIRST_FILE_EX_A) else {
        return INVALID_HANDLE_VALUE;
    };
    if let Some(transformed) = transform_path_a(path) {
        let path = winapi::to_cstring_lossy(&transformed);
        return next(path.as_ptr(), info_level, found, search_op, filter, flags);
    }
    next(path, info_level, found, search_op, filter, flags)
}

unsafe extern "system" fn hooked_find_first_file_ex_w(
    path: *const u16,
    info_level: u32,
    found: *mut c_void,
    search_op: u32,
    filter: *mut c_void,
    flags: u32,
) -> usize {
    let Some(next) = cast_next::<FindFirstFileExWFn>(NEXT_FIND_FIRST_FILE_EX_W) else {
        return INVALID_HANDLE_VALUE;
    };
    if let Some(transformed) = transform_path_w(path) {
        let path = winapi::string_to_wide(&transformed);
        return next(path.as_ptr(), info_level, found, search_op, filter, flags);
    }
    next(path, info_level, found, search_op, filter, flags)
}

unsafe extern "system" fn hooked_path_file_exists_a(path: *const c_char) -> i32 {
    let Some(next) = cast_next::<PathFileExistsAFn>(NEXT_PATH_FILE_EXISTS_A) else {
        return 0;
    };
    if let Some(transformed) = transform_path_a(path) {
        let path = winapi::to_cstring_lossy(&transformed);
        return next(path.as_ptr());
    }
    next(path)
}

unsafe extern "system" fn hooked_path_file_exists_w(path: *const u16) -> i32 {
    let Some(next) = cast_next::<PathFileExistsWFn>(NEXT_PATH_FILE_EXISTS_W) else {
        return 0;
    };
    if let Some(transformed) = transform_path_w(path) {
        let path = winapi::string_to_wide(&transformed);
        return next(path.as_ptr());
    }
    next(path)
}

unsafe extern "system" fn hooked_create_directory_a(
    path: *const c_char,
    security: *const c_void,
) -> i32 {
    let Some(next) = cast_next::<CreateDirectoryAFn>(NEXT_CREATE_DIRECTORY_A) else {
        return 0;
    };
    if let Some(transformed) = transform_path_a(path) {
        let path = winapi::to_cstring_lossy(&transformed);
        return next(path.as_ptr(), security);
    }
    next(path, security)
}

unsafe extern "system" fn hooked_create_directory_w(
    path: *const u16,
    security: *const c_void,
) -> i32 {
    let Some(next) = cast_next::<CreateDirectoryWFn>(NEXT_CREATE_DIRECTORY_W) else {
        return 0;
    };
    if let Some(transformed) = transform_path_w(path) {
        let path = winapi::string_to_wide(&transformed);
        return next(path.as_ptr(), security);
    }
    next(path, security)
}

unsafe extern "system" fn hooked_get_drive_type_a(path: *const c_char) -> u32 {
    let Some(next) = cast_next::<GetDriveTypeAFn>(NEXT_GET_DRIVE_TYPE_A) else {
        return 0;
    };
    if let Some(transformed) = transform_path_a(path) {
        let path = winapi::to_cstring_lossy(&transformed);
        return next(path.as_ptr());
    }
    next(path)
}

unsafe extern "system" fn hooked_get_drive_type_w(path: *const u16) -> u32 {
    let Some(next) = cast_next::<GetDriveTypeWFn>(NEXT_GET_DRIVE_TYPE_W) else {
        return 0;
    };
    if let Some(transformed) = transform_path_w(path) {
        let path = winapi::string_to_wide(&transformed);
        return next(path.as_ptr());
    }
    next(path)
}

unsafe extern "system" fn hooked_delete_file_a(path: *const c_char) -> i32 {
    let Some(next) = cast_next::<DeleteFileAFn>(NEXT_DELETE_FILE_A) else {
        return 0;
    };
    if let Some(transformed) = transform_path_a(path) {
        let path = winapi::to_cstring_lossy(&transformed);
        return next(path.as_ptr());
    }
    next(path)
}

unsafe extern "system" fn hooked_delete_file_w(path: *const u16) -> i32 {
    let Some(next) = cast_next::<DeleteFileWFn>(NEXT_DELETE_FILE_W) else {
        return 0;
    };
    if let Some(transformed) = transform_path_w(path) {
        let path = winapi::string_to_wide(&transformed);
        return next(path.as_ptr());
    }
    next(path)
}

unsafe extern "system" fn hooked_get_private_profile_string_a(
    app: *const c_char,
    key: *const c_char,
    default: *const c_char,
    out: *mut c_char,
    size: u32,
    path: *const c_char,
) -> u32 {
    let Some(next) = cast_next::<GetPrivateProfileStringAFn>(NEXT_GET_PRIVATE_PROFILE_STRING_A)
    else {
        return 0;
    };
    if let Some(transformed) = transform_path_a(path) {
        let path = winapi::to_cstring_lossy(&transformed);
        return next(app, key, default, out, size, path.as_ptr());
    }
    next(app, key, default, out, size, path)
}

unsafe extern "system" fn hooked_get_private_profile_string_w(
    app: *const u16,
    key: *const u16,
    default: *const u16,
    out: *mut u16,
    size: u32,
    path: *const u16,
) -> u32 {
    let Some(next) = cast_next::<GetPrivateProfileStringWFn>(NEXT_GET_PRIVATE_PROFILE_STRING_W)
    else {
        return 0;
    };
    if let Some(transformed) = transform_path_w(path) {
        let path = winapi::string_to_wide(&transformed);
        return next(app, key, default, out, size, path.as_ptr());
    }
    next(app, key, default, out, size, path)
}

unsafe fn cast_next<T>(next: *const ()) -> Option<T> {
    (!next.is_null()).then(|| std::mem::transmute_copy(&next))
}
