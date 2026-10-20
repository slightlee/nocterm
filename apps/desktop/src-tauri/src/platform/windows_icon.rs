//! Windows 平台适配：窗口图标按 DPI 精确设置。
//!
//! tao 注册窗口类时未设置类图标（HICON 为空），系统回退为「从 exe 图标
//! 资源按 32px 基准加载再拉伸」，在 >100% 显示缩放下任务栏发糊；桌面快捷
//! 方式走 shell 智能选帧路径不受影响。
//!
//! `LoadImageW` 按自定义尺寸加载多帧 ico 时可能选中较小的帧再拉伸（结果
//! 与系统默认一样糊），因此这里直接解析内嵌 icon.ico 的帧目录，取与目标
//! DPI 尺寸一致的原始帧，用 CreateIconFromResourceEx 以原生尺寸生成
//! HICON（零重采样），再经 WM_SETICON 设置：大图标管任务栏与 Alt-Tab，
//! 小图标管标题栏。

use tauri::WebviewWindow;

/// WM_SETICON 消息与 wParam 常量（避免引入额外 feature 依赖）。
const WM_SETICON: u32 = 0x0080;
const ICON_SMALL: usize = 0;
const ICON_BIG: usize = 1;

/// tauri-winres `set_icon` 默认把 icon.ico 嵌入为资源 ID 32512。
const RESOURCE_ICON_ID: isize = 32512;

/// CreateIconFromResourceEx 要求的 PNG 图标格式版本（Vista+ 支持）。
const PNG_ICON_VERSION: u32 = 0x0003_0000;
const LR_DEFAULTCOLOR: u32 = 0x0000_0000;
/// 与发布图标同源的完整多帧 ico（16~256px）。
const ICON_FILE: &[u8] = include_bytes!("../../icons/icon.ico");

/// 为主窗口设置 DPI 匹配图标，并监听缩放变化自动重设（跨显示器拖动）。
pub fn attach_dpi_aware_icon(window: &WebviewWindow) {
    apply(window);
    let handle = window.clone();
    window.on_window_event(move |event| {
        // 切换显示器导致缩放变化时，按新 DPI 重新生成两个尺寸的 HICON。
        if let tauri::WindowEvent::ScaleFactorChanged { .. } = event {
            apply(&handle);
        }
    });
}

fn apply(window: &WebviewWindow) {
    if let Err(error) = apply_once(window) {
        eprintln!("[nocterm] 设置窗口 DPI 图标失败: {error}");
    }
}

fn apply_once(window: &WebviewWindow) -> Result<(), String> {
    // hwnd() 返回 windows crate 的 HWND，统一转 isize 再按 windows-sys 目标类型还原。
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    // SAFETY: hwnd 来自当前窗口实例；HICON 由进程持有至退出，不提前销毁。
    let (big_icon, small_icon) = unsafe { set_icons(hwnd) };
    if big_icon == 0 || small_icon == 0 {
        return Err("HICON 生成失败，任务栏将回退系统默认拉伸行为".to_string());
    }
    Ok(())
}

/// 返回设置后的 (大图标, 小图标) 句柄；0 表示该项加载失败。
unsafe fn set_icons(hwnd: isize) -> (isize, isize) {
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW;

    // SAFETY: hwnd 来自当前窗口实例，GetDpiForWindow 仅做查询。
    let dpi = unsafe { GetDpiForWindow(hwnd as _).max(96) } as u32;
    // 大图标基准 32px、小图标基准 16px，均按 DPI 线性放大到整数像素。
    let big_size = (dpi * 32 / 96) as i32;
    let small_size = (dpi * 16 / 96) as i32;

    // SAFETY: 均为查询/加载型调用，入参为常量与窗口自身句柄。
    let (big_icon, small_icon) = unsafe { (load_icon(big_size), load_icon(small_size)) };
    eprintln!(
        "[nocterm] DPI 图标: dpi={dpi} big={big_size}px→{big_icon:#x} small={small_size}px→{small_icon:#x}"
    );

    // SAFETY: WM_SETICON 的 wParam/lParam 按文档取值，HICON 由进程持有至退出。
    unsafe {
        SendMessageW(hwnd as _, WM_SETICON, ICON_SMALL as _, small_icon as _);
        SendMessageW(hwnd as _, WM_SETICON, ICON_BIG as _, big_icon as _);
    }
    (big_icon, small_icon)
}

/// 按目标 DPI 尺寸加载窗口图标：
/// 1. 解析内嵌 icon.ico 帧目录，选尺寸一致（或最接近）的原始帧，
///    以原生尺寸生成 HICON——绝不重采样；
/// 2. 解析失败再用 LoadImageW 读 exe 资源兜底（内部可能选小帧拉伸）。
///
/// 两者都失败返回 0。
unsafe fn load_icon(size: i32) -> isize {
    // SAFETY: CreateIconFromResourceEx 输入为静态内嵌数据。
    if let Some(icon) = unsafe { icon_from_ico_frame(size) } {
        return icon;
    }
    eprintln!("[nocterm] icon.ico 未命中 {size}px 帧，回退资源加载");
    // SAFETY: 查询型调用，资源 ID 为整数形式（MAKEINTRESOURCE 语义）。
    unsafe { icon_from_resource(size) }
}

/// 在内嵌 ico 中找目标尺寸帧并按原生尺寸生成 HICON；返回 None 表示无命中。
unsafe fn icon_from_ico_frame(size: i32) -> Option<isize> {
    use windows_sys::Win32::UI::WindowsAndMessaging::CreateIconFromResourceEx;

    // ICO 目录结构：2 字节保留 + 2 字节类型 + 2 字节帧数，每帧目录 16 字节。
    let count = u16::from_le_bytes([ICON_FILE[4], ICON_FILE[5]]) as usize;
    let mut frames: Vec<(i32, &[u8])> = Vec::with_capacity(count);
    for index in 0..count {
        let entry = 6 + index * 16;
        if entry + 16 > ICON_FILE.len() {
            return None;
        }
        // 宽高字节即像素值，仅 0 表示 256px；ico 帧宽高恒相等。
        let width = match ICON_FILE[entry] {
            0 => 256,
            byte => i32::from(byte),
        };
        let data_size =
            u32::from_le_bytes(ICON_FILE[entry + 8..entry + 12].try_into().ok()?) as usize;
        let data_offset =
            u32::from_le_bytes(ICON_FILE[entry + 12..entry + 16].try_into().ok()?) as usize;
        if data_offset + data_size > ICON_FILE.len() {
            return None;
        }
        frames.push((width, &ICON_FILE[data_offset..data_offset + data_size]));
    }

    // 选帧策略：精确命中优先；否则取大于目标的最小帧（只缩不放），再退最大帧。
    let frame = frames
        .iter()
        .find(|(frame_width, _)| *frame_width == size)
        .or_else(|| {
            frames
                .iter()
                .filter(|(frame_width, _)| *frame_width > size)
                .min_by_key(|(frame_width, _)| *frame_width)
        })
        .or_else(|| frames.iter().max_by_key(|(frame_width, _)| *frame_width))?;

    // cx/cy 传 0 且未设 LR_DEFAULTSIZE：按帧原生尺寸生成，零重采样。
    let handle = unsafe {
        CreateIconFromResourceEx(
            frame.1.as_ptr(),
            frame.1.len() as u32,
            1,
            PNG_ICON_VERSION,
            0,
            0,
            LR_DEFAULTCOLOR,
        ) as isize
    };
    eprintln!("[nocterm] ico 帧 {size}px 命中 {}px→{handle:#x}", frame.0);
    (handle != 0).then_some(handle)
}

/// 兜底：LoadImageW 读 exe 资源（内部可能选小帧拉伸），仅作降级路径。
unsafe fn icon_from_resource(size: i32) -> isize {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{IMAGE_ICON, LoadImageW};

    // SAFETY: 查询型调用，资源 ID 为整数形式（MAKEINTRESOURCE 语义）。
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let handle = LoadImageW(
            instance,
            RESOURCE_ICON_ID as _,
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        );
        handle as isize
    }
}
