//! Best-effort name of the process listening on a TCP port.

pub fn name(port: u16) -> Option<String> {
    if port == 0 {
        return None;
    }
    platform_name(port)
}

#[cfg(windows)]
fn platform_name(port: u16) -> Option<String> {
    for pid in pids_on_port(port) {
        if pid == 0 || pid == std::process::id() {
            continue;
        }
        if let Some(name) = process_name(pid) {
            return Some(name);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn platform_name(port: u16) -> Option<String> {
    let output = std::process::Command::new("/usr/sbin/lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let self_pid = std::process::id().to_string();
    for line in text.lines().skip(1) {
        let mut cols = line.split_whitespace();
        let command = cols.next()?;
        let pid = cols.next()?;
        if pid == self_pid {
            continue;
        }
        if !command.is_empty() {
            return Some(command.to_string());
        }
    }
    None
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_name(_port: u16) -> Option<String> {
    None
}

#[cfg(windows)]
fn pids_on_port(port: u16) -> Vec<u32> {
    let mut pids = query_extended(2, 3, port, false);
    pids.extend(query_extended(23, 3, port, true));
    pids.extend(query_extended(2, 5, port, false));
    pids.extend(query_extended(23, 5, port, true));
    pids.extend(query_table2(port));
    if pids.is_empty() {
        pids.extend(netstat_pids(port));
    }
    pids
}

#[cfg(windows)]
fn query_extended(family: u32, class: u32, port: u16, ipv6: bool) -> Vec<u32> {
    let mut pids = Vec::new();
    let Some(buffer) =
        tcp_table(|size, ptr| unsafe { GetExtendedTcpTable(ptr, size, 1, family, class, 0) })
    else {
        return pids;
    };
    let count = u32::from_le_bytes(buffer[0..4].try_into().unwrap_or([0; 4])) as usize;
    let row_size = if ipv6 { 56 } else { 24 };
    let port_off = if ipv6 { 20 } else { 8 };
    let pid_off = if ipv6 { 52 } else { 20 };
    for i in 0..count {
        let start = 4 + i * row_size;
        if start + row_size > buffer.len() {
            break;
        }
        let row = &buffer[start..start + row_size];
        if host_port(row, port_off) == port {
            pids.push(u32::from_le_bytes(
                row[pid_off..pid_off + 4].try_into().unwrap_or([0; 4]),
            ));
        }
    }
    pids
}

#[cfg(windows)]
fn query_table2(port: u16) -> Vec<u32> {
    let mut pids = Vec::new();
    let Some(buffer) = tcp_table(|size, ptr| unsafe { GetTcpTable2(ptr, size, 1) }) else {
        return pids;
    };
    let count = u32::from_le_bytes(buffer[0..4].try_into().unwrap_or([0; 4])) as usize;
    const ROW: usize = 28;
    for i in 0..count {
        let start = 4 + i * ROW;
        if start + ROW > buffer.len() {
            break;
        }
        let row = &buffer[start..start + ROW];
        let state = u32::from_le_bytes(row[0..4].try_into().unwrap_or([0; 4]));
        if state == 2 && host_port(row, 8) == port {
            pids.push(u32::from_le_bytes(row[20..24].try_into().unwrap_or([0; 4])));
        }
    }
    pids
}

#[cfg(windows)]
fn tcp_table(query: impl Fn(&mut u32, *mut u8) -> u32) -> Option<Vec<u8>> {
    let mut size = 0u32;
    if query(&mut size, std::ptr::null_mut()) != 122 || size == 0 {
        return None;
    }
    let mut buffer = vec![0u8; size as usize];
    if query(&mut size, buffer.as_mut_ptr()) != 0 || buffer.len() < 4 {
        return None;
    }
    Some(buffer)
}

#[cfg(windows)]
fn netstat_pids(port: u16) -> Vec<u32> {
    let output = match std::process::Command::new("netstat")
        .args(["-ano", "-p", "tcp"])
        .output()
    {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };
    let needle = format!(":{port}");
    let mut pids = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if !line.contains("LISTENING") || !line.contains(&needle) {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 5 || !cols[0].eq_ignore_ascii_case("TCP") {
            continue;
        }
        let Some((_, local_port)) = cols[1].rsplit_once(':') else {
            continue;
        };
        if local_port.parse() != Ok(port) {
            continue;
        }
        if let Ok(pid) = cols[4].parse() {
            pids.push(pid);
        }
    }
    pids
}

#[cfg(windows)]
fn host_port(row: &[u8], offset: usize) -> u16 {
    let raw = u32::from_le_bytes(row[offset..offset + 4].try_into().unwrap_or([0; 4]));
    u16::from_be((raw & 0xFFFF) as u16)
}

#[cfg(windows)]
fn process_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(0x1000, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        if ok == 0 || len == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let stem = std::path::Path::new(&path)
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())?;
        Some(stem.to_string())
    }
}

#[cfg(windows)]
#[link(name = "iphlpapi")]
unsafe extern "system" {
    fn GetExtendedTcpTable(
        table: *mut u8,
        size: *mut u32,
        order: i32,
        af: u32,
        table_class: u32,
        reserved: u32,
    ) -> u32;
    fn GetTcpTable2(table: *mut u8, size: *mut u32, order: i32) -> u32;
}

#[cfg(windows)]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut core::ffi::c_void;
    fn QueryFullProcessImageNameW(
        process: *mut core::ffi::c_void,
        flags: u32,
        name: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
}

#[cfg(test)]
mod tests {
    #[test]
    fn lookup_is_best_effort() {
        let _ = super::name(8088);
    }
}
