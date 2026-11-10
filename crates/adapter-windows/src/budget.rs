//! What the memory budget manager needs from Windows (low-memory brief, step 7):
//! how much physical memory is free, and a hard memory cap on a child process (a Job Object
//! that also kills the child when the job is closed, so a model process can never outlive
//! jitmo or grow past its cap).

/// Free physical memory right now, in bytes. `None` off Windows or when it cannot be read.
pub fn free_physical_bytes() -> Option<u64> {
    #[cfg(windows)]
    return os::free_physical_bytes();
    #[cfg(not(windows))]
    None
}

/// Total physical memory, in bytes.
pub fn total_physical_bytes() -> Option<u64> {
    #[cfg(windows)]
    return os::total_physical_bytes();
    #[cfg(not(windows))]
    None
}

/// A Job Object holding one child process. Dropping it closes the job, which ends the child.
pub struct MemoryCap {
    #[cfg(windows)]
    job: isize,
}

impl Drop for MemoryCap {
    fn drop(&mut self) {
        #[cfg(windows)]
        os::close(self.job);
    }
}

/// Put `child` in a job that limits its memory to `bytes` and ends it with the job.
pub fn cap_memory(child: &std::process::Child, bytes: u64) -> Result<MemoryCap, String> {
    #[cfg(windows)]
    {
        os::cap(child, bytes)
    }
    #[cfg(not(windows))]
    {
        let _ = (child, bytes);
        Err("memory caps need Windows".into())
    }
}

#[cfg(windows)]
mod os {
    use super::MemoryCap;
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    #[repr(C)]
    struct BasicLimits {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set: usize,
        maximum_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct IoCounters {
        read_ops: u64,
        write_ops: u64,
        other_ops: u64,
        read_bytes: u64,
        write_bytes: u64,
        other_bytes: u64,
    }

    #[repr(C)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x0000_0100;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
        fn CreateJobObjectW(attributes: *const u8, name: *const u16) -> isize;
        fn SetInformationJobObject(job: isize, class: i32, info: *const u8, size: u32) -> i32;
        fn AssignProcessToJobObject(job: isize, process: isize) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    fn status() -> Option<MemoryStatusEx> {
        let mut s = MemoryStatusEx {
            length: std::mem::size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        // SAFETY: `s` is a valid struct of the size it declares in `length`.
        (unsafe { GlobalMemoryStatusEx(&mut s) } != 0).then_some(s)
    }

    pub fn free_physical_bytes() -> Option<u64> {
        status().map(|s| s.avail_phys)
    }

    pub fn total_physical_bytes() -> Option<u64> {
        status().map(|s| s.total_phys)
    }

    pub fn close(job: isize) {
        // SAFETY: `job` is a handle this module created and has not closed.
        unsafe {
            CloseHandle(job);
        }
    }

    pub fn cap(child: &std::process::Child, bytes: u64) -> Result<MemoryCap, String> {
        // SAFETY: plain Win32 calls with valid arguments; the handle is closed on every
        // failure path and otherwise owned by the returned `MemoryCap`.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job == 0 {
                return Err("could not create a job object".into());
            }
            let limits = ExtendedLimits {
                basic: BasicLimits {
                    per_process_user_time_limit: 0,
                    per_job_user_time_limit: 0,
                    limit_flags: JOB_OBJECT_LIMIT_PROCESS_MEMORY
                        | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    minimum_working_set: 0,
                    maximum_working_set: 0,
                    active_process_limit: 0,
                    affinity: 0,
                    priority_class: 0,
                    scheduling_class: 0,
                },
                io: IoCounters {
                    read_ops: 0,
                    write_ops: 0,
                    other_ops: 0,
                    read_bytes: 0,
                    write_bytes: 0,
                    other_bytes: 0,
                },
                process_memory_limit: bytes as usize,
                job_memory_limit: 0,
                peak_process_memory_used: 0,
                peak_job_memory_used: 0,
            };
            let ok = SetInformationJobObject(
                job,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                &limits as *const ExtendedLimits as *const u8,
                std::mem::size_of::<ExtendedLimits>() as u32,
            );
            if ok == 0 {
                CloseHandle(job);
                return Err("could not set the memory limit".into());
            }
            if AssignProcessToJobObject(job, child.as_raw_handle() as isize) == 0 {
                CloseHandle(job);
                return Err("could not put the process in the job".into());
            }
            Ok(MemoryCap { job })
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn free_memory_is_plausible() {
        let free = free_physical_bytes().unwrap();
        let total = total_physical_bytes().unwrap();
        assert!(free > 10_000_000 && free <= total);
    }

    #[test]
    fn a_capped_child_ends_when_the_cap_is_dropped() {
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "ping -n 30 127.0.0.1 > nul"])
            .spawn()
            .unwrap();
        let cap = cap_memory(&child, 512 * 1024 * 1024).unwrap();
        assert!(child.try_wait().unwrap().is_none());
        drop(cap);
        // Closing the job ends the process (kill-on-close).
        let mut ended = false;
        for _ in 0..40 {
            if child.try_wait().unwrap().is_some() {
                ended = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(ended, "the child must end with its job");
    }
}
