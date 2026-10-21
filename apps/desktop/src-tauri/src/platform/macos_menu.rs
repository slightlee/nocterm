//! macOS 原生应用菜单。
//!
//! 菜单外壳和标准编辑/窗口行为交给系统，Nocterm 业务动作通过一个受控事件交给
//! React 应用壳。这样不会绕过前端已有的标签关闭确认与会话生命周期。

use tauri::{
    AppHandle, Builder, Emitter, Manager, Runtime, WindowEvent,
    menu::{HELP_SUBMENU_ID, Menu, MenuItem, PredefinedMenuItem, Submenu},
};
use tauri_plugin_opener::OpenerExt;

pub const APP_MENU_EVENT: &str = "nocterm://app-menu-action";

const OPEN_ABOUT: &str = "open-about";
const OPEN_SETTINGS: &str = "open-settings";
const NEW_LOCAL_TERMINAL: &str = "new-local-terminal";
const CLOSE_CURRENT_TAB: &str = "close-current-tab";
const TOGGLE_SIDEBAR: &str = "toggle-sidebar";
const TOGGLE_AI: &str = "toggle-ai";
const OPEN_PROJECT: &str = "open-project";
const REPORT_ISSUE: &str = "report-issue";

const PROJECT_URL: &str = "https://github.com/slightlee/nocterm";
const ISSUE_URL: &str = "https://github.com/slightlee/nocterm/issues/new/choose";

// 编辑菜单是主菜单中的第三项；7 个节点包含撤销、重做、分隔线和四个编辑动作。
// AppKit/WebKit 只会在这些自有节点之后追加 AutoFill、听写和字符面板入口。
const EDIT_MENU_INDEX: isize = 2;
const EDIT_MENU_ITEM_COUNT: isize = 7;

/// 仅在 macOS 装配原生菜单；Windows 继续使用现有自绘标题栏与窗口控件。
pub fn configure<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    disable_optional_system_edit_items();

    builder
        .menu(build_menu)
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .on_window_event(|window, event| {
            if matches!(event, WindowEvent::Focused(true)) {
                let app = window.app_handle().clone();
                if let Err(error) = app.run_on_main_thread(trim_automatic_edit_menu_items) {
                    eprintln!("[nocterm] 调度 macOS 编辑菜单清理失败: {error}");
                }
            }
        })
}

/// WebView 在 Tauri 的 setup 回调前创建，初次清理必须放在此时才能覆盖 AutoFill。
pub fn finish_setup<R: Runtime>(app: &AppHandle<R>) {
    if let Err(error) = app.run_on_main_thread(trim_automatic_edit_menu_items) {
        eprintln!("[nocterm] 调度 macOS 编辑菜单初始化失败: {error}");
    }
}

fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let app_menu = Submenu::with_items(
        app,
        app.package_info().name.clone(),
        true,
        &[
            &action(app, OPEN_ABOUT, "关于 Nocterm", None)?,
            &action(app, OPEN_SETTINGS, "设置…", Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, Some("服务"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some("隐藏 Nocterm"))?,
            &PredefinedMenuItem::hide_others(app, Some("隐藏其他应用"))?,
            &PredefinedMenuItem::show_all(app, Some("全部显示"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some("退出 Nocterm"))?,
        ],
    )?;

    let session_menu = Submenu::with_items(
        app,
        "会话",
        true,
        &[
            &action(app, NEW_LOCAL_TERMINAL, "新建本地终端", Some("CmdOrCtrl+T"))?,
            &action(
                app,
                CLOSE_CURRENT_TAB,
                "关闭当前标签页",
                Some("CmdOrCtrl+W"),
            )?,
        ],
    )?;

    let edit_menu = Submenu::with_items(
        app,
        "编辑",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("撤销"))?,
            &PredefinedMenuItem::redo(app, Some("重做"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("剪切"))?,
            &PredefinedMenuItem::copy(app, Some("复制"))?,
            &PredefinedMenuItem::paste(app, Some("粘贴"))?,
            &PredefinedMenuItem::select_all(app, Some("全选"))?,
        ],
    )?;

    let view_menu = Submenu::with_items(
        app,
        "显示",
        true,
        &[
            &action(app, TOGGLE_SIDEBAR, "显示或隐藏连接侧栏", None)?,
            &action(app, TOGGLE_AI, "显示或隐藏智能助手", None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::fullscreen(app, Some("切换全屏幕"))?,
        ],
    )?;

    // 不注册为 NSApplication 的 Window Menu，避免 AppKit 自动追加窗口排列、设备迁移和窗口列表。
    // 两个保留项仍是原生 First Responder 动作，不影响交通灯按钮或系统窗口快捷键。
    let window_menu = Submenu::with_items(
        app,
        "窗口",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("最小化"))?,
            &PredefinedMenuItem::maximize(app, Some("缩放"))?,
        ],
    )?;

    let help_menu = Submenu::with_id_and_items(
        app,
        HELP_SUBMENU_ID,
        "帮助",
        true,
        &[
            &action(app, OPEN_PROJECT, "项目主页", None)?,
            &action(app, REPORT_ISSUE, "报告问题…", None)?,
        ],
    )?;

    Menu::with_items(
        app,
        &[
            &app_menu,
            &session_menu,
            &edit_menu,
            &view_menu,
            &window_menu,
            &help_menu,
        ],
    )
}

fn action<R: Runtime>(
    app: &AppHandle<R>,
    id: &'static str,
    text: &'static str,
    accelerator: Option<&'static str>,
) -> tauri::Result<MenuItem<R>> {
    MenuItem::with_id(app, id, text, true, accelerator)
}

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, id: &str) {
    match id {
        OPEN_PROJECT => open_external(app, PROJECT_URL),
        REPORT_ISSUE => open_external(app, ISSUE_URL),
        _ => {
            let Some(action) = frontend_action(id) else {
                return;
            };
            if let Err(error) = app.emit(APP_MENU_EVENT, action) {
                eprintln!("[nocterm] 发送应用菜单事件失败: {error}");
            }
        }
    }
}

fn frontend_action(id: &str) -> Option<&'static str> {
    match id {
        OPEN_ABOUT => Some(OPEN_ABOUT),
        OPEN_SETTINGS => Some(OPEN_SETTINGS),
        NEW_LOCAL_TERMINAL => Some(NEW_LOCAL_TERMINAL),
        CLOSE_CURRENT_TAB => Some(CLOSE_CURRENT_TAB),
        TOGGLE_SIDEBAR => Some(TOGGLE_SIDEBAR),
        TOGGLE_AI => Some(TOGGLE_AI),
        _ => None,
    }
}

fn open_external<R: Runtime>(app: &AppHandle<R>, url: &'static str) {
    if let Err(error) = app.opener().open_url(url, None::<&str>) {
        eprintln!("[nocterm] 打开菜单外部链接失败: {error}");
    }
}

fn disable_optional_system_edit_items() {
    // 这两个应用级偏好是 AppKit 提供的稳定入口，必须在设置主菜单前写入。
    for key in [
        c"NSDisabledDictationMenuItem",
        c"NSDisabledCharacterPaletteMenuItem",
    ] {
        if let Err(error) = native::set_application_default_bool(key, true) {
            eprintln!("[nocterm] 禁用 macOS 系统编辑菜单项失败: {error}");
        }
    }
}

fn trim_automatic_edit_menu_items() {
    let remove_count = match native::edit_menu_item_count() {
        Ok(item_count) => removable_edit_item_count(item_count),
        Err(error) => {
            eprintln!("[nocterm] 读取 macOS 编辑菜单失败: {error}");
            return;
        }
    };

    for _ in 0..remove_count {
        if let Err(error) = native::remove_edit_menu_item(EDIT_MENU_ITEM_COUNT) {
            eprintln!("[nocterm] 清理 macOS 自动编辑菜单项失败: {error}");
            return;
        }
    }
}

fn removable_edit_item_count(item_count: isize) -> usize {
    item_count.saturating_sub(EDIT_MENU_ITEM_COUNT).max(0) as usize
}

/// 仅封装当前菜单需求用到的公开 Objective-C runtime/AppKit 调用，避免把平台类型泄露出去。
mod native {
    use std::ffi::{CStr, c_char, c_void};

    type Object = *mut c_void;
    type Selector = *const c_void;

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Object;
        fn objc_msgSend();
        fn sel_registerName(name: *const c_char) -> Selector;
    }

    pub(super) fn set_application_default_bool(
        key: &'static CStr,
        value: bool,
    ) -> Result<(), &'static str> {
        // SAFETY: All selectors are public Foundation APIs with signatures mirrored by the
        // typed send helpers below. Returned Objective-C objects remain owned by their frameworks.
        unsafe {
            let defaults =
                send_object(class(c"NSUserDefaults")?, selector(c"standardUserDefaults"));
            let key = send_c_string(
                class(c"NSString")?,
                selector(c"stringWithUTF8String:"),
                key.as_ptr(),
            );
            if defaults.is_null() || key.is_null() {
                return Err("Foundation 未返回应用偏好对象");
            }
            send_bool_object(defaults, selector(c"setBool:forKey:"), value, key);
        }
        Ok(())
    }

    pub(super) fn edit_menu_item_count() -> Result<isize, &'static str> {
        // SAFETY: The call chain only reads NSApplication.mainMenu and its third submenu on the
        // main thread. Each nullable Objective-C return is checked before the next message.
        let menu = unsafe { edit_menu()? };
        Ok(unsafe { send_isize(menu, selector(c"numberOfItems")) })
    }

    pub(super) fn remove_edit_menu_item(index: isize) -> Result<(), &'static str> {
        // SAFETY: The caller obtains the current count immediately before removal and always
        // removes the first system-owned trailing item at EDIT_MENU_ITEM_COUNT.
        let menu = unsafe { edit_menu()? };
        unsafe { send_index(menu, selector(c"removeItemAtIndex:"), index) };
        Ok(())
    }

    unsafe fn edit_menu() -> Result<Object, &'static str> {
        let application =
            unsafe { send_object(class(c"NSApplication")?, selector(c"sharedApplication")) };
        if application.is_null() {
            return Err("NSApplication 不可用");
        }

        let main_menu = unsafe { send_object(application, selector(c"mainMenu")) };
        if main_menu.is_null() {
            return Err("主菜单尚未初始化");
        }

        let top_level_count = unsafe { send_isize(main_menu, selector(c"numberOfItems")) };
        if top_level_count <= super::EDIT_MENU_INDEX {
            return Err("主菜单缺少编辑菜单");
        }

        let edit_item = unsafe {
            send_object_index(main_menu, selector(c"itemAtIndex:"), super::EDIT_MENU_INDEX)
        };
        let edit_menu = unsafe { send_object(edit_item, selector(c"submenu")) };
        if edit_menu.is_null() {
            return Err("编辑菜单尚未初始化");
        }
        Ok(edit_menu)
    }

    unsafe fn class(name: &CStr) -> Result<Object, &'static str> {
        let class = unsafe { objc_getClass(name.as_ptr()) };
        if class.is_null() {
            Err("Objective-C 类不可用")
        } else {
            Ok(class)
        }
    }

    fn selector(name: &CStr) -> Selector {
        // SAFETY: All call sites pass static, NUL-terminated selector names.
        unsafe { sel_registerName(name.as_ptr()) }
    }

    unsafe fn send_object(receiver: Object, selector: Selector) -> Object {
        let send: unsafe extern "C" fn(Object, Selector) -> Object =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        unsafe { send(receiver, selector) }
    }

    unsafe fn send_c_string(receiver: Object, selector: Selector, value: *const c_char) -> Object {
        let send: unsafe extern "C" fn(Object, Selector, *const c_char) -> Object =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        unsafe { send(receiver, selector, value) }
    }

    unsafe fn send_bool_object(receiver: Object, selector: Selector, value: bool, object: Object) {
        let send: unsafe extern "C" fn(Object, Selector, bool, Object) =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        unsafe { send(receiver, selector, value, object) }
    }

    unsafe fn send_object_index(receiver: Object, selector: Selector, index: isize) -> Object {
        let send: unsafe extern "C" fn(Object, Selector, isize) -> Object =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        unsafe { send(receiver, selector, index) }
    }

    unsafe fn send_isize(receiver: Object, selector: Selector) -> isize {
        let send: unsafe extern "C" fn(Object, Selector) -> isize =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        unsafe { send(receiver, selector) }
    }

    unsafe fn send_index(receiver: Object, selector: Selector, index: isize) {
        let send: unsafe extern "C" fn(Object, Selector, isize) =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        unsafe { send(receiver, selector, index) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_only_frontend_owned_actions() {
        assert_eq!(frontend_action(OPEN_SETTINGS), Some(OPEN_SETTINGS));
        assert_eq!(frontend_action(CLOSE_CURRENT_TAB), Some(CLOSE_CURRENT_TAB));
        assert_eq!(frontend_action(OPEN_PROJECT), None);
        assert_eq!(frontend_action("unknown"), None);
    }

    #[test]
    fn removes_only_items_appended_after_the_owned_edit_menu() {
        assert_eq!(removable_edit_item_count(6), 0);
        assert_eq!(removable_edit_item_count(EDIT_MENU_ITEM_COUNT), 0);
        assert_eq!(removable_edit_item_count(EDIT_MENU_ITEM_COUNT + 4), 4);
    }
}
