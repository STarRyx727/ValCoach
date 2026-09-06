"""
Search the running VALORANT process memory for transform constants.

Usage:
1. Launch VALORANT (the game itself, not just the launcher)
2. Run this script as Administrator: python scripts/find_china_transform.py

This script searches the decrypted process memory for:
- The Multiplier constant (0x2545f4914f6cdd1d) used by the PRNG
- Known SeedAddend values to locate the transform function
- All `add r32, imm32` instructions near the Multiplier reference
"""

import ctypes
import ctypes.wintypes as wt
import struct
import sys

KERNEL32 = ctypes.WinDLL("kernel32", use_last_error=True)
PSAPI = ctypes.WinDLL("psapi", use_last_error=True)

PROCESS_VM_READ = 0x0010
PROCESS_QUERY_INFORMATION = 0x0400

class MEMORY_BASIC_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("BaseAddress", ctypes.c_void_p),
        ("AllocationBase", ctypes.c_void_p),
        ("AllocationProtect", wt.DWORD),
        ("RegionSize", ctypes.c_size_t),
        ("State", wt.DWORD),
        ("Protect", wt.DWORD),
        ("Type", wt.DWORD),
    ]

MEM_COMMIT = 0x1000
PAGE_READWRITE = 0x04
PAGE_READONLY = 0x02
PAGE_EXECUTE_READ = 0x20
PAGE_EXECUTE_READWRITE = 0x40
PAGE_WRITECOPY = 0x08
PAGE_EXECUTE_WRITECOPY = 0x80

MULTIPLIER = 0x2545f4914f6cdd1d
MULTIPLIER_LO = 0x4f6cdd1d
MULTIPLIER_HI = 0x2545f491
GLOBAL_SEED_ADDEND = 0x48c26613

def find_process(name):
    process_ids = (wt.DWORD * 1024)()
    bytes_returned = wt.DWORD()
    PSAPI.EnumProcesses(ctypes.byref(process_ids), ctypes.sizeof(process_ids), ctypes.byref(bytes_returned))
    count = bytes_returned.value // ctypes.sizeof(wt.DWORD)
    for i in range(count):
        pid = process_ids[i]
        if pid == 0:
            continue
        handle = KERNEL32.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, pid)
        if not handle:
            continue
        image_name = (ctypes.c_char * 260)()
        PSAPI.GetProcessImageFileNameA(handle, image_name, 260)
        KERNEL32.CloseHandle(handle)
        if name.lower() in image_name.value.decode("ascii", errors="replace").lower():
            return pid
    return None

def scan_process_memory(pid, patterns):
    handle = KERNEL32.OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, False, pid)
    if not handle:
        print(f"Failed to open process {pid}: error {ctypes.get_last_error()}")
        return {}

    results = {name: [] for name in patterns}
    address = 0
    mbi = MEMORY_BASIC_INFORMATION()
    
    while address < 0x7fffffffffff:
        if KERNEL32.VirtualQueryEx(handle, ctypes.c_void_p(address), ctypes.byref(mbi), ctypes.sizeof(mbi)) == 0:
            break
        
        region_size = mbi.RegionSize
        if mbi.State == MEM_COMMIT and mbi.Protect in (PAGE_READWRITE, PAGE_READONLY, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_WRITECOPY, PAGE_EXECUTE_WRITECOPY):
            base = mbi.BaseAddress or 0
            size = region_size
            try:
                buffer = (ctypes.c_char * min(size, 64 * 1024 * 1024))()
                bytes_read = ctypes.c_size_t()
                
                chunk_size = 4 * 1024 * 1024
                for offset in range(0, size, chunk_size):
                    read_size = min(chunk_size, size - offset)
                    if KERNEL32.ReadProcessMemory(handle, ctypes.c_void_p(base + offset), buffer, read_size, ctypes.byref(bytes_read)):
                        data = buffer[:bytes_read.value]
                        for name, pattern in patterns.items():
                            pos = 0
                            while True:
                                pos = data.find(pattern, pos)
                                if pos == -1:
                                    break
                                results[name].append(base + offset + pos)
                                pos += 1
            except Exception:
                pass
        
        address += region_size
        if region_size == 0:
            break
    
    KERNEL32.CloseHandle(handle)
    return results

def disasm_context(handle, address, before=64, after=64):
    buffer = (ctypes.c_char * (before + after))()
    bytes_read = ctypes.c_size_t()
    if KERNEL32.ReadProcessMemory(handle, ctypes.c_void_p(address - before), buffer, before + after, ctypes.byref(bytes_read)):
        return buffer[:bytes_read.value]
    return None

def main():
    print("Searching for VALORANT process...")
    pid = find_process("VALORANT-Win64-Shipping")
    if not pid:
        print("VALORANT process not found. Please launch the game first.")
        sys.exit(1)
    
    print(f"Found VALORANT process: PID {pid}")
    
    patterns = {
        "Multiplier (8 bytes)": struct.pack("<Q", MULTIPLIER),
        "Multiplier_lo (4 bytes)": struct.pack("<I", MULTIPLIER_LO),
        "Multiplier_hi (4 bytes)": struct.pack("<I", MULTIPLIER_HI),
        "Global_SeedAddend (4 bytes)": struct.pack("<I", GLOBAL_SEED_ADDEND),
    }
    
    print("Scanning process memory (this may take a minute)...")
    results = scan_process_memory(pid, patterns)
    
    print()
    for name, hits in results.items():
        print(f"{name}: {len(hits)} hits")
        for hit in hits[:10]:
            print(f"  0x{hit:016x}")
    
    # If we found the Multiplier, look for SeedAddend nearby
    mult_hits = results.get("Multiplier (8 bytes)", [])
    if not mult_hits:
        mult_hits = results.get("Multiplier_lo (4 bytes)", [])
    
    if mult_hits:
        print()
        print(f"=== Analyzing area around first Multiplier hit ===")
        handle = KERNEL32.OpenProcess(PROCESS_VM_READ, False, pid)
        if handle:
            hit = mult_hits[0]
            ctx = disasm_context(handle, hit, 256, 256)
            if ctx:
                print(f"Hex dump around 0x{hit:016x}:")
                for i in range(0, len(ctx), 16):
                    addr = hit - 256 + i
                    hex_bytes = " ".join(f"{b:02x}" for b in ctx[i:i+16])
                    print(f"  {addr:016x}  {hex_bytes}")
                    
                    # Check for add reg, imm32 instructions
                    for j in range(i, min(i + 16, len(ctx) - 5)):
                        if ctx[j] == 0x05:  # add eax, imm32
                            imm = struct.unpack_from("<I", ctx, j + 1)[0]
                            if imm > 0x10000000:
                                print(f"    ^^^ add eax, 0x{imm:08x} at 0x{hit - 256 + j:016x}")
                        elif ctx[j] == 0x81 and j + 5 < len(ctx) and (ctx[j+1] & 0xF8) == 0xC0:
                            imm = struct.unpack_from("<I", ctx, j + 2)[0]
                            if imm > 0x10000000:
                                reg = ["eax","ecx","edx","ebx","esp","ebp","esi","edi"][ctx[j+1] & 7]
                                print(f"    ^^^ add {reg}, 0x{imm:08x} at 0x{hit - 256 + j:016x}")
            KERNEL32.CloseHandle(handle)
    else:
        print()
        print("Multiplier constant not found in memory!")
        print("The game might not have loaded the replay system yet.")
        print("Try watching a replay in-game first, then run this script again.")

if __name__ == "__main__":
    main()
