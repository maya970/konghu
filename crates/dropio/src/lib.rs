//! 把字节写到调用者给出的路径。已有的符号链接只删除链接本身，不写入它指向的文件。

use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, DeleteFileW, GetFileAttributesW, GetFileInformationByHandle, GetFinalPathNameByHandleW,
    MoveFileExW, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_READ, INVALID_FILE_ATTRIBUTES, MOVEFILE_WRITE_THROUGH, OPEN_EXISTING, VOLUME_NAME_DOS,
};

const GENERIC_READ: u32 = 0x8000_0000;

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = plain_name(path)?;
    let parent = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or("没有目录")?
        .canonicalize()
        .map_err(|err| err.to_string())?;
    if !parent.is_dir() {
        return Err("目录不存在".into());
    }
    let dest = parent.join(name);
    let tmp = parent.join(random_tmp_name()?);
    write_new(&tmp, bytes)?;
    match install(&tmp, &dest) {
        Ok(()) => verify(&dest, &parent, name),
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

fn plain_name(path: &Path) -> Result<&str, String> {
    let name = path.file_name().and_then(|s| s.to_str()).ok_or("文件名无效")?;
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) {
        return Err("文件名无效".into());
    }
    Ok(name)
}

fn random_tmp_name() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|_| "无法读取系统随机数")?;
    let mut name = String::from(".");
    for byte in bytes {
        name.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        name.push(char::from(b"0123456789abcdef"[(byte & 0x0f) as usize]));
    }
    name.push_str(".tmp");
    Ok(name)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| err.to_string())?;
    file.write_all(bytes).map_err(|err| err.to_string())?;
    file.sync_all().map_err(|err| err.to_string())?;
    Ok(())
}

fn install(tmp: &Path, dest: &Path) -> Result<(), String> {
    for _ in 0..5 {
        match attributes(dest)? {
            Some(_) => delete_dir_entry(dest)?,
            None => {
                if move_new(tmp, dest)? {
                    return Ok(());
                }
            }
        }
    }
    Err("无法把文件放到目标名字上".into())
}

fn move_new(tmp: &Path, dest: &Path) -> Result<bool, String> {
    let from = wide(tmp);
    let to = wide(dest);
    let ok = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if ok != 0 {
        return Ok(true);
    }
    let err = unsafe { GetLastError() };
    if err == ERROR_ALREADY_EXISTS || err == ERROR_FILE_EXISTS {
        return Ok(false);
    }
    Err("无法替换目标文件".into())
}

fn verify(dest: &Path, parent: &Path, name: &str) -> Result<(), String> {
    if attributes(dest)?.unwrap_or(0) & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        let _ = delete_dir_entry(dest);
        return Err("目标仍是符号链接".into());
    }
    let final_path = final_path(dest)?;
    let expected = parent.join(name);
    if normalize(&final_path) != normalize(&expected) {
        let _ = delete_dir_entry(dest);
        return Err("目标文件指向了目录之外".into());
    }
    Ok(())
}

fn attributes(path: &Path) -> Result<Option<u32>, String> {
    let wide = wide(path);
    let attr = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if attr == INVALID_FILE_ATTRIBUTES {
        let err = unsafe { GetLastError() };
        if err == ERROR_FILE_NOT_FOUND || err == ERROR_PATH_NOT_FOUND {
            return Ok(None);
        }
        return Err("无法查看文件属性".into());
    }
    Ok(Some(attr))
}

fn delete_dir_entry(path: &Path) -> Result<(), String> {
    let wide = wide(path);
    let ok = unsafe { DeleteFileW(wide.as_ptr()) };
    if ok == 0 {
        return Err("无法去掉已有文件".into());
    }
    Ok(())
}

fn final_path(path: &Path) -> Result<PathBuf, String> {
    let wide_name = wide(path);
    let handle = unsafe {
        CreateFileW(
            wide_name.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err("无法打开刚写出的文件".into());
    }
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let info_ok = unsafe { GetFileInformationByHandle(handle, &mut info) };
    let mut buf = [0u16; 1024];
    let n = unsafe { GetFinalPathNameByHandleW(handle, buf.as_mut_ptr(), buf.len() as u32, VOLUME_NAME_DOS) };
    unsafe { CloseHandle(handle) };
    if info_ok == 0 || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("目标仍是符号链接".into());
    }
    if n == 0 || n as usize >= buf.len() {
        return Err("无法核对文件路径".into());
    }
    Ok(PathBuf::from(String::from_utf16_lossy(&buf[..n as usize])))
}

fn normalize(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let stripped = raw
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| raw.strip_prefix(r"\\?\").map(|rest| rest.to_string()))
        .unwrap_or_else(|| raw.to_string());
    stripped.replace('/', "\\").to_ascii_lowercase()
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::fs::symlink_file;

    #[test]
    fn symlink_target_stays_intact() {
        let root = std::env::temp_dir().join(format!("konghu-dropio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let drop_dir = root.join("drop");
        std::fs::create_dir_all(&drop_dir).unwrap();
        let vault = root.join("vault.khuv");
        std::fs::write(&vault, b"VAULT-SECRET").unwrap();
        let planted = drop_dir.join(".outbox.khur.tmp");
        let dest = drop_dir.join("outbox.khur");
        symlink_file(&vault, &planted).unwrap();
        symlink_file(&vault, &dest).unwrap();

        let wrote = write_atomic(&dest, b"RECEIPT");
        let vault_bytes = std::fs::read(&vault).unwrap();
        assert_eq!(vault_bytes, b"VAULT-SECRET");
        assert!(wrote.is_ok(), "{wrote:?}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"RECEIPT");
        assert_eq!(attributes(&dest).unwrap().unwrap() & FILE_ATTRIBUTE_REPARSE_POINT, 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn hard_link_and_junction_leave_vault_bytes() {
        let root = std::env::temp_dir().join(format!("konghu-dropio-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profile = root.join("profile");
        let drop_dir = root.join("drop");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(&drop_dir).unwrap();
        let vault = profile.join("vault.khuv");
        std::fs::write(&vault, b"VAULT-SECRET").unwrap();

        let hard = drop_dir.join("outbox.khur");
        std::fs::hard_link(&vault, &hard).unwrap();
        let wrote = write_atomic(&hard, b"RECEIPT");
        assert_eq!(std::fs::read(&vault).unwrap(), b"VAULT-SECRET");
        assert!(wrote.is_ok(), "{wrote:?}");
        assert_eq!(std::fs::read(&hard).unwrap(), b"RECEIPT");

        let junction = root.join("junction");
        std::os::windows::fs::symlink_dir(&profile, &junction).unwrap();
        let via = junction.join("outbox.khur");
        write_atomic(&via, b"JUNCTION").unwrap();
        assert_eq!(std::fs::read(&vault).unwrap(), b"VAULT-SECRET");
        assert_eq!(std::fs::read(profile.join("outbox.khur")).unwrap(), b"JUNCTION");

        let ads = drop_dir.join("outbox.khur:hid");
        assert!(write_atomic(&ads, b"STREAM").is_err());
        assert_eq!(std::fs::read(&vault).unwrap(), b"VAULT-SECRET");

        let dotted = drop_dir.join("dotted.khur.");
        let dotted_result = write_atomic(&dotted, b"DOT");
        assert_eq!(std::fs::read(&vault).unwrap(), b"VAULT-SECRET");
        assert_eq!(std::fs::read(&hard).unwrap(), b"RECEIPT");
        assert_eq!(std::fs::read(profile.join("outbox.khur")).unwrap(), b"JUNCTION");
        if let Ok(()) = dotted_result {
            let extended = std::path::PathBuf::from(format!(r"\\?\{}", drop_dir.display()));
            let landed = std::fs::read_dir(&extended)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .find(|entry| std::fs::read(entry.path()).ok().as_deref() == Some(b"DOT"));
            let landed = landed.unwrap_or_else(|| panic!("尾随点写入没有落在投递目录"));
            let parent = landed.path().parent().unwrap().to_string_lossy().to_ascii_lowercase();
            assert!(parent.contains("\\drop"));
            assert!(!parent.contains("\\profile"));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn replaces_regular_file() {
        let root = std::env::temp_dir().join(format!("konghu-dropio-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let dest = root.join("pending.khub");
        std::fs::write(&dest, b"old").unwrap();
        write_atomic(&dest, b"new").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&root);
    }
}
