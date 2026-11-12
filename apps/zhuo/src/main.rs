//! 浊机。默认视为不可信：只写简牍、只广播回执，碰不到种子。

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod fees;
mod registry;
mod rpc;

use crate::fees::propose_fees;
use crate::registry::{load_registry, Chain};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tablet::{Receipt, Tablet, TABLET_LEN, ASSET_ERC20, ASSET_NATIVE, FAMILY_EVM, MAX_RAW_TX};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateFontW, COLOR_WINDOW, DEFAULT_CHARSET, FW_NORMAL};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const ID_DROP: i32 = 1001;
const ID_FROM: i32 = 1002;
const ID_LIST: i32 = 1003;
const ID_RPC: i32 = 1004;
const ID_TO: i32 = 1005;
const ID_AMOUNT: i32 = 1006;
const ID_DECIMALS: i32 = 1007;
const ID_TOKEN: i32 = 1008;
const ID_STATUS: i32 = 1009;
const ID_SAVE: i32 = 1101;
const ID_TOKEN_CHECK: i32 = 1102;
const ID_WRITE: i32 = 1103;
const ID_BROADCAST: i32 = 1104;

struct App {
    chains: Vec<Chain>,
    profile: PathBuf,
    drop_edit: HWND,
    from_edit: HWND,
    list: HWND,
    rpc_edit: HWND,
    to_edit: HWND,
    amount_edit: HWND,
    decimals_edit: HWND,
    token_edit: HWND,
    token_check: HWND,
    status_edit: HWND,
    pending: Option<Vec<u8>>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn main() {
    let chains = match load_registry() {
        Ok(chains) => chains,
        Err(err) => {
            alert(&err);
            return;
        }
    };
    let profile = profile_dir(".konghu-zhuo");
    let class = wide("KonghuZhuo");
    let title = wide("浊机 — 空毂");
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
            820,
            760,
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
                app.chains = chains;
                app.profile = profile;
                app.fill_list();
                app.load_settings();
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
            let app = App::new(hwnd);
            APP.with(|slot| *slot.borrow_mut() = Some(app));
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as i32;
            let code = ((wparam >> 16) & 0xffff) as u16;
            if code == EN_CHANGE as u16 {
                return 0;
            }
            if id == ID_LIST && code == LBN_SELCHANGE as u16 {
                let update = APP.with(|slot| {
                    let Ok(borrow) = slot.try_borrow() else {
                        return None;
                    };
                    borrow.as_ref().and_then(|app| {
                        app.selected().map(|chain| (app.rpc_edit, chain.rpc.clone()))
                    })
                });
                if let Some((edit, rpc)) = update {
                    set_text(edit, &rpc);
                }
                return 0;
            }
            if code != BN_CLICKED as u16 {
                return 0;
            }
            let outcome = APP.with(|slot| {
                let mut borrow = slot.borrow_mut();
                let Some(app) = borrow.as_mut() else {
                    return Err("窗口尚未就绪".into());
                };
                match id {
                    ID_SAVE => app.save_settings(),
                    ID_WRITE => app.write_tablet(),
                    ID_BROADCAST => app.broadcast(),
                    _ => Ok(String::new()),
                }
            });
            if let Err(err) = outcome {
                APP.with(|slot| {
                    if let Some(app) = slot.borrow().as_ref() {
                        set_text(app.status_edit, &err);
                    }
                });
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

impl App {
    fn new(hwnd: HWND) -> Self {
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
            let label = |text, x, y, w| styled(hwnd, "STATIC", text, WS_CHILD | WS_VISIBLE, x, y, w, 24, 0, font);
            label("投递目录", 16, 16, 90);
            let drop_edit = styled(hwnd, "EDIT", "", edit_style(), 110, 14, 560, 26, ID_DROP, font);
            styled(hwnd, "BUTTON", "保存设置", button_style(), 680, 14, 100, 26, ID_SAVE, font);
            label("付款地址", 16, 50, 90);
            let from_edit = styled(hwnd, "EDIT", "", edit_style(), 110, 48, 670, 26, ID_FROM, font);
            label("链", 16, 86, 90);
            let list = styled(
                hwnd,
                "LISTBOX",
                "",
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_VSCROLL | WS_TABSTOP | LBS_NOTIFY as u32,
                110,
                86,
                300,
                220,
                ID_LIST,
                font,
            );
            label("节点", 430, 86, 50);
            let rpc_edit = styled(
                hwnd,
                "EDIT",
                "",
                edit_style() | ES_READONLY as u32,
                430,
                112,
                350,
                26,
                ID_RPC,
                font,
            );
            label("收款地址", 16, 320, 90);
            let to_edit = styled(hwnd, "EDIT", "", edit_style(), 110, 318, 670, 26, ID_TO, font);
            label("数量", 16, 356, 90);
            let amount_edit = styled(hwnd, "EDIT", "", edit_style(), 110, 354, 300, 26, ID_AMOUNT, font);
            label("代币小数位", 430, 356, 100);
            let decimals_edit = styled(hwnd, "EDIT", "18", edit_style(), 540, 354, 80, 26, ID_DECIMALS, font);
            let token_check = styled(
                hwnd,
                "BUTTON",
                "这是代币转账（只做 transfer，不做授权）",
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                110,
                392,
                420,
                24,
                ID_TOKEN_CHECK,
                font,
            );
            label("代币合约", 16, 424, 90);
            let token_edit = styled(hwnd, "EDIT", "", edit_style(), 110, 422, 670, 26, ID_TOKEN, font);
            styled(hwnd, "BUTTON", "写入简牍", button_style(), 110, 460, 140, 30, ID_WRITE, font);
            styled(hwnd, "BUTTON", "广播回执", button_style(), 260, 460, 140, 30, ID_BROADCAST, font);
            let status_edit = styled(
                hwnd,
                "EDIT",
                "浊机不保存种子。清机上核对链编号、全长地址和最小单位。",
                WS_CHILD | WS_VISIBLE | WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_READONLY as u32,
                16,
                504,
                770,
                200,
                ID_STATUS,
                font,
            );
            Self {
                chains: Vec::new(),
                profile: PathBuf::new(),
                drop_edit,
                from_edit,
                list,
                rpc_edit,
                to_edit,
                amount_edit,
                decimals_edit,
                token_edit,
                token_check,
                status_edit,
                pending: None,
            }
        }
    }

    fn fill_list(&self) {
        for chain in &self.chains {
            let line = wide(&format!("{:>12}  {}", chain.chain_id, chain.symbol));
            unsafe {
                SendMessageW(self.list, LB_ADDSTRING, 0, line.as_ptr() as LPARAM);
            }
        }
        if !self.chains.is_empty() {
            unsafe { SendMessageW(self.list, LB_SETCURSEL, 0, 0); }
            self.show_rpc();
        }
    }

    fn show_rpc(&self) {
        if let Some(chain) = self.selected() {
            set_text(self.rpc_edit, &chain.rpc);
        }
    }

    fn selected(&self) -> Option<&Chain> {
        let index = unsafe { SendMessageW(self.list, LB_GETCURSEL, 0, 0) };
        if index < 0 {
            None
        } else {
            self.chains.get(index as usize)
        }
    }

    fn load_settings(&mut self) {
        set_text(self.drop_edit, &read_line(&self.profile.join("drop.path")));
        set_text(self.from_edit, &read_line(&self.profile.join("from.addr")));
        let pending_path = self.profile.join("pending.khub");
        if let Ok(bytes) = read_limited(&pending_path, TABLET_LEN as u64) {
            if Tablet::decode(&bytes).is_ok() {
                self.pending = Some(bytes);
            }
        }
    }

    fn save_settings(&mut self) -> Result<String, String> {
        let drop = get_text(self.drop_edit);
        let from = get_text(self.from_edit);
        if !drop.trim().is_empty() && !Path::new(drop.trim()).is_dir() {
            return Err("投递目录不存在".into());
        }
        if !from.trim().is_empty() {
            rpc::parse_address(&from)?;
        }
        write_text(&self.profile.join("drop.path"), drop.trim())?;
        write_text(&self.profile.join("from.addr"), from.trim())?;
        let message = "已保存投递目录和付款地址".to_string();
        set_text(self.status_edit, &message);
        Ok(message)
    }

    fn write_tablet(&mut self) -> Result<String, String> {
        self.save_settings()?;
        let chain = self.selected().cloned().ok_or("先选择一条链")?;
        let from = rpc::parse_address(&get_text(self.from_edit))?;
        let recipient = rpc::parse_address(&get_text(self.to_edit))?;
        let token_mode = unsafe { SendMessageW(self.token_check, BM_GETCHECK, 0, 0) } == 1;
        let (kind, token, decimals, gas_limit) = if token_mode {
            let token = rpc::parse_address(&get_text(self.token_edit))?;
            let decimals = get_text(self.decimals_edit);
            let decimals: u8 = decimals.trim().parse().map_err(|_| "代币小数位不合法")?;
            (ASSET_ERC20, token, decimals, 120_000u64)
        } else {
            (ASSET_NATIVE, [0u8; 20], chain.decimals, 21_000u64)
        };
        let amount = tablet::parse_decimal_amount(&get_text(self.amount_edit), decimals)
            .map_err(|err| err.to_string())?;
        let remote = rpc::chain_id(&chain.rpc)?;
        if remote != chain.chain_id {
            return Err(format!("节点报告链编号 {remote}，登记表是 {}", chain.chain_id));
        }
        let nonce = rpc::nonce(&chain.rpc, &rpc::hex_encode(&from))?;
        let base = rpc::base_fee(&chain.rpc)?;
        let gas_price = if base.is_none() {
            Some(rpc::gas_price(&chain.rpc)?)
        } else {
            None
        };
        let fees = propose_fees(base, gas_price, rpc::priority_fee(&chain.rpc), gas_limit)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "系统时间不可用")?
            .as_secs();
        let tablet = Tablet::new(
            FAMILY_EVM,
            chain.chain_id,
            kind,
            token,
            recipient,
            amount,
            fees.fee_cap,
            now + 15 * 60,
            nonce,
            gas_limit,
            fees.max_fee,
            fees.priority,
        )
        .map_err(|err| err.to_string())?;
        let bytes = tablet.encode();
        let drop = PathBuf::from(get_text(self.drop_edit).trim());
        let inbox = drop_file(&drop, "inbox.khub")?;
        write_atomic(&inbox, &bytes)?;
        write_atomic(&self.profile.join("pending.khub"), &bytes)?;
        if let Ok(outbox) = drop_file(&drop, "outbox.khur") {
            let _ = std::fs::remove_file(outbox);
        }
        self.pending = Some(bytes.to_vec());
        let message = format!(
            "已写入简牍。链编号 {}，数量（最小单位）{}，费用上限（最小单位）{}。请到清机核对全长地址。",
            chain.chain_id,
            amount.to_dec_string(),
            fees.fee_cap.to_dec_string()
        );
        set_text(self.status_edit, &message);
        Ok(message)
    }

    fn broadcast(&mut self) -> Result<String, String> {
        let pending = self
            .pending
            .clone()
            .ok_or("没有待广播的简牍")?;
        let drop = PathBuf::from(get_text(self.drop_edit).trim());
        let outbox = drop_file(&drop, "outbox.khur")?;
        let bytes = read_limited(&outbox, (4 + 1 + TABLET_LEN + 2 + MAX_RAW_TX) as u64)?;
        let receipt = Receipt::decode(&bytes).map_err(|err| err.to_string())?;
        if receipt.tablet().encode().as_slice() != pending.as_slice() {
            return Err("回执不是刚刚写出的那张简牍".into());
        }
        konghu_evm::raw_matches_tablet(receipt.raw_tx(), &receipt.tablet()).map_err(|err| err.to_string())?;
        let chain = self
            .chains
            .iter()
            .find(|chain| chain.chain_id == receipt.tablet().chain_id())
            .ok_or("登记表里没有这条链")?;
        let hash = rpc::send_raw(&chain.rpc, receipt.raw_tx())?;
        let _ = std::fs::remove_file(&outbox);
        let message = format!("已广播。交易哈希：{hash}");
        set_text(self.status_edit, &message);
        Ok(message)
    }
}

fn edit_style() -> u32 {
    WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32
}

fn button_style() -> u32 {
    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32
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
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
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
        if canon.parent() != Some(drop.as_path()) || canon.file_name() != Some(std::ffi::OsStr::new(name))
        {
            return Err("投递文件指向了目录之外".into());
        }
        return Ok(canon);
    }
    Ok(path)
}

fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|_| "找不到投递文件".to_string())?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > max {
        return Err("投递物长度不对".into());
    }
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    if bytes.len() as u64 != meta.len() {
        return Err("投递物长度不对".into());
    }
    Ok(bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    dropio::write_atomic(path, bytes)
}

unsafe fn styled(
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

fn get_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }
}

fn set_text(hwnd: HWND, text: &str) {
    let buf = wide(text);
    unsafe { SetWindowTextW(hwnd, buf.as_ptr()); }
}

fn alert(text: &str) {
    let text = wide(text);
    let title = wide("浊机");
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING); }
}
