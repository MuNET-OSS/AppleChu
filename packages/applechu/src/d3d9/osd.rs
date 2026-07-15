use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::autoplay;

const D3DPT_TRIANGLELIST: u32 = 4;
const D3DFVF_XYZRHW: u32 = 0x004;
const D3DFVF_DIFFUSE: u32 = 0x040;
const D3DFVF_TEX1: u32 = 0x100;
const D3DSBT_ALL: u32 = 1;
const D3DFMT_A8R8G8B8: u32 = 21;
const D3DUSAGE_DYNAMIC: u32 = 0x200;
const D3DPOOL_DEFAULT: u32 = 0;
const D3DLOCK_DISCARD: u32 = 0x2000;

const D3DRS_ZENABLE: u32 = 7;
const D3DRS_FILLMODE: u32 = 8;
const D3DRS_SRCBLEND: u32 = 19;
const D3DRS_DESTBLEND: u32 = 20;
const D3DRS_CULLMODE: u32 = 22;
const D3DRS_ALPHABLENDENABLE: u32 = 27;
const D3DRS_FOGENABLE: u32 = 28;
const D3DRS_LIGHTING: u32 = 137;
const D3DRS_SCISSORTESTENABLE: u32 = 174;

const D3DTSS_COLOROP: u32 = 1;
const D3DTSS_COLORARG1: u32 = 2;
const D3DTSS_COLORARG2: u32 = 3;
const D3DTSS_ALPHAOP: u32 = 4;
const D3DTSS_ALPHAARG1: u32 = 5;
const D3DTSS_ALPHAARG2: u32 = 6;
const D3DTOP_MODULATE: u32 = 4;
const D3DTOP_SELECTARG1: u32 = 2;
const D3DTA_DIFFUSE: u32 = 0;
const D3DTA_TEXTURE: u32 = 2;

const D3DCULL_NONE: u32 = 1;
const D3DFILL_SOLID: u32 = 3;
const D3DBLEND_SRCALPHA: u32 = 5;
const D3DBLEND_INVSRCALPHA: u32 = 6;

const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const FW_BOLD: i32 = 700;
const DEFAULT_CHARSET: u32 = 1;
const ANTIALIASED_QUALITY: u32 = 4;
const TRANSPARENT_BK: i32 = 1;

const PANEL_COLOR: u32 = 0xA0423E3E;
const TEXT_COLOR: u32 = 0xFFFFFFFF;
const FPS_WIDTH: f32 = 140.0;
const FPS_HEIGHT: f32 = 30.0;
const FPS_MARGIN: f32 = 12.0;
const AUTOPLAY_WIDTH: f32 = 220.0;
const AUTOPLAY_HEIGHT: f32 = 36.0;
const AUTOPLAY_MARGIN: f32 = 20.0;

static FPS_VISIBLE: AtomicBool = AtomicBool::new(false);
static FPS_X10: AtomicU32 = AtomicU32::new(0);
static TEXT_CACHE: OnceLock<Mutex<Vec<CachedTextTexture>>> = OnceLock::new();

#[repr(C)]
#[derive(Clone, Copy)]
struct Viewport {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    min_z: f32,
    max_z: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ColorVertex {
    x: f32,
    y: f32,
    z: f32,
    rhw: f32,
    color: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TextureVertex {
    x: f32,
    y: f32,
    z: f32,
    rhw: f32,
    color: u32,
    u: f32,
    v: f32,
}

#[repr(C)]
struct D3dLockedRect {
    pitch: i32,
    bits: *mut c_void,
}

#[repr(C)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

#[repr(C)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    colors: [u32; 1],
}

#[repr(C)]
struct Size {
    cx: i32,
    cy: i32,
}

struct TextTexture {
    texture: *mut c_void,
    width: u32,
    height: u32,
}

struct CachedTextTexture {
    device: usize,
    text: String,
    texture: TextTexture,
}

unsafe impl Send for CachedTextTexture {}

struct Label {
    text: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: u32,
}

pub fn set_fps_visible(visible: bool) {
    FPS_VISIBLE.store(visible, Ordering::Relaxed);
}

pub fn set_fps_x10(fps_x10: u32) {
    FPS_X10.store(fps_x10, Ordering::Relaxed);
}

pub unsafe fn render(device: *mut c_void) {
    if device.is_null() {
        return;
    }

    let mut viewport = Viewport {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        min_z: 0.0,
        max_z: 1.0,
    };
    if get_viewport(device, &mut viewport) < 0 || viewport.width == 0 || viewport.height == 0 {
        return;
    }

    let mut labels = Vec::with_capacity(2);

    if FPS_VISIBLE.load(Ordering::Relaxed) {
        let fps_x10 = FPS_X10.load(Ordering::Relaxed);
        labels.push(Label {
            text: format!("{}.{} FPS", fps_x10 / 10, fps_x10 % 10),
            x: FPS_MARGIN,
            y: FPS_MARGIN,
            width: FPS_WIDTH,
            height: FPS_HEIGHT,
            color: TEXT_COLOR,
        });
    }

    let autoplay_text = if autoplay::is_enabled() {
        Some("AutoPlay ON")
    } else if autoplay::was_used() {
        Some("AutoPlay 曾使用")
    } else {
        None
    };
    if let Some(text) = autoplay_text {
        labels.push(Label {
            text: text.to_owned(),
            x: viewport.width as f32 - AUTOPLAY_MARGIN - AUTOPLAY_WIDTH,
            y: AUTOPLAY_MARGIN,
            width: AUTOPLAY_WIDTH,
            height: AUTOPLAY_HEIGHT,
            color: TEXT_COLOR,
        });
    }

    if labels.is_empty() {
        return;
    }

    let mut color_vertices = Vec::with_capacity(labels.len() * 6);
    let mut text_items = Vec::with_capacity(labels.len());
    let cache = TEXT_CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let Ok(mut cache) = cache.lock() else {
        return;
    };
    for label in labels {
        let Some((text_width, text_height)) = measure_text(&label.text) else {
            continue;
        };
        push_rect(
            &mut color_vertices,
            label.x,
            label.y,
            label.width,
            label.height,
            PANEL_COLOR,
        );
        let Some(texture) = cached_text_texture(
            &mut cache,
            device,
            &label.text,
            text_width as u32,
            text_height as u32,
        ) else {
            continue;
        };
        text_items.push((
            texture,
            label.x + ((label.width - text_width as f32) * 0.5).max(0.0),
            label.y + ((label.height - text_height as f32) * 0.5).max(0.0),
            label.color,
        ));
    }

    if color_vertices.is_empty() && text_items.is_empty() {
        return;
    }

    let mut state_block = ptr::null_mut();
    let has_state_block =
        create_state_block(device, D3DSBT_ALL, &mut state_block) >= 0 && !state_block.is_null();

    if begin_scene(device) < 0 {
        if has_state_block {
            release_state_block(state_block);
        }
        return;
    }

    setup_render_state(device);
    draw_color_vertices(device, &color_vertices);
    for (texture, x, y, color) in &text_items {
        if !texture.is_null() {
            draw_text_texture(device, &**texture, *x, *y, *color);
        }
    }
    end_scene(device);

    if has_state_block {
        apply_state_block(state_block);
        release_state_block(state_block);
    }
}

fn push_rect(vertices: &mut Vec<ColorVertex>, x: f32, y: f32, w: f32, h: f32, color: u32) {
    let x0 = x - 0.5;
    let y0 = y - 0.5;
    let x1 = x + w - 0.5;
    let y1 = y + h - 0.5;
    vertices.extend_from_slice(&[
        color_vertex(x0, y0, color),
        color_vertex(x1, y0, color),
        color_vertex(x1, y1, color),
        color_vertex(x0, y0, color),
        color_vertex(x1, y1, color),
        color_vertex(x0, y1, color),
    ]);
}

fn color_vertex(x: f32, y: f32, color: u32) -> ColorVertex {
    ColorVertex {
        x,
        y,
        z: 0.0,
        rhw: 1.0,
        color,
    }
}

fn texture_vertices(texture: &TextTexture, x: f32, y: f32, color: u32) -> [TextureVertex; 6] {
    let x0 = x - 0.5;
    let y0 = y - 0.5;
    let x1 = x + texture.width as f32 - 0.5;
    let y1 = y + texture.height as f32 - 0.5;
    [
        texture_vertex(x0, y0, color, 0.0, 0.0),
        texture_vertex(x1, y0, color, 1.0, 0.0),
        texture_vertex(x1, y1, color, 1.0, 1.0),
        texture_vertex(x0, y0, color, 0.0, 0.0),
        texture_vertex(x1, y1, color, 1.0, 1.0),
        texture_vertex(x0, y1, color, 0.0, 1.0),
    ]
}

fn texture_vertex(x: f32, y: f32, color: u32, u: f32, v: f32) -> TextureVertex {
    TextureVertex {
        x,
        y,
        z: 0.0,
        rhw: 1.0,
        color,
        u,
        v,
    }
}

fn measure_text(text: &str) -> Option<(i32, i32)> {
    unsafe {
        let hdc = CreateCompatibleDC(0);
        if hdc == 0 {
            return None;
        }
        let font = CreateFontA(
            18,
            0,
            0,
            0,
            FW_BOLD,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            0,
            0,
            ANTIALIASED_QUALITY,
            0,
            b"Segoe UI\0".as_ptr(),
        );
        let old_font = SelectObject(hdc, font);
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut size = Size { cx: 0, cy: 0 };
        let ok = GetTextExtentPoint32W(hdc, wide.as_ptr(), wide.len() as i32, &mut size) != 0;
        SelectObject(hdc, old_font);
        DeleteObject(font);
        DeleteDC(hdc);
        ok.then_some((size.cx + 1, size.cy + 1))
    }
}

unsafe fn cached_text_texture(
    cache: &mut Vec<CachedTextTexture>,
    device: *mut c_void,
    text: &str,
    width: u32,
    height: u32,
) -> Option<*const TextTexture> {
    let device_key = device as usize;
    if cache
        .first()
        .is_some_and(|entry| entry.device != device_key)
    {
        for entry in cache.iter() {
            release_unknown(entry.texture.texture);
        }
        cache.clear();
    }
    if let Some(index) = cache.iter().position(|entry| entry.text == text) {
        return Some(&cache[index].texture as *const TextTexture);
    }

    let width = width.max(1);
    let height = height.max(1);
    let bitmap = render_text_bitmap(text, width, height)?;
    let texture = create_texture_from_bitmap(device, width, height, &bitmap)?;
    cache.push(CachedTextTexture {
        device: device_key,
        text: text.to_owned(),
        texture,
    });
    cache
        .last()
        .map(|entry| &entry.texture as *const TextTexture)
}

unsafe fn render_text_bitmap(text: &str, width: u32, height: u32) -> Option<Vec<u8>> {
    let hdc = CreateCompatibleDC(0);
    if hdc == 0 {
        return None;
    }

    let mut bits = ptr::null_mut::<c_void>();
    let mut info = BitmapInfo {
        header: BitmapInfoHeader {
            size: std::mem::size_of::<BitmapInfoHeader>() as u32,
            width: width as i32,
            height: -(height as i32),
            planes: 1,
            bit_count: 32,
            compression: BI_RGB,
            size_image: width * height * 4,
            x_pels_per_meter: 0,
            y_pels_per_meter: 0,
            clr_used: 0,
            clr_important: 0,
        },
        colors: [0],
    };
    let bitmap = CreateDIBSection(hdc, &mut info, DIB_RGB_COLORS, &mut bits, 0, 0);
    if bitmap == 0 || bits.is_null() {
        DeleteDC(hdc);
        return None;
    }

    let old_bitmap = SelectObject(hdc, bitmap);
    let font = CreateFontA(
        18,
        0,
        0,
        0,
        FW_BOLD,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        0,
        0,
        ANTIALIASED_QUALITY,
        0,
        b"Segoe UI\0".as_ptr(),
    );
    let old_font = SelectObject(hdc, font);
    SetBkMode(hdc, TRANSPARENT_BK);
    SetTextColor(hdc, 0x00FFFFFF);

    let wide: Vec<u16> = text.encode_utf16().collect();
    TextOutW(hdc, 0, 0, wide.as_ptr(), wide.len() as i32);

    let len = (width * height * 4) as usize;
    let mut out = vec![0u8; len];
    ptr::copy_nonoverlapping(bits.cast::<u8>(), out.as_mut_ptr(), len);
    for pixel in out.as_chunks_mut::<4>().0 {
        let alpha = pixel[0].max(pixel[1]).max(pixel[2]);
        if alpha == 0 {
            pixel[0] = 0;
            pixel[1] = 0;
            pixel[2] = 0;
            pixel[3] = 0;
        } else {
            pixel[0] = 255;
            pixel[1] = 255;
            pixel[2] = 255;
            pixel[3] = alpha;
        }
    }

    SelectObject(hdc, old_font);
    SelectObject(hdc, old_bitmap);
    DeleteObject(font);
    DeleteObject(bitmap);
    DeleteDC(hdc);
    Some(out)
}

unsafe fn create_texture_from_bitmap(
    device: *mut c_void,
    width: u32,
    height: u32,
    bitmap: &[u8],
) -> Option<TextTexture> {
    let mut texture = ptr::null_mut();
    if create_texture(
        device,
        width,
        height,
        1,
        D3DUSAGE_DYNAMIC,
        D3DFMT_A8R8G8B8,
        D3DPOOL_DEFAULT,
        &mut texture,
        ptr::null_mut(),
    ) < 0
        || texture.is_null()
    {
        return None;
    }

    let mut locked = D3dLockedRect {
        pitch: 0,
        bits: ptr::null_mut(),
    };
    if lock_texture_rect(texture, 0, &mut locked, ptr::null(), D3DLOCK_DISCARD) < 0
        || locked.bits.is_null()
    {
        release_unknown(texture);
        return None;
    }

    let src_pitch = width as usize * 4;
    let dst_pitch = locked.pitch as usize;
    for row in 0..height as usize {
        ptr::copy_nonoverlapping(
            bitmap.as_ptr().add(row * src_pitch),
            locked.bits.cast::<u8>().add(row * dst_pitch),
            src_pitch,
        );
    }
    unlock_texture_rect(texture, 0);

    Some(TextTexture {
        texture,
        width,
        height,
    })
}

unsafe fn vtable_entry(instance: *mut c_void, index: usize) -> usize {
    let vtable = *(instance as *const *const usize);
    *vtable.add(index)
}

unsafe fn begin_scene(device: *mut c_void) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void) -> i32 =
        std::mem::transmute(vtable_entry(device, 41));
    func(device)
}

unsafe fn end_scene(device: *mut c_void) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void) -> i32 =
        std::mem::transmute(vtable_entry(device, 42));
    func(device)
}

unsafe fn get_viewport(device: *mut c_void, viewport: *mut Viewport) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void, *mut Viewport) -> i32 =
        std::mem::transmute(vtable_entry(device, 48));
    func(device, viewport)
}

unsafe fn set_render_state(device: *mut c_void, state: u32, value: u32) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void, u32, u32) -> i32 =
        std::mem::transmute(vtable_entry(device, 57));
    func(device, state, value)
}

unsafe fn create_state_block(device: *mut c_void, ty: u32, state_block: *mut *mut c_void) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 =
        std::mem::transmute(vtable_entry(device, 59));
    func(device, ty, state_block)
}

unsafe fn set_texture(device: *mut c_void, stage: u32, texture: *mut c_void) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32 =
        std::mem::transmute(vtable_entry(device, 65));
    func(device, stage, texture)
}

unsafe fn set_texture_stage_state(device: *mut c_void, stage: u32, ty: u32, value: u32) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void, u32, u32, u32) -> i32 =
        std::mem::transmute(vtable_entry(device, 67));
    func(device, stage, ty, value)
}

unsafe fn draw_primitive_up(
    device: *mut c_void,
    primitive_type: u32,
    primitive_count: u32,
    data: *const c_void,
    stride: u32,
) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void, u32, u32, *const c_void, u32) -> i32 =
        std::mem::transmute(vtable_entry(device, 83));
    func(device, primitive_type, primitive_count, data, stride)
}

unsafe fn set_fvf(device: *mut c_void, fvf: u32) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void, u32) -> i32 =
        std::mem::transmute(vtable_entry(device, 89));
    func(device, fvf)
}

unsafe fn set_vertex_shader(device: *mut c_void, shader: *mut c_void) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 =
        std::mem::transmute(vtable_entry(device, 92));
    func(device, shader)
}

unsafe fn set_pixel_shader(device: *mut c_void, shader: *mut c_void) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32 =
        std::mem::transmute(vtable_entry(device, 107));
    func(device, shader)
}

unsafe fn create_texture(
    device: *mut c_void,
    width: u32,
    height: u32,
    levels: u32,
    usage: u32,
    format: u32,
    pool: u32,
    texture: *mut *mut c_void,
    shared_handle: *mut usize,
) -> i32 {
    let func: unsafe extern "system" fn(
        *mut c_void,
        u32,
        u32,
        u32,
        u32,
        u32,
        u32,
        *mut *mut c_void,
        *mut usize,
    ) -> i32 = std::mem::transmute(vtable_entry(device, 23));
    func(
        device,
        width,
        height,
        levels,
        usage,
        format,
        pool,
        texture,
        shared_handle,
    )
}

unsafe fn lock_texture_rect(
    texture: *mut c_void,
    level: u32,
    locked: *mut D3dLockedRect,
    rect: *const c_void,
    flags: u32,
) -> i32 {
    let func: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut D3dLockedRect,
        *const c_void,
        u32,
    ) -> i32 = std::mem::transmute(vtable_entry(texture, 19));
    func(texture, level, locked, rect, flags)
}

unsafe fn unlock_texture_rect(texture: *mut c_void, level: u32) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void, u32) -> i32 =
        std::mem::transmute(vtable_entry(texture, 20));
    func(texture, level)
}

unsafe fn setup_render_state(device: *mut c_void) {
    set_texture(device, 0, ptr::null_mut());
    set_vertex_shader(device, ptr::null_mut());
    set_pixel_shader(device, ptr::null_mut());
    set_render_state(device, D3DRS_ZENABLE, 0);
    set_render_state(device, D3DRS_FILLMODE, D3DFILL_SOLID);
    set_render_state(device, D3DRS_CULLMODE, D3DCULL_NONE);
    set_render_state(device, D3DRS_LIGHTING, 0);
    set_render_state(device, D3DRS_FOGENABLE, 0);
    set_render_state(device, D3DRS_SCISSORTESTENABLE, 0);
    set_render_state(device, D3DRS_ALPHABLENDENABLE, 1);
    set_render_state(device, D3DRS_SRCBLEND, D3DBLEND_SRCALPHA);
    set_render_state(device, D3DRS_DESTBLEND, D3DBLEND_INVSRCALPHA);
    set_texture_stage_state(device, 0, D3DTSS_COLOROP, D3DTOP_MODULATE);
    set_texture_stage_state(device, 0, D3DTSS_COLORARG1, D3DTA_TEXTURE);
    set_texture_stage_state(device, 0, D3DTSS_COLORARG2, D3DTA_DIFFUSE);
    set_texture_stage_state(device, 0, D3DTSS_ALPHAOP, D3DTOP_MODULATE);
    set_texture_stage_state(device, 0, D3DTSS_ALPHAARG1, D3DTA_TEXTURE);
    set_texture_stage_state(device, 0, D3DTSS_ALPHAARG2, D3DTA_DIFFUSE);
}

unsafe fn draw_color_vertices(device: *mut c_void, vertices: &[ColorVertex]) {
    let primitive_count = (vertices.len() / 3) as u32;
    if primitive_count > 0 {
        set_texture(device, 0, ptr::null_mut());
        set_texture_stage_state(device, 0, D3DTSS_COLOROP, D3DTOP_SELECTARG1);
        set_texture_stage_state(device, 0, D3DTSS_COLORARG1, D3DTA_DIFFUSE);
        set_texture_stage_state(device, 0, D3DTSS_ALPHAOP, D3DTOP_SELECTARG1);
        set_texture_stage_state(device, 0, D3DTSS_ALPHAARG1, D3DTA_DIFFUSE);
        set_fvf(device, D3DFVF_XYZRHW | D3DFVF_DIFFUSE);
        draw_primitive_up(
            device,
            D3DPT_TRIANGLELIST,
            primitive_count,
            vertices.as_ptr().cast(),
            std::mem::size_of::<ColorVertex>() as u32,
        );
    }
}

unsafe fn draw_text_texture(
    device: *mut c_void,
    texture: &TextTexture,
    x: f32,
    y: f32,
    color: u32,
) {
    let vertices = texture_vertices(texture, x, y, color);
    set_texture(device, 0, texture.texture);
    set_texture_stage_state(device, 0, D3DTSS_COLOROP, D3DTOP_MODULATE);
    set_texture_stage_state(device, 0, D3DTSS_COLORARG1, D3DTA_TEXTURE);
    set_texture_stage_state(device, 0, D3DTSS_COLORARG2, D3DTA_DIFFUSE);
    set_texture_stage_state(device, 0, D3DTSS_ALPHAOP, D3DTOP_MODULATE);
    set_texture_stage_state(device, 0, D3DTSS_ALPHAARG1, D3DTA_TEXTURE);
    set_texture_stage_state(device, 0, D3DTSS_ALPHAARG2, D3DTA_DIFFUSE);
    set_fvf(device, D3DFVF_XYZRHW | D3DFVF_DIFFUSE | D3DFVF_TEX1);
    draw_primitive_up(
        device,
        D3DPT_TRIANGLELIST,
        2,
        vertices.as_ptr().cast(),
        std::mem::size_of::<TextureVertex>() as u32,
    );
}

unsafe fn apply_state_block(state_block: *mut c_void) -> i32 {
    let func: unsafe extern "system" fn(*mut c_void) -> i32 =
        std::mem::transmute(vtable_entry(state_block, 5));
    func(state_block)
}

unsafe fn release_state_block(state_block: *mut c_void) -> u32 {
    release_unknown(state_block)
}

unsafe fn release_unknown(instance: *mut c_void) -> u32 {
    let func: unsafe extern "system" fn(*mut c_void) -> u32 =
        std::mem::transmute(vtable_entry(instance, 2));
    func(instance)
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: usize) -> usize;
    fn DeleteDC(hdc: usize) -> i32;
    fn CreateDIBSection(
        hdc: usize,
        info: *mut BitmapInfo,
        usage: u32,
        bits: *mut *mut c_void,
        section: usize,
        offset: u32,
    ) -> usize;
    fn CreateFontA(
        h: i32,
        w: i32,
        esc: i32,
        orient: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strikeout: u32,
        charset: u32,
        out_prec: u32,
        clip_prec: u32,
        quality: u32,
        pitch: u32,
        face: *const u8,
    ) -> usize;
    fn SelectObject(hdc: usize, obj: usize) -> usize;
    fn DeleteObject(obj: usize) -> i32;
    fn SetBkMode(hdc: usize, mode: i32) -> i32;
    fn SetTextColor(hdc: usize, color: u32) -> u32;
    fn GetTextExtentPoint32W(hdc: usize, text: *const u16, count: i32, size: *mut Size) -> i32;
    fn TextOutW(hdc: usize, x: i32, y: i32, text: *const u16, count: i32) -> i32;
}
