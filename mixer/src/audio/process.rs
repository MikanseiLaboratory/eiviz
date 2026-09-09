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
pub fn activate_process_client(
    pid: u32,
) -> Result<windows::Win32::Media::Audio::IAudioClient, String> {
    windows_activate(pid)
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
    let mut by_key: HashMap<(String, String), CaptureProcess> = HashMap::new();
    for process in windows_snapshot() {
        if process.pid == 0 || process.pid == self_pid || skip_system_exe(&process.exe) {
            continue;
        }
        let exe = if process.exe.is_empty() {
            continue;
        } else {
            process.exe
        };
        let aumid = process.aumid;
        let key = (exe_basename(&exe), aumid.to_ascii_lowercase());
        by_key.entry(key).or_insert(CaptureProcess {
            name: exe.clone(),
            exe,
            aumid,
        });
    }
    let mut out: Vec<CaptureProcess> = by_key.into_values().collect();
    out.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    out
}

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
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
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
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::core::PWSTR;

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

#[cfg(windows)]
#[windows_core::implement(windows::Win32::Media::Audio::IActivateAudioInterfaceCompletionHandler)]
struct ActivateHandler(std::sync::mpsc::Sender<windows::core::Result<windows::core::IUnknown>>);

#[cfg(windows)]
impl windows::Win32::Media::Audio::IActivateAudioInterfaceCompletionHandler_Impl
    for ActivateHandler_Impl
{
    fn ActivateCompleted(
        &self,
        operation: windows::core::Ref<
            '_,
            windows::Win32::Media::Audio::IActivateAudioInterfaceAsyncOperation,
        >,
    ) -> windows::core::Result<()> {
        let result = match operation.as_ref() {
            Some(op) => retrieve_client(op),
            None => Err(windows::core::Error::from(
                windows::Win32::Foundation::E_POINTER,
            )),
        };
        let _ = self.0.send(result);
        Ok(())
    }
}

#[cfg(windows)]
fn retrieve_client(
    operation: &windows::Win32::Media::Audio::IActivateAudioInterfaceAsyncOperation,
) -> windows::core::Result<windows::core::IUnknown> {
    let mut status = windows::core::HRESULT::default();
    let mut unknown = None;
    unsafe {
        operation.GetActivateResult(&mut status, &mut unknown)?;
    }
    status.ok()?;
    unknown.ok_or_else(|| {
        windows::core::Error::new(
            windows::Win32::Foundation::E_FAIL,
            "process loopback activation returned no interface",
        )
    })
}

#[cfg(windows)]
fn windows_activate(pid: u32) -> Result<windows::Win32::Media::Audio::IAudioClient, String> {
    use std::mem::{ManuallyDrop, size_of};
    use std::sync::mpsc;
    use std::time::Duration;
    use windows::Win32::Media::Audio::{
        AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
        AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
        ActivateAudioInterfaceAsync, IActivateAudioInterfaceCompletionHandler, IAudioClient,
        PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    };
    use windows::Win32::System::Com::BLOB;
    use windows::Win32::System::Com::StructuredStorage::{
        PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
    };
    use windows::Win32::System::Variant::VT_BLOB;
    use windows::core::Interface;

    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let prop = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: std::ptr::from_mut(&mut params).cast(),
                    },
                },
            }),
        },
    };
    let (tx, rx) = mpsc::channel();
    let handler: IActivateAudioInterfaceCompletionHandler = ActivateHandler(tx).into();
    let _operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&prop),
            &handler,
        )
    }
    .map_err(|error| activate_error(error))?;
    let unknown = rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "WASAPI process loopback activation timed out".to_string())?
        .map_err(activate_error)?;
    unknown
        .cast()
        .map_err(|error| format!("process loopback IAudioClient: {error}"))
}

#[cfg(windows)]
fn activate_error(error: windows::core::Error) -> String {
    let code = error.code().0 as u32;
    if matches!(code, 0x8000_4001 | 0x8000_4002) {
        "WASAPI process loopback needs Windows 10 1903 or later".into()
    } else {
        format!("WASAPI process loopback activate: {error}")
    }
}

#[cfg(windows)]
pub fn activate_is_fatal(error: &str) -> bool {
    error.contains("Windows 10 1903")
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
