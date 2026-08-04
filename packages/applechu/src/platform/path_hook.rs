use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::Mutex;

use crate::iohook::hook_table::{hook_table_apply, null_module, HookSymbol};
use crate::platform::winapi;
use windows_sys::Win32::Foundation::HMODULE;

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
type CreateDirectoryExAFn =
    unsafe extern "system" fn(*const c_char, *const c_char, *const c_void) -> i32;
type CreateDirectoryExWFn = unsafe extern "system" fn(*const u16, *const u16, *const c_void) -> i32;
type SinglePathAFn = unsafe extern "system" fn(*const c_char) -> i32;
type SinglePathWFn = unsafe extern "system" fn(*const u16) -> i32;
type TwoPathAFn = unsafe extern "system" fn(*const c_char, *const c_char) -> i32;
type TwoPathWFn = unsafe extern "system" fn(*const u16, *const u16) -> i32;
type CopyFileAFn = unsafe extern "system" fn(*const c_char, *const c_char, i32) -> i32;
type CopyFileWFn = unsafe extern "system" fn(*const u16, *const u16, i32) -> i32;
type MoveFileExAFn = unsafe extern "system" fn(*const c_char, *const c_char, u32) -> i32;
type MoveFileExWFn = unsafe extern "system" fn(*const u16, *const u16, u32) -> i32;
type CopyFileExAFn = unsafe extern "system" fn(
    *const c_char,
    *const c_char,
    *const c_void,
    *mut c_void,
    *mut i32,
    u32,
) -> i32;
type CopyFileExWFn = unsafe extern "system" fn(
    *const u16,
    *const u16,
    *const c_void,
    *mut c_void,
    *mut i32,
    u32,
) -> i32;
type ReplaceFileAFn = unsafe extern "system" fn(
    *const c_char,
    *const c_char,
    *const c_char,
    u32,
    *mut c_void,
    *mut c_void,
) -> i32;
type ReplaceFileWFn = unsafe extern "system" fn(
    *const u16,
    *const u16,
    *const u16,
    u32,
    *mut c_void,
    *mut c_void,
) -> i32;
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
type GetPrivateProfileSectionWFn =
    unsafe extern "system" fn(*const u16, *mut u16, u32, *const u16) -> u32;

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
static mut NEXT_CREATE_DIRECTORY_EX_A: *const () = ptr::null();
static mut NEXT_CREATE_DIRECTORY_EX_W: *const () = ptr::null();
static mut NEXT_REMOVE_DIRECTORY_A: *const () = ptr::null();
static mut NEXT_REMOVE_DIRECTORY_W: *const () = ptr::null();
static mut NEXT_MOVE_FILE_A: *const () = ptr::null();
static mut NEXT_MOVE_FILE_W: *const () = ptr::null();
static mut NEXT_COPY_FILE_A: *const () = ptr::null();
static mut NEXT_COPY_FILE_W: *const () = ptr::null();
static mut NEXT_MOVE_FILE_EX_A: *const () = ptr::null();
static mut NEXT_MOVE_FILE_EX_W: *const () = ptr::null();
static mut NEXT_COPY_FILE_EX_A: *const () = ptr::null();
static mut NEXT_COPY_FILE_EX_W: *const () = ptr::null();
static mut NEXT_REPLACE_FILE_A: *const () = ptr::null();
static mut NEXT_REPLACE_FILE_W: *const () = ptr::null();
static mut NEXT_GET_DRIVE_TYPE_A: *const () = ptr::null();
static mut NEXT_GET_DRIVE_TYPE_W: *const () = ptr::null();
static mut NEXT_DELETE_FILE_A: *const () = ptr::null();
static mut NEXT_DELETE_FILE_W: *const () = ptr::null();
static mut NEXT_GET_PRIVATE_PROFILE_STRING_A: *const () = ptr::null();
static mut NEXT_GET_PRIVATE_PROFILE_STRING_W: *const () = ptr::null();
static mut NEXT_GET_PRIVATE_PROFILE_SECTION_W: *const () = ptr::null();

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
        let symbols = kernel_symbols();
        hook_table_apply(null_module(), KERNEL32_DLL, &symbols);
        let shlwapi_symbols = shlwapi_symbols();
        hook_table_apply(null_module(), SHLWAPI_DLL, &shlwapi_symbols);
        if let Some(api) = crate::util::api::API.get() {
            api.log_info("Path compatibility ready");
        }
    }

    *initialized = true;
}

/// 对后加载的 Thinca DLL 补装路径 Hook
pub unsafe fn apply_hooks(module: HMODULE) -> usize {
    init();
    let mut patched = 0;
    let symbols = kernel_symbols();
    patched += hook_table_apply(module, KERNEL32_DLL, &symbols);
    let symbols = shlwapi_symbols();
    patched += hook_table_apply(module, SHLWAPI_DLL, &symbols);
    patched
}

unsafe fn kernel_symbols() -> [HookSymbol; 33] {
    [
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
            name: "CreateDirectoryExA",
            patch: hooked_create_directory_ex_a as *const (),
            original: ptr::addr_of_mut!(NEXT_CREATE_DIRECTORY_EX_A),
        },
        HookSymbol {
            name: "CreateDirectoryExW",
            patch: hooked_create_directory_ex_w as *const (),
            original: ptr::addr_of_mut!(NEXT_CREATE_DIRECTORY_EX_W),
        },
        HookSymbol {
            name: "RemoveDirectoryA",
            patch: hooked_remove_directory_a as *const (),
            original: ptr::addr_of_mut!(NEXT_REMOVE_DIRECTORY_A),
        },
        HookSymbol {
            name: "RemoveDirectoryW",
            patch: hooked_remove_directory_w as *const (),
            original: ptr::addr_of_mut!(NEXT_REMOVE_DIRECTORY_W),
        },
        HookSymbol {
            name: "MoveFileA",
            patch: hooked_move_file_a as *const (),
            original: ptr::addr_of_mut!(NEXT_MOVE_FILE_A),
        },
        HookSymbol {
            name: "MoveFileW",
            patch: hooked_move_file_w as *const (),
            original: ptr::addr_of_mut!(NEXT_MOVE_FILE_W),
        },
        HookSymbol {
            name: "CopyFileA",
            patch: hooked_copy_file_a as *const (),
            original: ptr::addr_of_mut!(NEXT_COPY_FILE_A),
        },
        HookSymbol {
            name: "CopyFileW",
            patch: hooked_copy_file_w as *const (),
            original: ptr::addr_of_mut!(NEXT_COPY_FILE_W),
        },
        HookSymbol {
            name: "MoveFileExA",
            patch: hooked_move_file_ex_a as *const (),
            original: ptr::addr_of_mut!(NEXT_MOVE_FILE_EX_A),
        },
        HookSymbol {
            name: "MoveFileExW",
            patch: hooked_move_file_ex_w as *const (),
            original: ptr::addr_of_mut!(NEXT_MOVE_FILE_EX_W),
        },
        HookSymbol {
            name: "CopyFileExA",
            patch: hooked_copy_file_ex_a as *const (),
            original: ptr::addr_of_mut!(NEXT_COPY_FILE_EX_A),
        },
        HookSymbol {
            name: "CopyFileExW",
            patch: hooked_copy_file_ex_w as *const (),
            original: ptr::addr_of_mut!(NEXT_COPY_FILE_EX_W),
        },
        HookSymbol {
            name: "ReplaceFileA",
            patch: hooked_replace_file_a as *const (),
            original: ptr::addr_of_mut!(NEXT_REPLACE_FILE_A),
        },
        HookSymbol {
            name: "ReplaceFileW",
            patch: hooked_replace_file_w as *const (),
            original: ptr::addr_of_mut!(NEXT_REPLACE_FILE_W),
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
        HookSymbol {
            name: "GetPrivateProfileSectionW",
            patch: hooked_get_private_profile_section_w as *const (),
            original: ptr::addr_of_mut!(NEXT_GET_PRIVATE_PROFILE_SECTION_W),
        },
    ]
}

unsafe fn shlwapi_symbols() -> [HookSymbol; 2] {
    [
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
    ]
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

unsafe extern "system" fn hooked_create_directory_ex_a(
    template: *const c_char,
    path: *const c_char,
    security: *const c_void,
) -> i32 {
    let Some(next) = cast_next::<CreateDirectoryExAFn>(NEXT_CREATE_DIRECTORY_EX_A) else {
        return 0;
    };
    let template_value = transform_path_a(template).map(|value| winapi::to_cstring_lossy(&value));
    let path_value = transform_path_a(path).map(|value| winapi::to_cstring_lossy(&value));
    next(
        template_value
            .as_ref()
            .map_or(template, |value| value.as_ptr()),
        path_value.as_ref().map_or(path, |value| value.as_ptr()),
        security,
    )
}

unsafe extern "system" fn hooked_create_directory_ex_w(
    template: *const u16,
    path: *const u16,
    security: *const c_void,
) -> i32 {
    let Some(next) = cast_next::<CreateDirectoryExWFn>(NEXT_CREATE_DIRECTORY_EX_W) else {
        return 0;
    };
    let template_value = transform_path_w(template).map(|value| winapi::string_to_wide(&value));
    let path_value = transform_path_w(path).map(|value| winapi::string_to_wide(&value));
    next(
        template_value
            .as_ref()
            .map_or(template, |value| value.as_ptr()),
        path_value.as_ref().map_or(path, |value| value.as_ptr()),
        security,
    )
}

unsafe extern "system" fn hooked_remove_directory_a(path: *const c_char) -> i32 {
    call_single_path_a(NEXT_REMOVE_DIRECTORY_A, path)
}

unsafe extern "system" fn hooked_remove_directory_w(path: *const u16) -> i32 {
    call_single_path_w(NEXT_REMOVE_DIRECTORY_W, path)
}

unsafe extern "system" fn hooked_move_file_a(
    source: *const c_char,
    destination: *const c_char,
) -> i32 {
    call_two_paths_a(NEXT_MOVE_FILE_A, source, destination)
}

unsafe extern "system" fn hooked_move_file_w(source: *const u16, destination: *const u16) -> i32 {
    call_two_paths_w(NEXT_MOVE_FILE_W, source, destination)
}

unsafe extern "system" fn hooked_copy_file_a(
    source: *const c_char,
    destination: *const c_char,
    fail_if_exists: i32,
) -> i32 {
    let Some(next) = cast_next::<CopyFileAFn>(NEXT_COPY_FILE_A) else {
        return 0;
    };
    let source_value = transform_path_a(source).map(|value| winapi::to_cstring_lossy(&value));
    let destination_value =
        transform_path_a(destination).map(|value| winapi::to_cstring_lossy(&value));
    next(
        source_value.as_ref().map_or(source, |value| value.as_ptr()),
        destination_value
            .as_ref()
            .map_or(destination, |value| value.as_ptr()),
        fail_if_exists,
    )
}

unsafe extern "system" fn hooked_copy_file_w(
    source: *const u16,
    destination: *const u16,
    fail_if_exists: i32,
) -> i32 {
    let Some(next) = cast_next::<CopyFileWFn>(NEXT_COPY_FILE_W) else {
        return 0;
    };
    let source_value = transform_path_w(source).map(|value| winapi::string_to_wide(&value));
    let destination_value =
        transform_path_w(destination).map(|value| winapi::string_to_wide(&value));
    next(
        source_value.as_ref().map_or(source, |value| value.as_ptr()),
        destination_value
            .as_ref()
            .map_or(destination, |value| value.as_ptr()),
        fail_if_exists,
    )
}

unsafe extern "system" fn hooked_move_file_ex_a(
    source: *const c_char,
    destination: *const c_char,
    flags: u32,
) -> i32 {
    let Some(next) = cast_next::<MoveFileExAFn>(NEXT_MOVE_FILE_EX_A) else {
        return 0;
    };
    let source_value = transform_path_a(source).map(|value| winapi::to_cstring_lossy(&value));
    let destination_value =
        transform_path_a(destination).map(|value| winapi::to_cstring_lossy(&value));
    next(
        source_value.as_ref().map_or(source, |value| value.as_ptr()),
        destination_value
            .as_ref()
            .map_or(destination, |value| value.as_ptr()),
        flags,
    )
}

unsafe extern "system" fn hooked_move_file_ex_w(
    source: *const u16,
    destination: *const u16,
    flags: u32,
) -> i32 {
    let Some(next) = cast_next::<MoveFileExWFn>(NEXT_MOVE_FILE_EX_W) else {
        return 0;
    };
    let source_value = transform_path_w(source).map(|value| winapi::string_to_wide(&value));
    let destination_value =
        transform_path_w(destination).map(|value| winapi::string_to_wide(&value));
    next(
        source_value.as_ref().map_or(source, |value| value.as_ptr()),
        destination_value
            .as_ref()
            .map_or(destination, |value| value.as_ptr()),
        flags,
    )
}

unsafe extern "system" fn hooked_copy_file_ex_a(
    source: *const c_char,
    destination: *const c_char,
    progress: *const c_void,
    data: *mut c_void,
    cancel: *mut i32,
    flags: u32,
) -> i32 {
    let Some(next) = cast_next::<CopyFileExAFn>(NEXT_COPY_FILE_EX_A) else {
        return 0;
    };
    let source_value = transform_path_a(source).map(|value| winapi::to_cstring_lossy(&value));
    let destination_value =
        transform_path_a(destination).map(|value| winapi::to_cstring_lossy(&value));
    next(
        source_value.as_ref().map_or(source, |value| value.as_ptr()),
        destination_value
            .as_ref()
            .map_or(destination, |value| value.as_ptr()),
        progress,
        data,
        cancel,
        flags,
    )
}

unsafe extern "system" fn hooked_copy_file_ex_w(
    source: *const u16,
    destination: *const u16,
    progress: *const c_void,
    data: *mut c_void,
    cancel: *mut i32,
    flags: u32,
) -> i32 {
    let Some(next) = cast_next::<CopyFileExWFn>(NEXT_COPY_FILE_EX_W) else {
        return 0;
    };
    let source_value = transform_path_w(source).map(|value| winapi::string_to_wide(&value));
    let destination_value =
        transform_path_w(destination).map(|value| winapi::string_to_wide(&value));
    next(
        source_value.as_ref().map_or(source, |value| value.as_ptr()),
        destination_value
            .as_ref()
            .map_or(destination, |value| value.as_ptr()),
        progress,
        data,
        cancel,
        flags,
    )
}

unsafe extern "system" fn hooked_replace_file_a(
    replaced: *const c_char,
    replacement: *const c_char,
    backup: *const c_char,
    flags: u32,
    exclude: *mut c_void,
    reserved: *mut c_void,
) -> i32 {
    let Some(next) = cast_next::<ReplaceFileAFn>(NEXT_REPLACE_FILE_A) else {
        return 0;
    };
    let replaced_value = transform_path_a(replaced).map(|value| winapi::to_cstring_lossy(&value));
    let replacement_value =
        transform_path_a(replacement).map(|value| winapi::to_cstring_lossy(&value));
    let backup_value = transform_path_a(backup).map(|value| winapi::to_cstring_lossy(&value));
    next(
        replaced_value
            .as_ref()
            .map_or(replaced, |value| value.as_ptr()),
        replacement_value
            .as_ref()
            .map_or(replacement, |value| value.as_ptr()),
        backup_value.as_ref().map_or(backup, |value| value.as_ptr()),
        flags,
        exclude,
        reserved,
    )
}

unsafe extern "system" fn hooked_replace_file_w(
    replaced: *const u16,
    replacement: *const u16,
    backup: *const u16,
    flags: u32,
    exclude: *mut c_void,
    reserved: *mut c_void,
) -> i32 {
    let Some(next) = cast_next::<ReplaceFileWFn>(NEXT_REPLACE_FILE_W) else {
        return 0;
    };
    let replaced_value = transform_path_w(replaced).map(|value| winapi::string_to_wide(&value));
    let replacement_value =
        transform_path_w(replacement).map(|value| winapi::string_to_wide(&value));
    let backup_value = transform_path_w(backup).map(|value| winapi::string_to_wide(&value));
    next(
        replaced_value
            .as_ref()
            .map_or(replaced, |value| value.as_ptr()),
        replacement_value
            .as_ref()
            .map_or(replacement, |value| value.as_ptr()),
        backup_value.as_ref().map_or(backup, |value| value.as_ptr()),
        flags,
        exclude,
        reserved,
    )
}

unsafe fn call_single_path_a(next: *const (), path: *const c_char) -> i32 {
    let Some(next) = cast_next::<SinglePathAFn>(next) else {
        return 0;
    };
    let value = transform_path_a(path).map(|value| winapi::to_cstring_lossy(&value));
    next(value.as_ref().map_or(path, |value| value.as_ptr()))
}

unsafe fn call_single_path_w(next: *const (), path: *const u16) -> i32 {
    let Some(next) = cast_next::<SinglePathWFn>(next) else {
        return 0;
    };
    let value = transform_path_w(path).map(|value| winapi::string_to_wide(&value));
    next(value.as_ref().map_or(path, |value| value.as_ptr()))
}

unsafe fn call_two_paths_a(
    next: *const (),
    source: *const c_char,
    destination: *const c_char,
) -> i32 {
    let Some(next) = cast_next::<TwoPathAFn>(next) else {
        return 0;
    };
    let source_value = transform_path_a(source).map(|value| winapi::to_cstring_lossy(&value));
    let destination_value =
        transform_path_a(destination).map(|value| winapi::to_cstring_lossy(&value));
    next(
        source_value.as_ref().map_or(source, |value| value.as_ptr()),
        destination_value
            .as_ref()
            .map_or(destination, |value| value.as_ptr()),
    )
}

unsafe fn call_two_paths_w(next: *const (), source: *const u16, destination: *const u16) -> i32 {
    let Some(next) = cast_next::<TwoPathWFn>(next) else {
        return 0;
    };
    let source_value = transform_path_w(source).map(|value| winapi::string_to_wide(&value));
    let destination_value =
        transform_path_w(destination).map(|value| winapi::string_to_wide(&value));
    next(
        source_value.as_ref().map_or(source, |value| value.as_ptr()),
        destination_value
            .as_ref()
            .map_or(destination, |value| value.as_ptr()),
    )
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

unsafe extern "system" fn hooked_get_private_profile_section_w(
    app: *const u16,
    out: *mut u16,
    size: u32,
    path: *const u16,
) -> u32 {
    let Some(next) = cast_next::<GetPrivateProfileSectionWFn>(NEXT_GET_PRIVATE_PROFILE_SECTION_W)
    else {
        return 0;
    };
    if let Some(transformed) = transform_path_w(path) {
        let path = winapi::string_to_wide(&transformed);
        return next(app, out, size, path.as_ptr());
    }
    next(app, out, size, path)
}

unsafe fn cast_next<T>(next: *const ()) -> Option<T> {
    (!next.is_null()).then(|| std::mem::transmute_copy(&next))
}
