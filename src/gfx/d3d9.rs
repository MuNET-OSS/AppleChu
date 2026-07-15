use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use once_cell::sync::OnceCell;

use crate::config::Config;
use crate::gfx::WindowConfig;
use crate::util::api::Api;
use crate::util::iat_hook::hook_iat;

crate::config_section! {
    pub(crate) struct D3D9ExConfig => D3D9EX_CONFIG_SECTION {
        section: "D3D9Ex",
        order: 110,
        default_enabled: true,
        always_enabled: false,
        hidden: false,
        comment: "D3D9Ex 透明升级、设备丢失恢复与快速重启",
        fields: {
            pub device_lost_recover: bool = true,
            comment: "设备丢失后自动恢复";
            pub fast_restart: bool = true,
            comment: "启用快速重启补丁";
        }
    }
}

const D3DERR_DEVICELOST: i32 = 0x8876_0868u32 as i32;
const D3DERR_DEVICENOTRESET: i32 = 0x8876_0869u32 as i32;
const D3DERR_DEVICEHUNG: i32 = 0x8876_086cu32 as i32;

const D3D_SDK_VERSION: u32 = 32;
const D3DCREATE_FPU_PRESERVE: u32 = 0x0000_0002;
const D3DCREATE_MULTITHREADED: u32 = 0x0000_0004;

const DEVICE_PRESENT_INDEX: usize = 17;
const DEVICE_PRESENT_EX_INDEX: usize = 121;

type Direct3DCreate9Fn = unsafe extern "system" fn(u32) -> *mut c_void;
type Direct3DCreate9ExFn = unsafe extern "system" fn(u32, *mut *mut c_void) -> i32;

macro_rules! fptr {
    ($func:expr) => {
        $func as *const () as usize
    };
}

static ORIG_DIRECT3D_CREATE9: AtomicUsize = AtomicUsize::new(0);
static DIRECT3D_CREATE9_EX: AtomicUsize = AtomicUsize::new(0);
static MONITOR_INDEX: AtomicU32 = AtomicU32::new(0);
static WINDOWED: AtomicBool = AtomicBool::new(false);
static D3D9EX_ENABLED: AtomicBool = AtomicBool::new(false);
static DEVICE_LOST_RECOVER: AtomicBool = AtomicBool::new(true);
static D3D9_VTABLE: OnceCell<Box<[usize; 17]>> = OnceCell::new();
static DEVICE_VTABLE: OnceCell<Box<[usize; 119]>> = OnceCell::new();

#[repr(C)]
#[derive(Clone, Copy)]
struct D3dPresentParameters {
    back_buffer_width: u32,
    back_buffer_height: u32,
    back_buffer_format: u32,
    back_buffer_count: u32,
    multi_sample_type: u32,
    multi_sample_quality: u32,
    swap_effect: u32,
    device_window: usize,
    windowed: i32,
    enable_auto_depth_stencil: i32,
    auto_depth_stencil_format: u32,
    flags: u32,
    full_screen_refresh_rate_in_hz: u32,
    presentation_interval: u32,
}

#[repr(C)]
struct D3d9ExProxy {
    vtable: *const usize,
    real: *mut c_void,
}

#[repr(C)]
struct D3d9DeviceExProxy {
    vtable: *const usize,
    real: *mut c_void,
    present_parameters: D3dPresentParameters,
}

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryA(name: *const c_char) -> usize;
    fn GetProcAddress(module: usize, name: *const c_char) -> *const c_void;
    fn Sleep(milliseconds: u32);
}

pub fn init(api: &Api, config: &Config) {
    let window = config.section::<WindowConfig>();
    let d3d9ex_config = config.section::<D3D9ExConfig>();
    let windowed = window
        .as_ref()
        .is_some_and(|config| config.enabled && config.windowed);
    let monitor = window
        .as_ref()
        .filter(|config| config.enabled)
        .and_then(|config| u32::try_from(config.monitor.max(0)).ok())
        .unwrap_or(0);
    let d3d9ex = d3d9ex_config.as_ref().is_some_and(|config| config.enabled);

    if !window.as_ref().is_some_and(|config| config.enabled) && !d3d9ex {
        return;
    }
    if !windowed && monitor == 0 && !d3d9ex {
        return;
    }

    MONITOR_INDEX.store(monitor, Ordering::SeqCst);
    WINDOWED.store(windowed, Ordering::SeqCst);
    D3D9EX_ENABLED.store(d3d9ex, Ordering::SeqCst);
    DEVICE_LOST_RECOVER.store(
        d3d9ex_config
            .as_ref()
            .is_some_and(|config| config.enabled && config.device_lost_recover),
        Ordering::SeqCst,
    );

    if d3d9ex {
        resolve_direct3d_create9_ex(api);
    }

    let original = unsafe {
        hook_iat(
            api.game_base(),
            "d3d9.dll",
            "Direct3DCreate9",
            hooked_direct3d_create9 as *const (),
        )
    };

    if let Some(original) = original {
        ORIG_DIRECT3D_CREATE9.store(fptr!(original), Ordering::SeqCst);
        api.log_info(&format!(
            "gfx: D3D9 hook installed (d3d9ex={d3d9ex}, windowed={windowed}, monitor={monitor})"
        ));
    } else {
        api.log_warn("gfx: Direct3DCreate9 import not found");
    }
}

fn resolve_direct3d_create9_ex(api: &Api) {
    let module = unsafe { LoadLibraryA(c"d3d9.dll".as_ptr()) };
    if module == 0 {
        api.log_warn("gfx: failed to load d3d9.dll for Direct3DCreate9Ex");
        return;
    }

    let proc = unsafe { GetProcAddress(module, c"Direct3DCreate9Ex".as_ptr()) };
    if proc.is_null() {
        api.log_warn("gfx: Direct3DCreate9Ex unavailable; falling back to D3D9");
        return;
    }

    DIRECT3D_CREATE9_EX.store(fptr!(proc), Ordering::SeqCst);
}

unsafe extern "system" fn hooked_direct3d_create9(sdk_version: u32) -> *mut c_void {
    if D3D9EX_ENABLED.load(Ordering::SeqCst) {
        if let Some(proxy) = create_d3d9ex_proxy(sdk_version) {
            return proxy.cast();
        }
    }

    let original_addr = ORIG_DIRECT3D_CREATE9.load(Ordering::SeqCst);
    if original_addr == 0 {
        return ptr::null_mut();
    }

    let original: Direct3DCreate9Fn = std::mem::transmute(original_addr);
    original(sdk_version)
}

unsafe fn create_d3d9ex_proxy(sdk_version: u32) -> Option<*mut D3d9ExProxy> {
    let create_ex_addr = DIRECT3D_CREATE9_EX.load(Ordering::SeqCst);
    if create_ex_addr == 0 {
        return None;
    }

    let create_ex: Direct3DCreate9ExFn = std::mem::transmute(create_ex_addr);
    let mut real = ptr::null_mut();
    if create_ex(sdk_version.max(D3D_SDK_VERSION), &mut real) < 0 || real.is_null() {
        return None;
    }

    Some(Box::into_raw(Box::new(D3d9ExProxy {
        vtable: d3d9_vtable(),
        real,
    })))
}

fn d3d9_vtable() -> *const usize {
    D3D9_VTABLE
        .get_or_init(|| {
            Box::new([
                fptr!(d3d9_query_interface),
                fptr!(d3d9_add_ref),
                fptr!(d3d9_release),
                fptr!(d3d9_register_software_device),
                fptr!(d3d9_get_adapter_count),
                fptr!(d3d9_get_adapter_identifier),
                fptr!(d3d9_get_adapter_mode_count),
                fptr!(d3d9_enum_adapter_modes),
                fptr!(d3d9_get_adapter_display_mode),
                fptr!(d3d9_check_device_type),
                fptr!(d3d9_check_device_format),
                fptr!(d3d9_check_device_multi_sample_type),
                fptr!(d3d9_check_depth_stencil_match),
                fptr!(d3d9_check_device_format_conversion),
                fptr!(d3d9_get_device_caps),
                fptr!(d3d9_get_adapter_monitor),
                fptr!(d3d9_create_device),
            ])
        })
        .as_ptr()
}

fn device_vtable() -> *const usize {
    DEVICE_VTABLE
        .get_or_init(|| {
            Box::new([
                fptr!(device_query_interface),
                fptr!(device_add_ref),
                fptr!(device_release),
                fptr!(device_test_cooperative_level),
                fptr!(device_get_available_texture_mem),
                fptr!(device_evict_managed_resources),
                fptr!(device_get_direct3d),
                fptr!(device_get_device_caps),
                fptr!(device_get_display_mode),
                fptr!(device_get_creation_parameters),
                fptr!(device_set_cursor_properties),
                fptr!(device_set_cursor_position),
                fptr!(device_show_cursor),
                fptr!(device_create_additional_swap_chain),
                fptr!(device_get_swap_chain),
                fptr!(device_get_number_of_swap_chains),
                fptr!(device_reset),
                fptr!(device_present),
                fptr!(device_get_back_buffer),
                fptr!(device_get_raster_status),
                fptr!(device_set_dialog_box_mode),
                fptr!(device_set_gamma_ramp),
                fptr!(device_get_gamma_ramp),
                fptr!(device_create_texture),
                fptr!(device_create_volume_texture),
                fptr!(device_create_cube_texture),
                fptr!(device_create_vertex_buffer),
                fptr!(device_create_index_buffer),
                fptr!(device_create_render_target),
                fptr!(device_create_depth_stencil_surface),
                fptr!(device_update_surface),
                fptr!(device_update_texture),
                fptr!(device_get_render_target_data),
                fptr!(device_get_front_buffer_data),
                fptr!(device_stretch_rect),
                fptr!(device_color_fill),
                fptr!(device_create_offscreen_plain_surface),
                fptr!(device_set_render_target),
                fptr!(device_get_render_target),
                fptr!(device_set_depth_stencil_surface),
                fptr!(device_get_depth_stencil_surface),
                fptr!(device_begin_scene),
                fptr!(device_end_scene),
                fptr!(device_clear),
                fptr!(device_set_transform),
                fptr!(device_get_transform),
                fptr!(device_multiply_transform),
                fptr!(device_set_viewport),
                fptr!(device_get_viewport),
                fptr!(device_set_material),
                fptr!(device_get_material),
                fptr!(device_set_light),
                fptr!(device_get_light),
                fptr!(device_light_enable),
                fptr!(device_get_light_enable),
                fptr!(device_set_clip_plane),
                fptr!(device_get_clip_plane),
                fptr!(device_set_render_state),
                fptr!(device_get_render_state),
                fptr!(device_create_state_block),
                fptr!(device_begin_state_block),
                fptr!(device_end_state_block),
                fptr!(device_set_clip_status),
                fptr!(device_get_clip_status),
                fptr!(device_get_texture),
                fptr!(device_set_texture),
                fptr!(device_get_texture_stage_state),
                fptr!(device_set_texture_stage_state),
                fptr!(device_get_sampler_state),
                fptr!(device_set_sampler_state),
                fptr!(device_validate_device),
                fptr!(device_set_palette_entries),
                fptr!(device_get_palette_entries),
                fptr!(device_set_current_texture_palette),
                fptr!(device_get_current_texture_palette),
                fptr!(device_set_scissor_rect),
                fptr!(device_get_scissor_rect),
                fptr!(device_set_software_vertex_processing),
                fptr!(device_get_software_vertex_processing),
                fptr!(device_set_npatch_mode),
                fptr!(device_get_npatch_mode),
                fptr!(device_draw_primitive),
                fptr!(device_draw_indexed_primitive),
                fptr!(device_draw_primitive_up),
                fptr!(device_draw_indexed_primitive_up),
                fptr!(device_process_vertices),
                fptr!(device_create_vertex_declaration),
                fptr!(device_set_vertex_declaration),
                fptr!(device_get_vertex_declaration),
                fptr!(device_set_fvf),
                fptr!(device_get_fvf),
                fptr!(device_create_vertex_shader),
                fptr!(device_set_vertex_shader),
                fptr!(device_get_vertex_shader),
                fptr!(device_set_vertex_shader_constant_f),
                fptr!(device_get_vertex_shader_constant_f),
                fptr!(device_set_vertex_shader_constant_i),
                fptr!(device_get_vertex_shader_constant_i),
                fptr!(device_set_vertex_shader_constant_b),
                fptr!(device_get_vertex_shader_constant_b),
                fptr!(device_set_stream_source),
                fptr!(device_get_stream_source),
                fptr!(device_set_stream_source_freq),
                fptr!(device_get_stream_source_freq),
                fptr!(device_set_indices),
                fptr!(device_get_indices),
                fptr!(device_create_pixel_shader),
                fptr!(device_set_pixel_shader),
                fptr!(device_get_pixel_shader),
                fptr!(device_set_pixel_shader_constant_f),
                fptr!(device_get_pixel_shader_constant_f),
                fptr!(device_set_pixel_shader_constant_i),
                fptr!(device_get_pixel_shader_constant_i),
                fptr!(device_set_pixel_shader_constant_b),
                fptr!(device_get_pixel_shader_constant_b),
                fptr!(device_draw_rect_patch),
                fptr!(device_draw_tri_patch),
                fptr!(device_delete_patch),
                fptr!(device_create_query),
            ])
        })
        .as_ptr()
}

unsafe fn vtable_entry(instance: *mut c_void, index: usize) -> usize {
    let vtable = *(instance as *const *const usize);
    *vtable.add(index)
}

macro_rules! d3d9_forward_i32 {
    ($name:ident, $idx:expr) => {
        unsafe extern "system" fn $name(this: *mut D3d9ExProxy) -> i32 {
            let real = (*this).real;
            let func: unsafe extern "system" fn(*mut c_void) -> i32 = std::mem::transmute(vtable_entry(real, $idx));
            func(real)
        }
    };
    ($name:ident, $idx:expr $(, $arg:ident: $ty:ty)*) => {
        unsafe extern "system" fn $name(this: *mut D3d9ExProxy, $($arg: $ty),*) -> i32 {
            let real = (*this).real;
            let func: unsafe extern "system" fn(*mut c_void, $($ty),*) -> i32 = std::mem::transmute(vtable_entry(real, $idx));
            func(real, $($arg),*)
        }
    };
}

macro_rules! d3d9_forward_u32 {
    ($name:ident, $idx:expr) => {
        unsafe extern "system" fn $name(this: *mut D3d9ExProxy) -> u32 {
            let real = (*this).real;
            let func: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(vtable_entry(real, $idx));
            func(real)
        }
    };
    ($name:ident, $idx:expr $(, $arg:ident: $ty:ty)*) => {
        unsafe extern "system" fn $name(this: *mut D3d9ExProxy, $($arg: $ty),*) -> u32 {
            let real = (*this).real;
            let func: unsafe extern "system" fn(*mut c_void, $($ty),*) -> u32 = std::mem::transmute(vtable_entry(real, $idx));
            func(real, $($arg),*)
        }
    };
}

macro_rules! device_forward_i32 {
    ($name:ident, $idx:expr) => {
        unsafe extern "system" fn $name(this: *mut D3d9DeviceExProxy) -> i32 {
            let real = (*this).real;
            let func: unsafe extern "system" fn(*mut c_void) -> i32 = std::mem::transmute(vtable_entry(real, $idx));
            func(real)
        }
    };
    ($name:ident, $idx:expr $(, $arg:ident: $ty:ty)*) => {
        unsafe extern "system" fn $name(this: *mut D3d9DeviceExProxy, $($arg: $ty),*) -> i32 {
            let real = (*this).real;
            let func: unsafe extern "system" fn(*mut c_void, $($ty),*) -> i32 = std::mem::transmute(vtable_entry(real, $idx));
            func(real, $($arg),*)
        }
    };
}

macro_rules! device_forward_u32 {
    ($name:ident, $idx:expr) => {
        unsafe extern "system" fn $name(this: *mut D3d9DeviceExProxy) -> u32 {
            let real = (*this).real;
            let func: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(vtable_entry(real, $idx));
            func(real)
        }
    };
    ($name:ident, $idx:expr $(, $arg:ident: $ty:ty)*) => {
        unsafe extern "system" fn $name(this: *mut D3d9DeviceExProxy, $($arg: $ty),*) -> u32 {
            let real = (*this).real;
            let func: unsafe extern "system" fn(*mut c_void, $($ty),*) -> u32 = std::mem::transmute(vtable_entry(real, $idx));
            func(real, $($arg),*)
        }
    };
}

macro_rules! device_forward_void {
    ($name:ident, $idx:expr) => {
        unsafe extern "system" fn $name(this: *mut D3d9DeviceExProxy) {
            let real = (*this).real;
            let func: unsafe extern "system" fn(*mut c_void) = std::mem::transmute(vtable_entry(real, $idx));
            func(real)
        }
    };
    ($name:ident, $idx:expr $(, $arg:ident: $ty:ty)*) => {
        unsafe extern "system" fn $name(this: *mut D3d9DeviceExProxy, $($arg: $ty),*) {
            let real = (*this).real;
            let func: unsafe extern "system" fn(*mut c_void, $($ty),*) = std::mem::transmute(vtable_entry(real, $idx));
            func(real, $($arg),*)
        }
    };
}

unsafe extern "system" fn d3d9_query_interface(
    this: *mut D3d9ExProxy,
    riid: *const c_void,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() {
        return 0x8000_4003u32 as i32;
    }
    *out = ptr::null_mut();
    let real = (*this).real;
    let func: unsafe extern "system" fn(*mut c_void, *const c_void, *mut *mut c_void) -> i32 =
        std::mem::transmute(vtable_entry(real, 0));
    let result = func(real, riid, out);
    if result >= 0 {
        *out = this.cast();
    }
    result
}

d3d9_forward_u32!(d3d9_add_ref, 1);

unsafe extern "system" fn d3d9_release(this: *mut D3d9ExProxy) -> u32 {
    let real = (*this).real;
    let func: unsafe extern "system" fn(*mut c_void) -> u32 =
        std::mem::transmute(vtable_entry(real, 2));
    let count = func(real);
    if count == 0 {
        drop(Box::from_raw(this));
    }
    count
}

d3d9_forward_i32!(d3d9_register_software_device, 3, init_function: *mut c_void);
d3d9_forward_u32!(d3d9_get_adapter_count, 4);
d3d9_forward_i32!(d3d9_get_adapter_identifier, 5, adapter: u32, flags: u32, identifier: *mut c_void);
d3d9_forward_u32!(d3d9_get_adapter_mode_count, 6, adapter: u32, format: u32);
d3d9_forward_i32!(d3d9_enum_adapter_modes, 7, adapter: u32, format: u32, mode: u32, display_mode: *mut c_void);
d3d9_forward_i32!(d3d9_get_adapter_display_mode, 8, adapter: u32, display_mode: *mut c_void);
d3d9_forward_i32!(d3d9_check_device_type, 9, adapter: u32, device_type: u32, adapter_format: u32, back_buffer_format: u32, windowed: i32);
d3d9_forward_i32!(d3d9_check_device_format, 10, adapter: u32, device_type: u32, adapter_format: u32, usage: u32, resource_type: u32, check_format: u32);
d3d9_forward_i32!(d3d9_check_device_multi_sample_type, 11, adapter: u32, device_type: u32, surface_format: u32, windowed: i32, multi_sample_type: u32, quality_levels: *mut u32);
d3d9_forward_i32!(d3d9_check_depth_stencil_match, 12, adapter: u32, device_type: u32, adapter_format: u32, render_target_format: u32, depth_stencil_format: u32);
d3d9_forward_i32!(d3d9_check_device_format_conversion, 13, adapter: u32, device_type: u32, source_format: u32, target_format: u32);
d3d9_forward_i32!(d3d9_get_device_caps, 14, adapter: u32, device_type: u32, caps: *mut c_void);

unsafe extern "system" fn d3d9_get_adapter_monitor(this: *mut D3d9ExProxy, adapter: u32) -> usize {
    let real = (*this).real;
    let func: unsafe extern "system" fn(*mut c_void, u32) -> usize =
        std::mem::transmute(vtable_entry(real, 15));
    func(real, adapter)
}

unsafe extern "system" fn d3d9_create_device(
    this: *mut D3d9ExProxy,
    adapter: u32,
    device_type: u32,
    focus_window: usize,
    behavior_flags: u32,
    presentation_parameters: *mut D3dPresentParameters,
    returned_device_interface: *mut *mut c_void,
) -> i32 {
    if returned_device_interface.is_null() {
        return 0x8000_4003u32 as i32;
    }
    *returned_device_interface = ptr::null_mut();

    let selected_adapter = MONITOR_INDEX.load(Ordering::SeqCst);
    if WINDOWED.load(Ordering::SeqCst) && !presentation_parameters.is_null() {
        (*presentation_parameters).windowed = 1;
        (*presentation_parameters).full_screen_refresh_rate_in_hz = 0;
    }

    let real = (*this).real;
    let func: unsafe extern "system" fn(
        *mut c_void,
        u32,
        u32,
        usize,
        u32,
        *mut D3dPresentParameters,
        *mut *mut c_void,
    ) -> i32 = std::mem::transmute(vtable_entry(real, 16));

    let mut real_device = ptr::null_mut();
    let result = func(
        real,
        if selected_adapter == 0 {
            adapter
        } else {
            selected_adapter
        },
        device_type,
        focus_window,
        behavior_flags | D3DCREATE_FPU_PRESERVE | D3DCREATE_MULTITHREADED,
        presentation_parameters,
        &mut real_device,
    );
    if result >= 0 && !real_device.is_null() {
        *returned_device_interface = Box::into_raw(Box::new(D3d9DeviceExProxy {
            vtable: device_vtable(),
            real: real_device,
            present_parameters: if presentation_parameters.is_null() {
                std::mem::zeroed()
            } else {
                *presentation_parameters
            },
        }))
        .cast();
    }
    result
}

unsafe extern "system" fn device_query_interface(
    this: *mut D3d9DeviceExProxy,
    riid: *const c_void,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() {
        return 0x8000_4003u32 as i32;
    }
    *out = ptr::null_mut();
    let real = (*this).real;
    let func: unsafe extern "system" fn(*mut c_void, *const c_void, *mut *mut c_void) -> i32 =
        std::mem::transmute(vtable_entry(real, 0));
    let result = func(real, riid, out);
    if result >= 0 {
        *out = this.cast();
    }
    result
}

device_forward_u32!(device_add_ref, 1);

unsafe extern "system" fn device_release(this: *mut D3d9DeviceExProxy) -> u32 {
    let real = (*this).real;
    let func: unsafe extern "system" fn(*mut c_void) -> u32 =
        std::mem::transmute(vtable_entry(real, 2));
    let count = func(real);
    if count == 0 {
        drop(Box::from_raw(this));
    }
    count
}

device_forward_i32!(device_test_cooperative_level, 3);
device_forward_u32!(device_get_available_texture_mem, 4);
device_forward_i32!(device_evict_managed_resources, 5);
device_forward_i32!(device_get_direct3d, 6, direct3d: *mut *mut c_void);
device_forward_i32!(device_get_device_caps, 7, caps: *mut c_void);
device_forward_i32!(device_get_display_mode, 8, swap_chain: u32, mode: *mut c_void);
device_forward_i32!(device_get_creation_parameters, 9, parameters: *mut c_void);
device_forward_i32!(device_set_cursor_properties, 10, x_hotspot: u32, y_hotspot: u32, cursor_bitmap: *mut c_void);
device_forward_void!(device_set_cursor_position, 11, x: i32, y: i32, flags: u32);
device_forward_i32!(device_show_cursor, 12, show: i32);
device_forward_i32!(device_create_additional_swap_chain, 13, params: *mut D3dPresentParameters, swap_chain: *mut *mut c_void);
device_forward_i32!(device_get_swap_chain, 14, index: u32, swap_chain: *mut *mut c_void);
device_forward_u32!(device_get_number_of_swap_chains, 15);

unsafe extern "system" fn device_reset(
    this: *mut D3d9DeviceExProxy,
    params: *mut D3dPresentParameters,
) -> i32 {
    if !params.is_null() {
        (*this).present_parameters = *params;
    }
    let real = (*this).real;
    let func: unsafe extern "system" fn(*mut c_void, *mut D3dPresentParameters) -> i32 =
        std::mem::transmute(vtable_entry(real, 16));
    func(real, params)
}

unsafe extern "system" fn device_present(
    this: *mut D3d9DeviceExProxy,
    source_rect: *const c_void,
    dest_rect: *const c_void,
    dest_window_override: usize,
    dirty_region: *const c_void,
) -> i32 {
    let real = (*this).real;
    let present_index = if D3D9EX_ENABLED.load(Ordering::SeqCst) {
        DEVICE_PRESENT_EX_INDEX
    } else {
        DEVICE_PRESENT_INDEX
    };
    let func: unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        *const c_void,
        usize,
        *const c_void,
        u32,
    ) -> i32 = std::mem::transmute(vtable_entry(real, present_index));

    let result = func(
        real,
        source_rect,
        dest_rect,
        dest_window_override,
        dirty_region,
        0,
    );
    if result == D3DERR_DEVICELOST && DEVICE_LOST_RECOVER.load(Ordering::SeqCst) {
        recover_lost_device(this);
    }
    result
}

unsafe fn recover_lost_device(this: *mut D3d9DeviceExProxy) {
    let real = (*this).real;
    let test: unsafe extern "system" fn(*mut c_void) -> i32 =
        std::mem::transmute(vtable_entry(real, 3));
    let reset: unsafe extern "system" fn(*mut c_void, *mut D3dPresentParameters) -> i32 =
        std::mem::transmute(vtable_entry(real, 16));

    let mut status = test(real);
    while status == D3DERR_DEVICELOST {
        Sleep(10);
        status = test(real);
    }

    if status == D3DERR_DEVICENOTRESET {
        let reset_result = reset(real, &mut (*this).present_parameters);
        if reset_result != D3DERR_DEVICEHUNG {
            // Recovery succeeded or failed with a non-hung result; the next Present decides final state.
        }
    }
}

device_forward_i32!(device_get_back_buffer, 18, swap_chain: u32, back_buffer: u32, ty: u32, surface: *mut *mut c_void);
device_forward_i32!(device_get_raster_status, 19, swap_chain: u32, status: *mut c_void);
device_forward_i32!(device_set_dialog_box_mode, 20, enable: i32);
device_forward_void!(device_set_gamma_ramp, 21, swap_chain: u32, flags: u32, ramp: *const c_void);
device_forward_void!(device_get_gamma_ramp, 22, swap_chain: u32, ramp: *mut c_void);
device_forward_i32!(device_create_texture, 23, width: u32, height: u32, levels: u32, usage: u32, format: u32, pool: u32, texture: *mut *mut c_void, shared_handle: *mut usize);
device_forward_i32!(device_create_volume_texture, 24, width: u32, height: u32, depth: u32, levels: u32, usage: u32, format: u32, pool: u32, texture: *mut *mut c_void, shared_handle: *mut usize);
device_forward_i32!(device_create_cube_texture, 25, edge_length: u32, levels: u32, usage: u32, format: u32, pool: u32, texture: *mut *mut c_void, shared_handle: *mut usize);
device_forward_i32!(device_create_vertex_buffer, 26, length: u32, usage: u32, fvf: u32, pool: u32, buffer: *mut *mut c_void, shared_handle: *mut usize);
device_forward_i32!(device_create_index_buffer, 27, length: u32, usage: u32, format: u32, pool: u32, buffer: *mut *mut c_void, shared_handle: *mut usize);
device_forward_i32!(device_create_render_target, 28, width: u32, height: u32, format: u32, multi_sample: u32, quality: u32, lockable: i32, surface: *mut *mut c_void, shared_handle: *mut usize);
device_forward_i32!(device_create_depth_stencil_surface, 29, width: u32, height: u32, format: u32, multi_sample: u32, quality: u32, discard: i32, surface: *mut *mut c_void, shared_handle: *mut usize);
device_forward_i32!(device_update_surface, 30, source: *mut c_void, source_rect: *const c_void, destination: *mut c_void, dest_point: *const c_void);
device_forward_i32!(device_update_texture, 31, source: *mut c_void, destination: *mut c_void);
device_forward_i32!(device_get_render_target_data, 32, render_target: *mut c_void, dest_surface: *mut c_void);
device_forward_i32!(device_get_front_buffer_data, 33, swap_chain: u32, dest_surface: *mut c_void);
device_forward_i32!(device_stretch_rect, 34, source: *mut c_void, source_rect: *const c_void, dest: *mut c_void, dest_rect: *const c_void, filter: u32);
device_forward_i32!(device_color_fill, 35, surface: *mut c_void, rect: *const c_void, color: u32);
device_forward_i32!(device_create_offscreen_plain_surface, 36, width: u32, height: u32, format: u32, pool: u32, surface: *mut *mut c_void, shared_handle: *mut usize);
device_forward_i32!(device_set_render_target, 37, index: u32, surface: *mut c_void);
device_forward_i32!(device_get_render_target, 38, index: u32, surface: *mut *mut c_void);
device_forward_i32!(device_set_depth_stencil_surface, 39, surface: *mut c_void);
device_forward_i32!(device_get_depth_stencil_surface, 40, surface: *mut *mut c_void);
device_forward_i32!(device_begin_scene, 41);
device_forward_i32!(device_end_scene, 42);
device_forward_i32!(device_clear, 43, count: u32, rects: *const c_void, flags: u32, color: u32, z: f32, stencil: u32);
device_forward_i32!(device_set_transform, 44, state: u32, matrix: *const c_void);
device_forward_i32!(device_get_transform, 45, state: u32, matrix: *mut c_void);
device_forward_i32!(device_multiply_transform, 46, state: u32, matrix: *const c_void);
device_forward_i32!(device_set_viewport, 47, viewport: *const c_void);
device_forward_i32!(device_get_viewport, 48, viewport: *mut c_void);
device_forward_i32!(device_set_material, 49, material: *const c_void);
device_forward_i32!(device_get_material, 50, material: *mut c_void);
device_forward_i32!(device_set_light, 51, index: u32, light: *const c_void);
device_forward_i32!(device_get_light, 52, index: u32, light: *mut c_void);
device_forward_i32!(device_light_enable, 53, index: u32, enable: i32);
device_forward_i32!(device_get_light_enable, 54, index: u32, enable: *mut i32);
device_forward_i32!(device_set_clip_plane, 55, index: u32, plane: *const f32);
device_forward_i32!(device_get_clip_plane, 56, index: u32, plane: *mut f32);
device_forward_i32!(device_set_render_state, 57, state: u32, value: u32);
device_forward_i32!(device_get_render_state, 58, state: u32, value: *mut u32);
device_forward_i32!(device_create_state_block, 59, ty: u32, state_block: *mut *mut c_void);
device_forward_i32!(device_begin_state_block, 60);
device_forward_i32!(device_end_state_block, 61, state_block: *mut *mut c_void);
device_forward_i32!(device_set_clip_status, 62, clip_status: *const c_void);
device_forward_i32!(device_get_clip_status, 63, clip_status: *mut c_void);
device_forward_i32!(device_get_texture, 64, stage: u32, texture: *mut *mut c_void);
device_forward_i32!(device_set_texture, 65, stage: u32, texture: *mut c_void);
device_forward_i32!(device_get_texture_stage_state, 66, stage: u32, ty: u32, value: *mut u32);
device_forward_i32!(device_set_texture_stage_state, 67, stage: u32, ty: u32, value: u32);
device_forward_i32!(device_get_sampler_state, 68, sampler: u32, ty: u32, value: *mut u32);
device_forward_i32!(device_set_sampler_state, 69, sampler: u32, ty: u32, value: u32);
device_forward_i32!(device_validate_device, 70, passes: *mut u32);
device_forward_i32!(device_set_palette_entries, 71, palette_number: u32, entries: *const c_void);
device_forward_i32!(device_get_palette_entries, 72, palette_number: u32, entries: *mut c_void);
device_forward_i32!(device_set_current_texture_palette, 73, palette_number: u32);
device_forward_i32!(device_get_current_texture_palette, 74, palette_number: *mut u32);
device_forward_i32!(device_set_scissor_rect, 75, rect: *const c_void);
device_forward_i32!(device_get_scissor_rect, 76, rect: *mut c_void);
device_forward_i32!(device_set_software_vertex_processing, 77, software: i32);

unsafe extern "system" fn device_get_software_vertex_processing(
    this: *mut D3d9DeviceExProxy,
) -> i32 {
    let real = (*this).real;
    let func: unsafe extern "system" fn(*mut c_void) -> i32 =
        std::mem::transmute(vtable_entry(real, 78));
    func(real)
}

device_forward_i32!(device_set_npatch_mode, 79, segments: f32);

unsafe extern "system" fn device_get_npatch_mode(this: *mut D3d9DeviceExProxy) -> f32 {
    let real = (*this).real;
    let func: unsafe extern "system" fn(*mut c_void) -> f32 =
        std::mem::transmute(vtable_entry(real, 80));
    func(real)
}

device_forward_i32!(device_draw_primitive, 81, primitive_type: u32, start_vertex: u32, primitive_count: u32);
device_forward_i32!(device_draw_indexed_primitive, 82, primitive_type: u32, base_vertex_index: i32, min_vertex_index: u32, num_vertices: u32, start_index: u32, primitive_count: u32);
device_forward_i32!(device_draw_primitive_up, 83, primitive_type: u32, primitive_count: u32, vertex_stream_zero_data: *const c_void, vertex_stream_zero_stride: u32);
device_forward_i32!(device_draw_indexed_primitive_up, 84, primitive_type: u32, min_vertex_index: u32, num_vertices: u32, primitive_count: u32, index_data: *const c_void, index_data_format: u32, vertex_stream_zero_data: *const c_void, vertex_stream_zero_stride: u32);
device_forward_i32!(device_process_vertices, 85, source_start_index: u32, dest_index: u32, vertex_count: u32, dest_buffer: *mut c_void, vertex_decl: *mut c_void, flags: u32);
device_forward_i32!(device_create_vertex_declaration, 86, elements: *const c_void, declaration: *mut *mut c_void);
device_forward_i32!(device_set_vertex_declaration, 87, declaration: *mut c_void);
device_forward_i32!(device_get_vertex_declaration, 88, declaration: *mut *mut c_void);
device_forward_i32!(device_set_fvf, 89, fvf: u32);
device_forward_i32!(device_get_fvf, 90, fvf: *mut u32);
device_forward_i32!(device_create_vertex_shader, 91, function: *const u32, shader: *mut *mut c_void);
device_forward_i32!(device_set_vertex_shader, 92, shader: *mut c_void);
device_forward_i32!(device_get_vertex_shader, 93, shader: *mut *mut c_void);
device_forward_i32!(device_set_vertex_shader_constant_f, 94, start_register: u32, constant_data: *const f32, vector4f_count: u32);
device_forward_i32!(device_get_vertex_shader_constant_f, 95, start_register: u32, constant_data: *mut f32, vector4f_count: u32);
device_forward_i32!(device_set_vertex_shader_constant_i, 96, start_register: u32, constant_data: *const i32, vector4i_count: u32);
device_forward_i32!(device_get_vertex_shader_constant_i, 97, start_register: u32, constant_data: *mut i32, vector4i_count: u32);
device_forward_i32!(device_set_vertex_shader_constant_b, 98, start_register: u32, constant_data: *const i32, bool_count: u32);
device_forward_i32!(device_get_vertex_shader_constant_b, 99, start_register: u32, constant_data: *mut i32, bool_count: u32);
device_forward_i32!(device_set_stream_source, 100, stream_number: u32, stream_data: *mut c_void, offset_in_bytes: u32, stride: u32);
device_forward_i32!(device_get_stream_source, 101, stream_number: u32, stream_data: *mut *mut c_void, offset_in_bytes: *mut u32, stride: *mut u32);
device_forward_i32!(device_set_stream_source_freq, 102, stream_number: u32, setting: u32);
device_forward_i32!(device_get_stream_source_freq, 103, stream_number: u32, setting: *mut u32);
device_forward_i32!(device_set_indices, 104, index_data: *mut c_void);
device_forward_i32!(device_get_indices, 105, index_data: *mut *mut c_void);
device_forward_i32!(device_create_pixel_shader, 106, function: *const u32, shader: *mut *mut c_void);
device_forward_i32!(device_set_pixel_shader, 107, shader: *mut c_void);
device_forward_i32!(device_get_pixel_shader, 108, shader: *mut *mut c_void);
device_forward_i32!(device_set_pixel_shader_constant_f, 109, start_register: u32, constant_data: *const f32, vector4f_count: u32);
device_forward_i32!(device_get_pixel_shader_constant_f, 110, start_register: u32, constant_data: *mut f32, vector4f_count: u32);
device_forward_i32!(device_set_pixel_shader_constant_i, 111, start_register: u32, constant_data: *const i32, vector4i_count: u32);
device_forward_i32!(device_get_pixel_shader_constant_i, 112, start_register: u32, constant_data: *mut i32, vector4i_count: u32);
device_forward_i32!(device_set_pixel_shader_constant_b, 113, start_register: u32, constant_data: *const i32, bool_count: u32);
device_forward_i32!(device_get_pixel_shader_constant_b, 114, start_register: u32, constant_data: *mut i32, bool_count: u32);
device_forward_i32!(device_draw_rect_patch, 115, handle: u32, num_segs: *const f32, rect_patch_info: *const c_void);
device_forward_i32!(device_draw_tri_patch, 116, handle: u32, num_segs: *const f32, tri_patch_info: *const c_void);
device_forward_i32!(device_delete_patch, 117, handle: u32);
device_forward_i32!(device_create_query, 118, ty: u32, query: *mut *mut c_void);
