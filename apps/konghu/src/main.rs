//! 空毂的入口。双击后出现这一扇窗，并打开清机和浊机。

#![cfg_attr(not(test), windows_subsystem = "windows")]

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateFontW, COLOR_WINDOW, DEFAULT_CHARSET, FW_NORMAL};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const ID_QING: i32 = 1;
const ID_ZHUO: i32 = 2;

fn main() {
    let class = wide("Konghu");
    let title = wide("空毂");
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let mut window_class: WNDCLASSW = std::mem::zeroed();
        window_class.lpfnWndProc = Some(wndproc);
        window_class.hInstance = instance;
        window_class.lpszClassName = class.as_ptr();
        window_class.hCursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        window_class.hbrBackground = (COLOR_WINDOW + 1) as _;
        if RegisterClassW(&window_class) == 0 {
            alert("无法注册窗口");
            return;
        }
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            520,
            280,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            alert("无法创建窗口");
            return;
        }
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let font = CreateFontW(
                -18,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                0,
                0,
                wide("Microsoft YaHei").as_ptr(),
            );
            child(hwnd, "STATIC", "空毂是这一扇窗。清机负责签名，浊机负责联网。两扇窗随它一起打开。", WS_CHILD | WS_VISIBLE, 24, 24, 460, 48, 0, font);
            child(hwnd, "BUTTON", "打开清机", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32, 24, 96, 200, 40, ID_QING, font);
            child(hwnd, "BUTTON", "打开浊机", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32, 244, 96, 200, 40, ID_ZHUO, font);
            child(hwnd, "STATIC", "种子只出现在清机。浊机写简牍，清机签名，浊机再广播。", WS_CHILD | WS_VISIBLE, 24, 156, 460, 40, 0, font);
            open_sibling("qing.exe");
            open_sibling("zhuo.exe");
            0
        }
        WM_COMMAND => {
            if ((wparam >> 16) & 0xffff) as u16 != BN_CLICKED as u16 {
                return 0;
            }
            match (wparam & 0xffff) as i32 {
                ID_QING => open_sibling("qing.exe"),
                ID_ZHUO => open_sibling("zhuo.exe"),
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn open_sibling(name: &str) {
    let Ok(exe) = std::env::current_exe() else {
        alert("找不到程序所在目录");
        return;
    };
    let Some(dir) = exe.parent() else {
        alert("找不到程序所在目录");
        return;
    };
    let path = dir.join(name);
    if !path.is_file() {
        alert(&format!("旁边没有 {name}。它应该和空毂放在同一个文件夹。"));
        return;
    }
    let operation = wide("open");
    let file = wide(&path.display().to_string());
    let folder = wide(&dir.display().to_string());
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            folder.as_ptr(),
            SW_SHOWNORMAL,
        )
    };
    if (code as isize) <= 32 {
        alert(&format!("无法打开 {name}"));
    }
}

unsafe fn child(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: i32,
    font: windows_sys::Win32::Graphics::Gdi::HFONT,
) -> HWND {
    let class = wide(class);
    let text = wide(text);
    let hwnd = CreateWindowExW(
        0,
        class.as_ptr(),
        text.as_ptr(),
        style,
        x,
        y,
        w,
        h,
        parent,
        id as isize as HMENU,
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null(),
    );
    SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 1);
    hwnd
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn alert(text: &str) {
    let text = wide(text);
    let title = wide("空毂");
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONWARNING);
    }
}
