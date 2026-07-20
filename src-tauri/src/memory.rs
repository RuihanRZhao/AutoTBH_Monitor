//! Read-only Windows process memory primitives for the built-in live meter.
//!
//! Ported to Rust from the MIT-licensed Python reader in
//! <https://github.com/mad-labs-org/tbh-meter> (`reader/shared/memory.py`, `reader/il2cpp/`).
//! Strictly read-only: opens the game process with PROCESS_VM_READ | PROCESS_QUERY_INFORMATION
//! and uses ReadProcessMemory. No writes, no code injection, no handles into game logic.

#![cfg(windows)]

use anyhow::{anyhow, Result};
use std::ffi::c_void;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW,
    MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Memory::{VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

pub struct GameProcess {
    handle: HANDLE,
    pub pid: u32,
    pub module_base: usize,
    pub module_size: usize,
}

impl Drop for GameProcess {
    fn drop(&mut self) {
        unsafe { let _ = CloseHandle(self.handle); }
    }
}

fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Find a running process by executable name (case-insensitive), e.g. "TaskBarHero.exe".
pub fn find_process(name: &str) -> Option<u32> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = None;
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let exe = wide_to_string(&entry.szExeFile);
                if exe.eq_ignore_ascii_case(name) {
                    found = Some(entry.th32ProcessID);
                    break;
                }
                if Process32NextW(snap, &mut entry).is_err() { break; }
            }
        }
        let _ = CloseHandle(snap);
        found
    }
}

/// Base address + size of a loaded module (e.g. "GameAssembly.dll") in the target process.
pub fn find_module(pid: u32, module: &str) -> Option<(usize, usize)> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid).ok()?;
        let mut entry = MODULEENTRY32W {
            dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = None;
        if Module32FirstW(snap, &mut entry).is_ok() {
            loop {
                let name = wide_to_string(&entry.szModule);
                if name.eq_ignore_ascii_case(module) {
                    found = Some((entry.modBaseAddr as usize, entry.modBaseSize as usize));
                    break;
                }
                if Module32NextW(snap, &mut entry).is_err() { break; }
            }
        }
        let _ = CloseHandle(snap);
        found
    }
}

impl GameProcess {
    /// Attach read-only to `process_name`, resolving `module_name`'s base address.
    pub fn attach(process_name: &str, module_name: &str) -> Result<Self> {
        let pid = find_process(process_name).ok_or_else(|| anyhow!("process not running: {process_name}"))?;
        let handle = unsafe { OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, false, pid) }
            .map_err(|e| anyhow!("OpenProcess failed (try running as admin): {e}"))?;
        let (module_base, module_size) =
            find_module(pid, module_name).ok_or_else(|| anyhow!("module not found: {module_name}"))?;
        Ok(Self { handle, pid, module_base, module_size })
    }

    pub fn read_bytes(&self, addr: usize, len: usize) -> Result<Vec<u8>> {
        if addr == 0 || len == 0 { return Err(anyhow!("bad read: addr=0x{addr:x} len={len}")); }
        let mut buf = vec![0u8; len];
        let mut read = 0usize;
        unsafe {
            ReadProcessMemory(
                self.handle,
                addr as *const c_void,
                buf.as_mut_ptr() as *mut c_void,
                len,
                Some(&mut read),
            )
            .map_err(|e| anyhow!("ReadProcessMemory 0x{addr:x}: {e}"))?;
        }
        if read != len { return Err(anyhow!("short read at 0x{addr:x}: {read}/{len}")); }
        Ok(buf)
    }

    pub fn read_u8(&self, addr: usize) -> Result<u8> { Ok(self.read_bytes(addr, 1)?[0]) }
    pub fn read_i32(&self, addr: usize) -> Result<i32> {
        Ok(i32::from_le_bytes(self.read_bytes(addr, 4)?.try_into().unwrap()))
    }
    pub fn read_u32(&self, addr: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read_bytes(addr, 4)?.try_into().unwrap()))
    }
    pub fn read_i64(&self, addr: usize) -> Result<i64> {
        Ok(i64::from_le_bytes(self.read_bytes(addr, 8)?.try_into().unwrap()))
    }
    pub fn read_u64(&self, addr: usize) -> Result<u64> {
        Ok(u64::from_le_bytes(self.read_bytes(addr, 8)?.try_into().unwrap()))
    }
    pub fn read_f32(&self, addr: usize) -> Result<f32> {
        Ok(f32::from_le_bytes(self.read_bytes(addr, 4)?.try_into().unwrap()))
    }
    pub fn read_f64(&self, addr: usize) -> Result<f64> {
        Ok(f64::from_le_bytes(self.read_bytes(addr, 8)?.try_into().unwrap()))
    }
    /// 64-bit pointer read.
    pub fn read_ptr(&self, addr: usize) -> Result<usize> { Ok(self.read_u64(addr)? as usize) }

    /// IL2CPP `System.String`: [0x00 klass][0x08 monitor][0x10 length:i32][0x14 UTF-16 chars].
    pub fn read_il2cpp_string(&self, addr: usize) -> Result<String> {
        if addr == 0 { return Ok(String::new()); }
        let len = self.read_i32(addr + 0x10)?;
        if len <= 0 || len > 8192 { return Ok(String::new()); }
        let bytes = self.read_bytes(addr + 0x14, (len as usize) * 2)?;
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Ok(String::from_utf16_lossy(&units))
    }

    /// IL2CPP `Array<T>`: [0x00 klass][0x08 monitor][0x10 bounds][0x18 max_length][0x20 data…].
    pub fn read_il2cpp_array_len(&self, addr: usize) -> Result<i64> {
        if addr == 0 { return Ok(0); }
        Ok(self.read_i64(addr + 0x18)?)
    }
    pub fn il2cpp_array_data(&self, addr: usize) -> usize { addr + 0x20 }

    /// IL2CPP `List<T>`: [0x10 items(Array<T>)][0x18 size:i32].
    pub fn read_il2cpp_list(&self, addr: usize) -> Result<(usize, i32)> {
        if addr == 0 { return Ok((0, 0)); }
        let items = self.read_ptr(addr + 0x10)?;
        let size = self.read_i32(addr + 0x18)?;
        Ok((items, size))
    }

    /// Read an ASCII C-string (IL2CPP class names). Rejects non-printable bytes.
    pub fn read_cstr(&self, addr: usize, maxlen: usize) -> Option<String> {
        if addr == 0 { return None; }
        let buf = self.read_bytes(addr, maxlen).ok()?;
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        if end == 0 { return Some(String::new()); }
        let s = &buf[..end];
        if s.iter().any(|&b| !(32..127).contains(&b)) { return None; }
        Some(String::from_utf8_lossy(s).to_string())
    }

    /// Validate that `k` looks like an `Il2CppClass` and return its name.
    /// A class self-references through ELEMENT_CLASS (0x40) or CAST_CLASS (0x48).
    pub fn class_name(&self, k: usize) -> Option<String> {
        if k < 0x10000 || k > 0x7FFF_FFFF_FFFF || (k & 0x7) != 0 { return None; }
        let name_ptr = self.read_ptr(k + 0x10).ok()?;
        let name = self.read_cstr(name_ptr, 64)?;
        if name.is_empty() { return None; }
        let ec = self.read_ptr(k + 0x40).unwrap_or(0);
        let cc = self.read_ptr(k + 0x48).unwrap_or(0);
        if ec != k && cc != k { return None; }
        Some(name)
    }

    /// IL2CPP TypeInfoTable: `table = *(module_base + anchor_rva)`, class = `*(table + idx*8)`.
    pub fn class_by_type_index(&self, anchor_rva: usize, type_def_index: usize) -> Result<usize> {
        let table = self.read_ptr(self.module_base + anchor_rva)?;
        if table == 0 { return Err(anyhow!("null TypeInfoTable at rva 0x{anchor_rva:x}")); }
        let k = self.read_ptr(table + type_def_index * 8)?;
        if k == 0 { return Err(anyhow!("null class at index {type_def_index}")); }
        Ok(k)
    }

    /// Singleton instance for a manager class: parent(0x58) -> static fields(0xB8) -> +0.
    /// Must be re-derefed on every read: the GC relocates instances, only the class is stable.
    pub fn singleton_instance(&self, klass: usize) -> Result<usize> {
        let parent = self.read_ptr(klass + 0x58)?;
        if parent == 0 { return Err(anyhow!("no parent class")); }
        let sf = self.read_ptr(parent + 0xB8)?;
        if sf == 0 { return Err(anyhow!("no static fields")); }
        let inst = self.read_ptr(sf)?;
        if inst == 0 { return Err(anyhow!("singleton not initialised")); }
        Ok(inst)
    }

    /// Read `List<T>` element pointers in one batch.
    pub fn list_ptrs(&self, list_obj: usize, cap: i32) -> Result<Vec<usize>> {
        let (items, size) = self.read_il2cpp_list(list_obj)?;
        if items == 0 || size <= 0 || size > cap { return Ok(Vec::new()); }
        let buf = self.read_bytes(self.il2cpp_array_data(items), size as usize * 8)?;
        Ok(buf
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()) as usize)
            .filter(|p| *p != 0)
            .collect())
    }

    /// Iterate a `Dictionary<int,long>`-shaped dict with the 8-byte-value geometry
    /// (STRIDE 0x18, HASH 0x0, KEY 0x8, VALUE 0x10). A negative hash marks a tombstone.
    /// NOTE: this is NOT interchangeable with the 4-byte-value (float) geometry — mixing
    /// them silently corrupts gold/stat readings.
    pub fn dict8b_items(&self, dict_obj: usize, cap: i32) -> Result<Vec<(i32, i64)>> {
        let ent = self.read_ptr(dict_obj + 0x18)?;
        let cnt = self.read_i32(dict_obj + 0x20)?;
        if ent == 0 || cnt < 0 || cnt > cap { return Ok(Vec::new()); }
        let mut out = Vec::new();
        let (mut used, mut j) = (0i32, 0i32);
        // Bound by the entries array's REAL length: `cnt` counts live entries, so `cnt + 64`
        // stops early when more than 64 tombstoned slots precede the live ones — which silently
        // yields no combat gold (and therefore a permanently empty gold/hr column).
        let limit = self
            .read_il2cpp_array_len(ent)
            .ok()
            .filter(|n| *n > 0 && *n < 1_000_000)
            .map(|n| n as i32)
            .unwrap_or(cnt + 64)
            .max(cnt);
        while used < cnt && j < limit {
            let e = ent + 0x20 + (j as usize) * 0x18;
            j += 1;
            let h = match self.read_i32(e) { Ok(h) => h, Err(_) => break };
            if h < 0 { continue; } // tombstone
            used += 1;
            let k = self.read_i32(e + 0x8)?;
            let v = self.read_i64(e + 0x10)?;
            out.push((k, v));
        }
        Ok(out)
    }

    /// Iterate a `Dictionary<StatType,float>` (STRIDE 0x10, KEY 0x8, VALUE 0xC).
    pub fn dictfloat_items(&self, dict_obj: usize, cap: i32) -> Result<Vec<(i32, f32)>> {
        let ent = self.read_ptr(dict_obj + 0x18)?;
        let cnt = self.read_i32(dict_obj + 0x20)?;
        if ent == 0 || cnt < 0 || cnt > cap { return Ok(Vec::new()); }
        let mut out = Vec::new();
        let (mut used, mut j) = (0i32, 0i32);
        let limit = self
            .read_il2cpp_array_len(ent)
            .ok()
            .filter(|n| *n > 0 && *n < 1_000_000)
            .map(|n| n as i32)
            .unwrap_or(cnt + 64)
            .max(cnt);
        while used < cnt && j < limit {
            let e = ent + 0x20 + (j as usize) * 0x10;
            j += 1;
            let h = match self.read_i32(e) { Ok(h) => h, Err(_) => break };
            if h < 0 { continue; }
            used += 1;
            out.push((self.read_i32(e + 0x8)?, self.read_f32(e + 0xC)?));
        }
        Ok(out)
    }

    /// Find live instances of an IL2CPP class by scanning committed memory for objects whose
    /// first qword is the class pointer. Self-references inside the class object itself are
    /// excluded (a class contains pointers to itself).
    pub fn find_instances(&self, klass: usize, limit: usize) -> Vec<usize> {
        let needle = (klass as u64).to_le_bytes();
        let mut hits = Vec::new();
        for (base, size) in self.readable_regions(4096) {
            if hits.len() >= limit { break; }
            // Skip the class object's own neighbourhood.
            if base <= klass && klass < base + size && size < 0x1000 { continue; }
            let mut off = 0usize;
            const CHUNK: usize = 1 << 20;
            while off < size {
                let len = CHUNK.min(size - off);
                let buf = match self.read_bytes(base + off, len) { Ok(b) => b, Err(_) => break };
                let mut i = 0usize;
                while i + 8 <= buf.len() {
                    if buf[i..i + 8] == needle {
                        let addr = base + off + i;
                        if !(klass..klass + 0x400).contains(&addr) {
                            hits.push(addr);
                            if hits.len() >= limit { break; }
                        }
                    }
                    i += 8; // IL2CPP objects are 8-aligned
                }
                if hits.len() >= limit { break; }
                off += len;
            }
        }
        hits
    }

    /// Scan committed memory for a 4-aligned i32 equal to `value`. Used to locate a data record
    /// by a known key when the owning class isn't known yet.
    pub fn scan_i32(&self, value: i32, limit: usize) -> Vec<usize> {
        let needle = value.to_le_bytes();
        let mut hits = Vec::new();
        for (base, size) in self.readable_regions(4096) {
            if hits.len() >= limit { break; }
            let mut off = 0usize;
            const CHUNK: usize = 1 << 20;
            while off < size {
                let len = CHUNK.min(size - off);
                let buf = match self.read_bytes(base + off, len) { Ok(b) => b, Err(_) => break };
                let mut i = 0usize;
                while i + 4 <= buf.len() {
                    if buf[i..i + 4] == needle {
                        hits.push(base + off + i);
                        if hits.len() >= limit { break; }
                    }
                    i += 4;
                }
                if hits.len() >= limit { break; }
                off += len;
            }
        }
        hits
    }

    /// Build fingerprint from the PE header: "<version>-<TimeDateStamp:#x>-<SizeOfImage:#x>".
    pub fn pe_fingerprint(&self, version: &str) -> Result<String> {
        let e_lfanew = self.read_i32(self.module_base + 0x3C)? as usize;
        let sig = self.read_bytes(self.module_base + e_lfanew, 4)?;
        if &sig != b"PE\0\0" { return Err(anyhow!("bad PE signature")); }
        let ts = self.read_u32(self.module_base + e_lfanew + 0x8)?;
        let size = self.read_u32(self.module_base + e_lfanew + 0x50)?;
        Ok(format!("{version}-{ts:#x}-{size:#x}"))
    }

    /// Walk a pointer chain: deref `module_base + base_offset`, then apply each offset with a
    /// deref between steps; the final offset is added without dereferencing.
    pub fn resolve_chain(&self, base_offset: usize, offsets: &[usize]) -> Result<usize> {
        let mut addr = self.read_ptr(self.module_base + base_offset)?;
        for (i, off) in offsets.iter().enumerate() {
            if addr == 0 { return Err(anyhow!("null pointer at chain step {i}")); }
            if i == offsets.len() - 1 {
                addr += *off;
            } else {
                addr = self.read_ptr(addr + *off)?;
            }
        }
        Ok(addr)
    }

    /// Scan the module image for an AOB pattern ("48 8B 05 ?? ?? ?? ??"). Returns absolute addresses.
    pub fn aob_scan(&self, pattern: &str, limit: usize) -> Vec<usize> {
        let pat = parse_pattern(pattern);
        let mut hits = Vec::new();
        if pat.is_empty() { return hits; }
        // Read the module image in chunks with overlap so matches spanning chunk edges are found.
        const CHUNK: usize = 1 << 20;
        let overlap = pat.len();
        let mut off = 0usize;
        while off < self.module_size {
            let len = CHUNK.min(self.module_size - off);
            if let Ok(buf) = self.read_bytes(self.module_base + off, len) {
                for i in 0..=buf.len().saturating_sub(pat.len()) {
                    if pat.iter().enumerate().all(|(j, p)| match p { Some(b) => buf[i + j] == *b, None => true }) {
                        hits.push(self.module_base + off + i);
                        if hits.len() >= limit { return hits; }
                    }
                }
            }
            if len < CHUNK { break; }
            off += CHUNK - overlap;
        }
        hits
    }

    /// Resolve a RIP-relative displacement inside a matched instruction:
    /// `target = hit + instruction_end_offset + i32@(hit + displacement_offset)`.
    pub fn rip_target(&self, hit: usize, displacement_offset: usize, instruction_end_offset: usize) -> Result<usize> {
        let disp = self.read_i32(hit + displacement_offset)?;
        Ok((hit + instruction_end_offset).wrapping_add(disp as isize as usize))
    }

    /// Enumerate committed, readable regions (for heap-wide scans).
    pub fn readable_regions(&self, max_regions: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut addr: usize = 0;
        unsafe {
            while out.len() < max_regions {
                let mut mbi = MEMORY_BASIC_INFORMATION::default();
                let n = VirtualQueryEx(self.handle, Some(addr as *const c_void), &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
                if n == 0 { break; }
                let base = mbi.BaseAddress as usize;
                let size = mbi.RegionSize;
                let prot = mbi.Protect;
                let ok = mbi.State == MEM_COMMIT
                    && (prot & PAGE_GUARD).0 == 0
                    && (prot & PAGE_NOACCESS).0 == 0;
                if ok && size > 0 { out.push((base, size)); }
                let next = base.checked_add(size).unwrap_or(0);
                if next <= addr { break; }
                addr = next;
            }
        }
        out
    }
}

/// Parse "48 8B ?? 05" into byte matchers (None = wildcard).
fn parse_pattern(pattern: &str) -> Vec<Option<u8>> {
    pattern
        .split_whitespace()
        .map(|tok| {
            if tok.starts_with('?') { None } else { u8::from_str_radix(tok, 16).ok() }
        })
        .collect()
}

/// Parse "0x1234" / "1234" into a usize offset.
pub fn parse_offset(s: &str) -> Option<usize> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        usize::from_str_radix(hex, 16).ok()
    } else {
        t.parse::<usize>().ok()
    }
}
