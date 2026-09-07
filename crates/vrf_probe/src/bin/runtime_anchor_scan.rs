//! Read-only Windows process-memory scanner for payload-transform anchors.
//!
//! The scanner requests only query and VM-read access. It never writes to the target process,
//! changes page protections, creates a remote thread, or injects code.

#[cfg(not(windows))]
fn main() {
    eprintln!("runtime_anchor_scan is available only on Windows");
}

#[cfg(windows)]
mod windows {
    use std::{collections::BTreeMap, env, ffi::c_void, io, mem};

    use serde::Serialize;

    type Handle = *mut c_void;

    const PROCESS_VM_READ: u32 = 0x0010;
    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    const TH32CS_SNAPMODULE: u32 = 0x0000_0008;
    const TH32CS_SNAPMODULE32: u32 = 0x0000_0010;
    const MEM_COMMIT: u32 = 0x1000;
    const PAGE_NOACCESS: u32 = 0x01;
    const PAGE_GUARD: u32 = 0x100;
    const MAX_CHUNK_BYTES: usize = 4 * 1024 * 1024;

    #[repr(C)]
    struct MemoryBasicInformation {
        base_address: *mut c_void,
        allocation_base: *mut c_void,
        allocation_protect: u32,
        partition_id: u16,
        _alignment1: u16,
        region_size: usize,
        state: u32,
        protect: u32,
        kind: u32,
        _alignment2: u32,
    }

    #[repr(C)]
    struct ModuleEntry32W {
        size: u32,
        module_id: u32,
        process_id: u32,
        global_usage: u32,
        process_usage: u32,
        base_address: *mut u8,
        base_size: u32,
        module: Handle,
        module_name: [u16; 256],
        executable_path: [u16; 260],
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
        fn QueryFullProcessImageNameW(
            process: Handle,
            flags: u32,
            buffer: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn VirtualQueryEx(
            process: Handle,
            address: *const c_void,
            buffer: *mut MemoryBasicInformation,
            length: usize,
        ) -> usize;
        fn ReadProcessMemory(
            process: Handle,
            base_address: *const c_void,
            buffer: *mut c_void,
            size: usize,
            bytes_read: *mut usize,
        ) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
        fn Module32FirstW(snapshot: Handle, entry: *mut ModuleEntry32W) -> i32;
        fn Module32NextW(snapshot: Handle, entry: *mut ModuleEntry32W) -> i32;
    }

    struct ProcessHandle(Handle);

    impl Drop for ProcessHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    #[derive(Serialize)]
    struct Report {
        process_id: u32,
        image_path: Option<String>,
        module_base: String,
        module_size: u32,
        access: &'static str,
        committed_regions: usize,
        readable_regions: usize,
        readable_bytes: u64,
        unreadable_bytes: u64,
        anchors: BTreeMap<&'static str, Vec<Hit>>,
    }

    #[derive(Serialize)]
    struct Hit {
        address: String,
        region_base: String,
        protection: String,
        memory_type: String,
        context_hex: String,
    }

    const ANCHORS: &[(&str, &[u8])] = &[
        (
            "prng_multiplier_u64",
            &0x2545_f491_4f6c_dd1du64.to_le_bytes(),
        ),
        ("prng_multiplier_low_u32", &0x4f6c_dd1du32.to_le_bytes()),
        ("prng_multiplier_high_u32", &0x2545_f491u32.to_le_bytes()),
        ("global_seed_addend_13_05", &0x48c2_6613u32.to_le_bytes()),
        (
            "china_seed_addend_13_05_candidate",
            &0xf677_61c9u32.to_le_bytes(),
        ),
        ("byte_mix_1b0829", &0x001b_0829u32.to_le_bytes()),
        // Stable xorshift/rotate immediates around InitialPrng/AdvanceTransformState.
        ("advance_rotate_36", &[0x24]),
        ("advance_rotate_9", &[0x09]),
    ];

    fn protection_name(protect: u32) -> String {
        format!("0x{protect:08X}")
    }

    fn type_name(kind: u32) -> String {
        match kind {
            0x0002_0000 => "MEM_PRIVATE".into(),
            0x0004_0000 => "MEM_MAPPED".into(),
            0x0100_0000 => "MEM_IMAGE".into(),
            _ => format!("0x{kind:08X}"),
        }
    }

    fn readable(protect: u32) -> bool {
        protect != 0 && protect & (PAGE_NOACCESS | PAGE_GUARD) == 0
    }

    fn image_path(process: Handle) -> Option<String> {
        let mut buffer = vec![0u16; 32_768];
        let mut length = buffer.len() as u32;
        let ok =
            unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) };
        (ok != 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
    }

    fn wide_string(value: &[u16]) -> String {
        let length = value
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(value.len());
        String::from_utf16_lossy(&value[..length])
    }

    fn main_module(pid: u32) -> Result<(usize, u32, String), Box<dyn std::error::Error>> {
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
        if raw as isize == -1 {
            return Err(io::Error::last_os_error().into());
        }
        let snapshot = ProcessHandle(raw);
        let mut entry: ModuleEntry32W = unsafe { mem::zeroed() };
        entry.size = mem::size_of::<ModuleEntry32W>() as u32;
        let mut ok = unsafe { Module32FirstW(snapshot.0, &mut entry) };
        while ok != 0 {
            let name = wide_string(&entry.module_name);
            if name.eq_ignore_ascii_case("VALORANT-Win64-Shipping.exe") {
                return Ok((
                    entry.base_address as usize,
                    entry.base_size,
                    wide_string(&entry.executable_path),
                ));
            }
            ok = unsafe { Module32NextW(snapshot.0, &mut entry) };
        }
        Err("VALORANT-Win64-Shipping.exe module was not enumerable".into())
    }

    fn scan_chunk(
        chunk: &[u8],
        chunk_address: usize,
        region_base: usize,
        protection: u32,
        kind: u32,
        hits: &mut BTreeMap<&'static str, Vec<Hit>>,
    ) {
        for (name, needle) in ANCHORS {
            // One-byte immediates are useful only as context once a stronger anchor matched.
            if needle.len() == 1 {
                continue;
            }
            for (offset, window) in chunk.windows(needle.len()).enumerate() {
                if window != *needle {
                    continue;
                }
                let start = offset.saturating_sub(48);
                let end = (offset + needle.len() + 96).min(chunk.len());
                hits.entry(name).or_default().push(Hit {
                    address: format!("0x{:016X}", chunk_address + offset),
                    region_base: format!("0x{region_base:016X}"),
                    protection: protection_name(protection),
                    memory_type: type_name(kind),
                    context_hex: hex::encode_upper(&chunk[start..end]),
                });
                if hits.get(name).is_some_and(|values| values.len() >= 256) {
                    break;
                }
            }
        }
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let pid: u32 = env::args()
            .nth(1)
            .ok_or("usage: runtime_anchor_scan <pid>")?
            .parse()?;
        let raw = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
        if raw.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let process = ProcessHandle(raw);
        let (module_base, module_size, module_path) = main_module(pid)?;
        let module_end = module_base.saturating_add(module_size as usize);
        let mut address = module_base;
        let mut committed_regions = 0usize;
        let mut readable_regions = 0usize;
        let mut readable_bytes = 0u64;
        let mut unreadable_bytes = 0u64;
        let mut hits = BTreeMap::new();

        while address < module_end {
            let mut info: MemoryBasicInformation = unsafe { mem::zeroed() };
            let queried = unsafe {
                VirtualQueryEx(
                    process.0,
                    address as *const c_void,
                    &mut info,
                    mem::size_of::<MemoryBasicInformation>(),
                )
            };
            if queried == 0 || info.region_size == 0 {
                break;
            }
            let base = info.base_address as usize;
            let next = base.saturating_add(info.region_size);
            if info.state == MEM_COMMIT {
                committed_regions += 1;
                if readable(info.protect) {
                    let mut offset = 0usize;
                    let mut region_read = false;
                    while offset < info.region_size {
                        let requested = (info.region_size - offset).min(MAX_CHUNK_BYTES);
                        let mut buffer = vec![0u8; requested];
                        let mut actual = 0usize;
                        let ok = unsafe {
                            ReadProcessMemory(
                                process.0,
                                (base + offset) as *const c_void,
                                buffer.as_mut_ptr().cast(),
                                requested,
                                &mut actual,
                            )
                        };
                        if ok != 0 && actual != 0 {
                            region_read = true;
                            readable_bytes += actual as u64;
                            buffer.truncate(actual);
                            scan_chunk(
                                &buffer,
                                base + offset,
                                base,
                                info.protect,
                                info.kind,
                                &mut hits,
                            );
                        } else {
                            unreadable_bytes += requested as u64;
                        }
                        offset = offset.saturating_add(requested);
                    }
                    if region_read {
                        readable_regions += 1;
                    }
                } else {
                    unreadable_bytes += info.region_size as u64;
                }
            }
            if next <= address {
                break;
            }
            address = next;
        }

        println!(
            "{}",
            serde_json::to_string_pretty(&Report {
                process_id: pid,
                image_path: image_path(process.0).or(Some(module_path)),
                module_base: format!("0x{module_base:016X}"),
                module_size,
                access: "PROCESS_QUERY_INFORMATION | PROCESS_VM_READ",
                committed_regions,
                readable_regions,
                readable_bytes,
                unreadable_bytes,
                anchors: hits,
            })?
        );
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows::run()
}
