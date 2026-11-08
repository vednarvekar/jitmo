//! Process memory numbers for the test-set runner (PR2: peak under 2GB).

#[cfg(windows)]
mod os {
    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut ProcessMemoryCounters, cb: u32) -> i32;
    }

    fn counters() -> Option<ProcessMemoryCounters> {
        let mut c = ProcessMemoryCounters {
            cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
            page_fault_count: 0,
            peak_working_set_size: 0,
            working_set_size: 0,
            quota_peak_paged_pool_usage: 0,
            quota_paged_pool_usage: 0,
            quota_peak_non_paged_pool_usage: 0,
            quota_non_paged_pool_usage: 0,
            pagefile_usage: 0,
            peak_pagefile_usage: 0,
        };
        // SAFETY: `c` is a valid, correctly sized struct for the whole call, and
        // GetCurrentProcess returns a pseudo-handle that needs no closing.
        let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
        (ok != 0).then_some(c)
    }

    pub fn peak_working_set_bytes() -> Option<u64> {
        counters().map(|c| c.peak_working_set_size as u64)
    }

    pub fn working_set_bytes() -> Option<u64> {
        counters().map(|c| c.working_set_size as u64)
    }
}

/// Highest working set this process has reached so far. `None` off Windows.
pub fn peak_working_set_bytes() -> Option<u64> {
    #[cfg(windows)]
    return os::peak_working_set_bytes();
    #[cfg(not(windows))]
    None
}

/// Working set right now. `None` off Windows.
pub fn working_set_bytes() -> Option<u64> {
    #[cfg(windows)]
    return os::working_set_bytes();
    #[cfg(not(windows))]
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn reports_plausible_numbers_and_peak_never_below_current() {
        let now = working_set_bytes().unwrap();
        let peak = peak_working_set_bytes().unwrap();
        assert!(now > 1_000_000, "a running test binary uses more than 1 MB");
        assert!(peak >= now);
    }

    #[test]
    fn peak_rises_after_a_big_allocation() {
        let before = peak_working_set_bytes().unwrap();
        let mut big = vec![0u8; 64 * 1024 * 1024];
        for i in (0..big.len()).step_by(4096) {
            big[i] = 1; // touch each page so it is really resident
        }
        let after = peak_working_set_bytes().unwrap();
        assert!(after >= before + 32 * 1024 * 1024, "peak should grow by roughly the 64 MB touched");
        drop(big);
    }
}
