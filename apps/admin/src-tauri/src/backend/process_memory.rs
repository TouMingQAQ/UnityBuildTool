//! Windows Task Manager's default memory column uses private working set.
#[cfg(windows)]
pub(super) fn private_working_set(pid: u32) -> Option<u64> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetSystemInfo(info: *mut SystemInfo);
    }
    #[link(name = "psapi")]
    extern "system" { fn QueryWorkingSet(process: *mut c_void, buffer: *mut c_void, size: u32) -> i32; }
    #[repr(C)]
    struct SystemInfo { architecture: u32, page_size: u32, min: usize, max: usize, mask: usize, processors: u32, kind: u32, granularity: u32, level: u16, revision: u16 }
    unsafe {
        let handle = OpenProcess(0x0400 | 0x0010, 0, pid); // QUERY_INFORMATION | VM_READ
        if handle.is_null() { return None; }
        let mut result = None;
        let mut capacity = 16384usize;
        while capacity <= 4_194_304 {
            let mut buffer = vec![0usize; capacity];
            if QueryWorkingSet(handle, buffer.as_mut_ptr().cast(), (capacity * std::mem::size_of::<usize>()) as u32) != 0 {
                let entries = buffer[0];
                if entries < capacity {
                    let mut info: SystemInfo = std::mem::zeroed();
                    GetSystemInfo(&mut info);
                    // PSAPI_WORKING_SET_BLOCK.Shared is bit 8; shared pages are excluded.
                    let private_pages = buffer[1..1 + entries].iter().filter(|entry| **entry & (1 << 8) == 0).count();
                    result = Some(private_pages as u64 * info.page_size as u64);
                }
                break;
            }
            capacity *= 2;
        }
        CloseHandle(handle);
        result
    }
}
#[cfg(not(windows))]
pub(super) fn private_working_set(_pid: u32) -> Option<u64> { None }

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn current_process_has_private_resident_pages() {
        assert!(super::private_working_set(std::process::id()).unwrap() > 0);
        assert!(super::private_working_set(0).is_none());
    }
}
