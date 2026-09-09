use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureProcess {
    pub name: String,
    pub exe: String,
    pub aumid: String,
}

pub fn processes_json() -> String {
    let items: Vec<serde_json::Value> = list_processes()
        .into_iter()
        .map(|process| {
            serde_json::json!({
                "name": process.name,
                "exe": process.exe,
                "aumid": process.aumid,
            })
        })
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".into())
}

pub fn list_processes() -> Vec<CaptureProcess> {
    #[cfg(windows)]
    {
        windows_list()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

pub fn exe_basename(path: &str) -> String {
    path.replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim()
        .to_ascii_lowercase()
}

pub fn exe_matches(stored: &str, process_exe: &str) -> bool {
    let want = exe_basename(stored);
    let have = exe_basename(process_exe);
    if want.is_empty() || have.is_empty() {
        return false;
    }
    want == have || format!("{want}.exe") == have || format!("{have}.exe") == want
}

pub fn pick_tree_root(matches: &[(u32, u32)]) -> Option<u32> {
    if matches.is_empty() {
        return None;
    }
    let pids: HashSet<u32> = matches.iter().map(|(pid, _)| *pid).collect();
    matches
        .iter()
        .filter(|(_, parent)| !pids.contains(parent))
        .map(|(pid, _)| *pid)
        .min()
        .or_else(|| matches.iter().map(|(pid, _)| *pid).min())
}

#[cfg(windows)]
pub fn resolve_pid(exe: &str, aumid: &str) -> Option<u32> {
    let want_aumid = aumid.trim();
    let want_exe = exe.trim();
    if want_aumid.is_empty() && want_exe.is_empty() {
        return None;
    }
    let self_pid = unsafe { windows::Win32::System::Threading::GetCurrentProcessId() };
    let mut matched = Vec::new();
    for process in windows_snapshot() {
        if process.pid == 0 || process.pid == self_pid {
            continue;
        }
        let aumid_ok = !want_aumid.is_empty() && process.aumid.eq_ignore_ascii_case(want_aumid);
        let exe_ok = !want_exe.is_empty() && exe_matches(want_exe, &process.exe);
        if !want_aumid.is_empty() {
            if aumid_ok {
                matched.push((process.pid, process.parent));
            }
        } else if exe_ok {
            matched.push((process.pid, process.parent));
        }
    }
    pick_tree_root(&matched)
}

#[cfg(windows)]
pub fn process_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(handle);
        ok
    }
}

#[cfg(windows)]
struct SnapshotProcess {
    pid: u32,
    parent: u32,
    exe: String,
    aumid: String,
}

#[cfg(windows)]
fn windows_list() -> Vec<CaptureProcess> {
    use std::collections::HashMap;
    let self_pid = unsafe { windows::Win32::System::Threading::GetCurrentProcessId() };
    let titles = gui_window_titles();
    let mut by_key: HashMap<(String, String), CaptureProcess> = HashMap::new();
    for process in windows_snapshot() {
        if process.pid == 0 || process.pid == self_pid || skip_system_exe(&process.exe) {
            continue;
        }
        let Some(title) = titles.get(&process.pid) else {
            continue;
        };
        let exe = if process.exe.is_empty() {
            continue;
        } else {
            process.exe
        };
        let aumid = process.aumid;
        let name = if title.trim().is_empty() {
            exe.clone()
        } else {
            title.clone()
        };
        let key = (exe_basename(&exe), aumid.to_ascii_lowercase());
        by_key
            .entry(key)
            .and_modify(|existing| {
                if existing.name.len() < name.len() {
                    existing.name = name.clone();
                }
            })
            .or_insert(CaptureProcess { name, exe, aumid });
    }
    let mut out: Vec<CaptureProcess> = by_key.into_values().collect();
    out.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    out
}

#[cfg(windows)]
fn gui_window_titles() -> std::collections::HashMap<u32, String> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindow, GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_TOOLWINDOW,
    };

    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let titles = unsafe { &mut *(lparam.0 as *mut std::collections::HashMap<u32, String>) };
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            if GetWindow(hwnd, GW_OWNER)
                .ok()
                .is_some_and(|owner| !owner.0.is_null())
            {
                return BOOL(1);
            }
            if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW.0 as isize != 0 {
                return BOOL(1);
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 {
                return BOOL(1);
            }
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(hwnd, &mut buf);
            let title = if n > 0 {
                String::from_utf16_lossy(&buf[..n as usize])
            } else {
                String::new()
            };
            titles
                .entry(pid)
                .and_modify(|existing| {
                    if existing.len() < title.len() {
                        *existing = title.clone();
                    }
                })
                .or_insert(title);
        }
        BOOL(1)
    }

    let mut titles = std::collections::HashMap::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut titles as *mut _ as isize));
    }
    titles
}

#[cfg(windows)]
fn skip_system_exe(exe: &str) -> bool {
    matches!(
        exe_basename(exe).as_str(),
        "system"
            | "registry"
            | "smss.exe"
            | "csrss.exe"
            | "wininit.exe"
            | "services.exe"
            | "lsass.exe"
            | "svchost.exe"
            | "audiodg.exe"
            | "idle"
            | "secure system"
            | "memory compression"
    )
}

#[cfg(windows)]
fn windows_snapshot() -> Vec<SnapshotProcess> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return Vec::new();
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut out = Vec::new();
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let exe = wchar_to_string(&entry.szExeFile);
                let aumid = process_aumid(entry.th32ProcessID);
                out.push(SnapshotProcess {
                    pid: entry.th32ProcessID,
                    parent: entry.th32ParentProcessID,
                    exe,
                    aumid,
                });
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(HANDLE(snap.0));
        out
    }
}

#[cfg(windows)]
fn wchar_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|unit| *unit == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

#[cfg(windows)]
fn process_aumid(pid: u32) -> String {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut len = 256u32;
        let mut buf = vec![0u16; len as usize];
        let status = GetApplicationUserModelId(handle, &mut len, PWSTR(buf.as_mut_ptr()));
        let _ = CloseHandle(handle);
        if status != 0 {
            return String::new();
        }
        wchar_to_string(&buf)
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetApplicationUserModelId(
        hprocess: windows::Win32::Foundation::HANDLE,
        applicationusermodelidlength: *mut u32,
        applicationusermodelid: windows::core::PWSTR,
    ) -> u32;
}

#[cfg(test)]
mod tests {
    use super::{exe_matches, pick_tree_root, processes_json};

    #[test]
    fn exe_matches_basename_and_extension() {
        assert!(exe_matches("Spotify.exe", r"C:\Apps\Spotify.exe"));
        assert!(exe_matches("spotify", "Spotify.exe"));
        assert!(exe_matches(r"D:\bin\chrome.exe", "chrome.exe"));
        assert!(!exe_matches("chrome.exe", "firefox.exe"));
        assert!(!exe_matches("", "chrome.exe"));
    }

    #[test]
    fn pick_tree_root_prefers_parent_outside_the_match_set() {
        assert_eq!(pick_tree_root(&[]), None);
        assert_eq!(pick_tree_root(&[(40, 10), (41, 40), (42, 40)]), Some(40));
        assert_eq!(pick_tree_root(&[(8, 1), (9, 2)]), Some(8));
    }

    #[test]
    fn processes_json_is_array() {
        let json = processes_json();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.as_array().is_some(), "{json}");
    }
}
