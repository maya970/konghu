//! 清机。用户自己打开它。浊机不能启动它，也读不到种子。

#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateFontW, COLOR_WINDOW, DEFAULT_CHARSET, FW_NORMAL};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{VirtualLock, VirtualUnlock};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetKeyState, VK_CONTROL};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use zeroize::{Zeroize, Zeroizing};

const ID_PASS: i32 = 1001;
const ID_PASS2: i32 = 1002;
const ID_DROP: i32 = 1003;
const ID_CONFIRM: i32 = 1004;
const ID_INTENT: i32 = 1005;
const ID_STATUS: i32 = 1006;
const ID_CREATE: i32 = 1101;
const ID_UNLOCK: i32 = 1102;
const ID_LOCK: i32 = 1103;
const ID_SAVE: i32 = 1104;
const ID_READ: i32 = 1105;
const ID_SIGN: i32 = 1106;

static ORIG_EDIT: AtomicIsize = AtomicIsize::new(0);
static INTENT_HWND: AtomicIsize = AtomicIsize::new(0);
static CONFIRM_HWND: AtomicIsize = AtomicIsize::new(0);
static PASS_HWND: AtomicIsize = AtomicIsize::new(0);
static PASS2_HWND: AtomicIsize = AtomicIsize::new(0);

struct Locked32(Box<Zeroizing<[u8; 32]>>);

impl Locked32 {
    fn new(bytes: [u8; 32]) -> Self {
        let data = Box::new(Zeroizing::new(bytes));
        unsafe {
            VirtualLock(data.as_ptr() as _, 32);
        }
        Self(data)
    }

    fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Drop for Locked32 {
    fn drop(&mut self) {
        unsafe {
            VirtualUnlock(self.0.as_ptr() as _, 32);
        }
    }
}

struct App {
    profile: PathBuf,
    vault_path: PathBuf,
    label_path: PathBuf,
    labels: Vec<([u8; 20], String)>,
    pass_edit: HWND,
    pass2_edit: HWND,
    drop_edit: HWND,
    confirm_edit: HWND,
    intent_edit: HWND,
    status_edit: HWND,
    sign_btn: HWND,
    secret: Option<Locked32>,
    tablet: Option<tablet::Tablet>,
}

struct View {
    ui: Ui,
    intent: HWND,
    status: HWND,
    sign: HWND,
    pass: HWND,
    pass2: HWND,
}

struct Ui {
    status: String,
    intent: Option<String>,
    sign_enabled: bool,
    clear_passwords: bool,
}

impl Drop for Ui {
    fn drop(&mut self) {
        self.status.zeroize();
        if let Some(intent) = self.intent.as_mut() {
            intent.zeroize();
        }
    }
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn main() {
    unsafe {
        SetProcessWorkingSetSize(GetCurrentProcess(), 8 * 1024 * 1024, 64 * 1024 * 1024);
    }
    let class = wide("KonghuQing");
    let title = wide("清机 — 空毂");
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
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            760,
            720,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            alert("无法创建窗口");
            return;
        }
        APP.with(|slot| {
            if let Some(app) = slot.borrow_mut().as_mut() {
                app.load_local();
            }
        });
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
            APP.with(|slot| *slot.borrow_mut() = Some(App::new(hwnd)));
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as i32;
            let code = ((wparam >> 16) & 0xffff) as u16;
            if code == EN_CHANGE as u16 && id == ID_CONFIRM {
                let update = APP.with(|slot| {
                    let Ok(borrow) = slot.try_borrow() else {
                        return None;
                    };
                    borrow.as_ref().map(|app| (app.sign_btn, app.confirm_ok()))
                });
                if let Some((sign, enabled)) = update {
                    EnableWindow(sign, if enabled { 1 } else { 0 });
                }
                return 0;
            }
            if code == EN_CHANGE as u16 || code != BN_CLICKED as u16 {
                return 0;
            }
            let view = APP.with(|slot| {
                let mut borrow = slot.borrow_mut();
                let app = borrow.as_mut()?;
                let ui = match id {
                    ID_CREATE => app.create_vault(),
                    ID_UNLOCK => app.unlock(),
                    ID_LOCK => app.lock("已锁定，种子已清零"),
                    ID_SAVE => app.save_drop(),
                    ID_READ => app.read_tablet(),
                    ID_SIGN => app.sign(),
                    _ => return None,
                };
                Some(View {
                    ui,
                    intent: app.intent_edit,
                    status: app.status_edit,
                    sign: app.sign_btn,
                    pass: app.pass_edit,
                    pass2: app.pass2_edit,
                })
            });
            if let Some(view) = view {
                apply(view);
            }
            0
        }
        WM_ENDSESSION | WM_DESTROY => {
            APP.with(|slot| {
                if let Some(app) = slot.borrow_mut().as_mut() {
                    app.secret = None;
                    app.tablet = None;
                }
            });
            if msg == WM_DESTROY {
                PostQuitMessage(0);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

impl App {
    fn new(parent: HWND) -> Self {
        unsafe {
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
            let label = |text, x, y, w| child(parent, "STATIC", text, WS_CHILD | WS_VISIBLE, x, y, w, 24, 0, font);
            label("口令", 16, 16, 70);
            let pass_edit = child(parent, "EDIT", "", edit_style() | ES_PASSWORD as u32, 90, 14, 250, 26, ID_PASS, font);
            label("再输入", 350, 16, 70);
            let pass2_edit = child(parent, "EDIT", "", edit_style() | ES_PASSWORD as u32, 430, 14, 250, 26, ID_PASS2, font);
            child(parent, "BUTTON", "创建", button_style(), 16, 50, 90, 28, ID_CREATE, font);
            child(parent, "BUTTON", "解锁", button_style(), 116, 50, 90, 28, ID_UNLOCK, font);
            child(parent, "BUTTON", "锁定", button_style(), 216, 50, 90, 28, ID_LOCK, font);
            label("投递目录", 16, 92, 70);
            let drop_edit = child(parent, "EDIT", "", edit_style(), 90, 90, 500, 26, ID_DROP, font);
            child(parent, "BUTTON", "保存", button_style(), 600, 90, 90, 26, ID_SAVE, font);
            let intent_edit = child(
                parent,
                "EDIT",
                "清机不联网，不读剪贴板，不加载插件。",
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_READONLY as u32,
                16,
                128,
                710,
                300,
                ID_INTENT,
                font,
            );
            child(
                parent,
                "STATIC",
                "按自己的记录输入：链编号/数量/实际封顶/收款地址。代币再在封顶后加上合约地址。不要照着本窗口抄。",
                WS_CHILD | WS_VISIBLE | 0x0000_2000,
                16,
                434,
                710,
                36,
                0,
                font,
            );
            let confirm_edit = child(parent, "EDIT", "", edit_style(), 16, 474, 710, 26, ID_CONFIRM, font);
            child(parent, "BUTTON", "读取简牍", button_style(), 16, 508, 120, 26, ID_READ, font);
            let sign_btn = child(parent, "BUTTON", "签名", button_style(), 146, 508, 120, 26, ID_SIGN, font);
            EnableWindow(sign_btn, 0);
            SendMessageW(confirm_edit, 0x00C5, 320, 0);
            let status_edit = child(
                parent,
                "EDIT",
                "请用另一个 Windows 账户运行清机，并禁止这个程序出网。",
                WS_CHILD | WS_VISIBLE | WS_BORDER | ES_AUTOHSCROLL as u32 | ES_READONLY as u32,
                16,
                546,
                710,
                26,
                ID_STATUS,
                font,
            );
            INTENT_HWND.store(intent_edit as isize, Ordering::Relaxed);
            CONFIRM_HWND.store(confirm_edit as isize, Ordering::Relaxed);
            PASS_HWND.store(pass_edit as isize, Ordering::Relaxed);
            PASS2_HWND.store(pass2_edit as isize, Ordering::Relaxed);
            subclass(intent_edit);
            subclass(confirm_edit);
            subclass(pass_edit);
            subclass(pass2_edit);
            let profile = profile_dir(".konghu");
            Self {
                vault_path: profile.join("vault.khuv"),
                label_path: profile.join("labels.txt"),
                profile,
                labels: Vec::new(),
                pass_edit,
                pass2_edit,
                drop_edit,
                confirm_edit,
                intent_edit,
                status_edit,
                sign_btn,
                secret: None,
                tablet: None,
            }
        }
    }

    fn load_local(&mut self) {
        set_text(self.drop_edit, &read_line(&self.profile.join("drop.path")));
        self.labels = load_labels(&self.label_path);
        let status = if self.vault_path.is_file() {
            "保险库已存在。输入口令后解锁。"
        } else {
            "还没有保险库。输入两遍口令后创建。口令不是助记词的第 25 个词。"
        };
        set_text(self.status_edit, status);
    }

    fn create_vault(&mut self) -> Ui {
        let pass = read_secret(self.pass_edit);
        let again = read_secret(self.pass2_edit);
        if let Err(err) = check_password(&pass) {
            return ui_err(err);
        }
        if pass.as_str() != again.as_str() {
            return ui_err("两次口令不一致");
        }
        if self.vault_path.exists() {
            return ui_err("保险库已存在，清机不会覆盖它");
        }
        let entropy = match qing_core::random_entropy() {
            Ok(entropy) => Zeroizing::new(entropy),
            Err(err) => return ui_err(err.to_string()),
        };
        let words = match qing_core::mnemonic_from_entropy(&entropy) {
            Ok(words) => words,
            Err(err) => return ui_err(err.to_string()),
        };
        let sealed = match qing_core::seal(pass.as_bytes(), &entropy) {
            Ok(sealed) => sealed,
            Err(err) => return ui_err(err.to_string()),
        };
        if write_atomic(&self.vault_path, &sealed).is_err() {
            return ui_err("无法写入保险库");
        }
        match qing_core::secret_from_entropy(&entropy) {
            Ok(secret) => self.secret = Some(Locked32::new(*secret)),
            Err(err) => return ui_err(err.to_string()),
        }
        let mut intent = String::from("这是唯一一次显示助记词。抄完后点锁定。\r\n");
        intent.push_str(&words);
        Ui {
            status: "保险库已创建。助记词只在上面，不在剪贴板。".into(),
            intent: Some(intent),
            sign_enabled: false,
            clear_passwords: true,
        }
    }

    fn unlock(&mut self) -> Ui {
        let pass = read_secret(self.pass_edit);
        if let Err(err) = check_password(&pass) {
            return ui_err(err);
        }
        let bytes = match read_exact(&self.vault_path, qing_core::VAULT_LEN as u64) {
            Ok(bytes) => bytes,
            Err(err) => return ui_err(err),
        };
        let entropy = match qing_core::open(&bytes, pass.as_bytes()) {
            Ok(entropy) => entropy,
            Err(err) => return ui_err(err.to_string()),
        };
        let secret = match qing_core::secret_from_entropy(&entropy) {
            Ok(secret) => secret,
            Err(err) => return ui_err(err.to_string()),
        };
        let address = match qing_core::address_from_secret(&secret) {
            Ok(address) => address,
            Err(err) => return ui_err(err.to_string()),
        };
        self.secret = Some(Locked32::new(*secret));
        let mut intent = format!("付款地址：{}\r\n", qing_core::eip55(&address));
        if let Some(tablet) = self.tablet {
            match self.describe(tablet) {
                Ok(text) => intent = text,
                Err(err) => {
                    self.tablet = None;
                    return Ui {
                        status: err,
                        intent: Some(intent),
                        sign_enabled: false,
                        clear_passwords: true,
                    };
                }
            }
        }
        Ui {
            status: "已解锁。种子只留到这一笔签完。".into(),
            intent: Some(intent),
            sign_enabled: self.confirm_ok(),
            clear_passwords: true,
        }
    }

    fn lock(&mut self, status: &str) -> Ui {
        self.secret = None;
        self.tablet = None;
        Ui {
            status: status.into(),
            intent: Some(String::new()),
            sign_enabled: false,
            clear_passwords: true,
        }
    }

    fn save_drop(&mut self) -> Ui {
        let drop = get_text(self.drop_edit);
        if !drop.trim().is_empty() && !Path::new(drop.trim()).is_dir() {
            return note(self, "投递目录不存在");
        }
        if write_text(&self.profile.join("drop.path"), drop.trim()).is_err() {
            return note(self, "无法保存投递目录");
        }
        Ui {
            status: "已保存投递目录".into(),
            intent: None,
            sign_enabled: self.confirm_ok(),
            clear_passwords: false,
        }
    }

    fn read_tablet(&mut self) -> Ui {
        let drop = PathBuf::from(get_text(self.drop_edit).trim());
        let inbox = match drop_file(&drop, "inbox.khub") {
            Ok(path) => path,
            Err(err) => return note(self, err),
        };
        let bytes = match read_exact(&inbox, tablet::TABLET_LEN as u64) {
            Ok(bytes) => bytes,
            Err(err) => return note(self, err),
        };
        let tablet = match tablet::Tablet::decode(&bytes) {
            Ok(tablet) => tablet,
            Err(err) => return note(self, err.to_string()),
        };
        let text = match self.describe(tablet) {
            Ok(text) => text,
            Err(err) => return note(self, err),
        };
        self.tablet = Some(tablet);
        Ui {
            status: "已读取简牍。请按自己的记录输入确认串，不要照着本窗口抄。".into(),
            intent: Some(text),
            sign_enabled: self.confirm_ok(),
            clear_passwords: false,
        }
    }

    fn sign(&mut self) -> Ui {
        let Some(tablet) = self.tablet else {
            return note(self, "还没有简牍");
        };
        if !self.confirm_ok() {
            return note(self, "确认串和这张简牍不一致");
        }
        let Some(secret) = self.secret.as_ref() else {
            return note(self, "尚未解锁");
        };
        let now = match qing_core::now_unix() {
            Ok(now) => now,
            Err(err) => return note(self, err.to_string()),
        };
        let receipt = match qing_core::sign_tablet(&tablet, secret.bytes(), now) {
            Ok(receipt) => receipt,
            Err(err) => return note(self, err.to_string()),
        };
        let drop = PathBuf::from(get_text(self.drop_edit).trim());
        let outbox = match drop_file(&drop, "outbox.khur") {
            Ok(path) => path,
            Err(err) => return note(self, err),
        };
        if write_atomic(&outbox, &receipt.encode()).is_err() {
            return note(self, "无法写出回执");
        }
        if let Ok(inbox) = drop_file(&drop, "inbox.khub") {
            if read_exact(&inbox, tablet::TABLET_LEN as u64).ok().as_deref() == Some(tablet.encode().as_slice())
            {
                let _ = std::fs::remove_file(inbox);
            }
        }
        self.secret = None;
        self.tablet = None;
        Ui {
            status: "已签名并写出回执。种子已从内存清掉。".into(),
            intent: None,
            sign_enabled: false,
            clear_passwords: true,
        }
    }

    fn confirm_ok(&self) -> bool {
        let Some(tablet) = self.tablet else {
            return false;
        };
        if self.secret.is_none() {
            return false;
        }
        qing_core::confirmation_matches(&tablet, &read_secret(self.confirm_edit))
    }

    fn describe(&self, tablet: tablet::Tablet) -> Result<String, String> {
        let now = qing_core::now_unix().map_err(|err| err.to_string())?;
        let from = self
            .secret
            .as_ref()
            .map(|secret| qing_core::address_from_secret(secret.bytes()))
            .transpose()
            .map_err(|err| err.to_string())?;
        let labels = &self.labels;
        qing_core::inspect(&tablet, now, from.as_ref(), &|addr| {
            labels
                .iter()
                .find(|(key, _)| key == addr)
                .map(|(_, note)| note.clone())
        })
        .map_err(|err| err.to_string())
    }
}

fn note(app: &App, status: impl Into<String>) -> Ui {
    Ui {
        status: status.into(),
        intent: None,
        sign_enabled: app.confirm_ok(),
        clear_passwords: false,
    }
}

fn ui_err(status: impl Into<String>) -> Ui {
    Ui {
        status: status.into(),
        intent: None,
        sign_enabled: false,
        clear_passwords: true,
    }
}

fn apply(view: View) {
    if let Some(intent) = &view.ui.intent {
        set_text(view.intent, intent);
    }
    set_text(view.status, &view.ui.status);
    unsafe { EnableWindow(view.sign, if view.ui.sign_enabled { 1 } else { 0 }); }
    if view.ui.clear_passwords {
        set_text(view.pass, "");
        set_text(view.pass2, "");
    }
}

fn check_password(password: &str) -> Result<(), String> {
    let len = password.chars().count();
    if (8..=128).contains(&len) {
        Ok(())
    } else {
        Err("口令需要 8 到 128 个字符".into())
    }
}

fn load_labels(path: &Path) -> Vec<([u8; 20], String)> {
    let mut labels = Vec::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return labels;
    };
    for line in text.lines() {
        let Some((addr, note)) = line.split_once(|ch: char| ch.is_whitespace()) else {
            continue;
        };
        let Some(note) = qing_core::accept_note(note) else {
            continue;
        };
        if let Some(addr) = parse_hex_20(addr) {
            labels.push((addr, note.to_string()));
        }
    }
    labels
}

fn parse_hex_20(input: &str) -> Option<[u8; 20]> {
    let digits = input.trim().trim_start_matches("0x").trim_start_matches("0X");
    if digits.len() != 40 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn profile_dir(name: &str) -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = home.join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn read_line(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default().trim().to_string()
}

fn write_text(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|err| err.to_string())
}

fn drop_file(drop: &Path, name: &str) -> Result<PathBuf, String> {
    if name != "inbox.khub" && name != "outbox.khur" {
        return Err("投递目录只允许简牍和回执".into());
    }
    if !drop.is_dir() {
        return Err("投递目录不存在".into());
    }
    let drop = drop.canonicalize().map_err(|err| err.to_string())?;
    let path = drop.join(name);
    if path.exists() {
        let canon = path.canonicalize().map_err(|err| err.to_string())?;
        if canon.parent() != Some(drop.as_path()) || canon.file_name() != Some(OsStr::new(name)) {
            return Err("投递文件指向了目录之外".into());
        }
        return Ok(canon);
    }
    Ok(path)
}

fn read_exact(path: &Path, len: u64) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|_| "找不到文件".to_string())?;
    if !meta.is_file() || meta.len() != len {
        return Err("文件长度不对".into());
    }
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    if bytes.len() as u64 != len {
        return Err("文件长度不对".into());
    }
    Ok(bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    dropio::write_atomic(path, bytes)
}

fn edit_style() -> u32 {
    WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32
}

fn button_style() -> u32 {
    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32
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

fn subclass(hwnd: HWND) {
    unsafe {
        let previous = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, edit_proc as *const () as isize);
        if ORIG_EDIT.load(Ordering::Relaxed) == 0 {
            ORIG_EDIT.store(previous, Ordering::Relaxed);
        }
    }
}

unsafe extern "system" fn edit_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let intent = INTENT_HWND.load(Ordering::Relaxed) as HWND;
    let confirm = CONFIRM_HWND.load(Ordering::Relaxed) as HWND;
    let pass = PASS_HWND.load(Ordering::Relaxed) as HWND;
    let pass2 = PASS2_HWND.load(Ordering::Relaxed) as HWND;
    let sensitive = hwnd == intent || hwnd == pass || hwnd == pass2;
    if sensitive && (msg == WM_COPY || msg == WM_CUT || msg == WM_CONTEXTMENU) {
        return 0;
    }
    if hwnd == confirm && msg == WM_PASTE {
        return 0;
    }
    if msg == WM_KEYDOWN {
        let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
        let key = (wparam as u8).to_ascii_uppercase();
        if ctrl && sensitive && (key == b'C' || key == b'X' || key == b'V') {
            return 0;
        }
        if ctrl && hwnd == confirm && key == b'V' {
            return 0;
        }
    }
    let orig = ORIG_EDIT.load(Ordering::Relaxed);
    if orig == 0 {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    type Proc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;
    let proc: Proc = std::mem::transmute(orig);
    CallWindowProcW(Some(proc), hwnd, msg, wparam, lparam)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn get_text(hwnd: HWND) -> String {
    read_secret(hwnd).to_string()
}

fn read_secret(hwnd: HWND) -> Zeroizing<String> {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return Zeroizing::new(String::new());
        }
        let mut buf = vec![0u16; len as usize + 1];
        GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        let end = buf.iter().position(|unit| *unit == 0).unwrap_or(buf.len());
        let text = Zeroizing::new(String::from_utf16_lossy(&buf[..end]));
        for unit in &mut buf {
            *unit = 0;
        }
        text
    }
}

fn set_text(hwnd: HWND, text: &str) {
    let mut buf = wide(text);
    unsafe { SetWindowTextW(hwnd, buf.as_ptr()); }
    buf.zeroize();
}

fn alert(text: &str) {
    let text = wide(text);
    let title = wide("清机");
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONWARNING,
        );
    }
}
