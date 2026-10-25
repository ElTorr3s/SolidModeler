//! Locks down the STEP import worker before it touches untrusted bytes.
//!
//! The sandbox is not optional: [`lock_down`] fails closed, and the worker
//! refuses to read a file unless every layer is in place.
//!
//! * Resource limits cap address space, CPU time and open files, and forbid
//!   writing files or core dumps.
//! * `no_new_privs` stops privilege gain through exec.
//! * Landlock removes all filesystem access (no reads, no writes).
//! * Seccomp denies exec, fork, sockets, ptrace, mounts, namespaces, module
//!   loading and every syscall that writes to the filesystem, as a second
//!   line behind Landlock.
//!
//! What is left is enough to read stdin, compute, allocate, start threads
//! and write the result to stdout.

/// Limits for one worker run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerLimits {
    pub memory_bytes: u64,
    pub cpu_seconds: u64,
}

impl Default for WorkerLimits {
    fn default() -> Self {
        WorkerLimits {
            memory_bytes: 4 << 30,
            cpu_seconds: 300,
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::lock_down;

#[cfg(not(target_os = "linux"))]
pub fn lock_down(_: &WorkerLimits) -> Result<(), String> {
    Err("the STEP import sandbox is only implemented on Linux".into())
}

#[cfg(target_os = "linux")]
mod linux {
    use super::WorkerLimits;
    use landlock::{Access, AccessFs, AccessNet, Ruleset, RulesetAttr, RulesetStatus, Scope, ABI};
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule, TargetArch,
    };
    use std::collections::BTreeMap;

    pub fn lock_down(limits: &WorkerLimits) -> Result<(), String> {
        rlimits(limits)?;
        // SAFETY: prctl with these arguments only sets a flag on this process.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            return Err(format!("no_new_privs: {}", std::io::Error::last_os_error()));
        }
        landlock()?;
        seccomp()?;
        Ok(())
    }

    fn rlimits(limits: &WorkerLimits) -> Result<(), String> {
        let set = |resource, value: u64, name: &str| {
            let lim = libc::rlimit {
                rlim_cur: value,
                rlim_max: value,
            };
            // SAFETY: setrlimit reads the struct we pass and changes only
            // this process's limits.
            if unsafe { libc::setrlimit(resource, &lim) } != 0 {
                return Err(format!("{name}: {}", std::io::Error::last_os_error()));
            }
            Ok(())
        };
        set(libc::RLIMIT_AS, limits.memory_bytes, "RLIMIT_AS")?;
        set(libc::RLIMIT_CPU, limits.cpu_seconds, "RLIMIT_CPU")?;
        set(libc::RLIMIT_FSIZE, 0, "RLIMIT_FSIZE")?;
        set(libc::RLIMIT_CORE, 0, "RLIMIT_CORE")?;
        set(libc::RLIMIT_NOFILE, 16, "RLIMIT_NOFILE")?;
        Ok(())
    }

    fn landlock() -> Result<(), String> {
        // Handle every access right this kernel knows and grant none: no
        // path is reachable afterwards. Network and IPC scoping where the
        // kernel supports them.
        let abi = ABI::V6;
        let status = Ruleset::default()
            .handle_access(AccessFs::from_all(abi))
            .and_then(|r| r.handle_access(AccessNet::from_all(abi)))
            .and_then(|r| r.scope(Scope::from_all(abi)))
            .and_then(|r| r.create())
            .and_then(|r| r.restrict_self())
            .map_err(|e| format!("landlock: {e}"))?;
        match status.ruleset {
            RulesetStatus::FullyEnforced | RulesetStatus::PartiallyEnforced => Ok(()),
            RulesetStatus::NotEnforced => {
                Err("landlock is not available in this kernel; refusing to import".into())
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    const ARCH: TargetArch = TargetArch::x86_64;
    #[cfg(target_arch = "aarch64")]
    const ARCH: TargetArch = TargetArch::aarch64;

    fn masked(arg: u8, bits: u64) -> Result<SeccompRule, String> {
        let cond = SeccompCondition::new(
            arg,
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::MaskedEq(bits),
            bits,
        )
        .map_err(|e| e.to_string())?;
        SeccompRule::new(vec![cond]).map_err(|e| e.to_string())
    }

    fn apply(rules: BTreeMap<i64, Vec<SeccompRule>>, errno: i32) -> Result<(), String> {
        let filter = SeccompFilter::new(
            rules,
            SeccompAction::Allow,
            SeccompAction::Errno(errno as u32),
            ARCH,
        )
        .map_err(|e| format!("seccomp: {e}"))?;
        let program: BpfProgram = filter.try_into().map_err(|e| format!("seccomp: {e:?}"))?;
        seccompiler::apply_filter(&program).map_err(|e| format!("seccomp: {e}"))
    }

    fn seccomp() -> Result<(), String> {
        let mut deny: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
        let always = [
            libc::SYS_execve,
            libc::SYS_execveat,
            libc::SYS_fork,
            libc::SYS_vfork,
            libc::SYS_socket,
            libc::SYS_socketpair,
            libc::SYS_connect,
            libc::SYS_bind,
            libc::SYS_listen,
            libc::SYS_accept,
            libc::SYS_accept4,
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
            libc::SYS_mount,
            libc::SYS_umount2,
            libc::SYS_pivot_root,
            libc::SYS_chroot,
            libc::SYS_unshare,
            libc::SYS_setns,
            libc::SYS_init_module,
            libc::SYS_finit_module,
            libc::SYS_delete_module,
            libc::SYS_kexec_load,
            libc::SYS_bpf,
            libc::SYS_perf_event_open,
            libc::SYS_keyctl,
            libc::SYS_add_key,
            libc::SYS_request_key,
            libc::SYS_userfaultfd,
            libc::SYS_unlinkat,
            libc::SYS_renameat,
            libc::SYS_renameat2,
            libc::SYS_mkdirat,
            libc::SYS_symlinkat,
            libc::SYS_linkat,
            libc::SYS_fchmodat,
            libc::SYS_fchownat,
            libc::SYS_truncate,
            libc::SYS_mknodat,
            libc::SYS_setuid,
            libc::SYS_setgid,
            libc::SYS_setreuid,
            libc::SYS_setregid,
            libc::SYS_setresuid,
            libc::SYS_setresgid,
            libc::SYS_setgroups,
            libc::SYS_io_uring_setup,
        ];
        for nr in always {
            deny.insert(nr, Vec::new());
        }
        #[cfg(target_arch = "x86_64")]
        for nr in [
            libc::SYS_unlink,
            libc::SYS_rename,
            libc::SYS_mkdir,
            libc::SYS_rmdir,
            libc::SYS_symlink,
            libc::SYS_link,
            libc::SYS_chmod,
            libc::SYS_chown,
            libc::SYS_lchown,
            libc::SYS_creat,
            libc::SYS_mknod,
        ] {
            deny.insert(nr, Vec::new());
        }
        // Opening for writing or creating; read-only opens are left to
        // Landlock, which denies every path anyway.
        let write_bits = [libc::O_WRONLY, libc::O_RDWR, libc::O_CREAT, libc::O_TRUNC];
        let open_rules = |arg: u8| -> Result<Vec<SeccompRule>, String> {
            write_bits.iter().map(|&b| masked(arg, b as u64)).collect()
        };
        deny.insert(libc::SYS_openat, open_rules(2)?);
        #[cfg(target_arch = "x86_64")]
        deny.insert(libc::SYS_open, open_rules(1)?);
        // New processes (clone without CLONE_THREAD); threads stay allowed.
        let not_thread = SeccompCondition::new(
            0,
            SeccompCmpArgLen::Qword,
            SeccompCmpOp::MaskedEq(libc::CLONE_THREAD as u64),
            0,
        )
        .map_err(|e| e.to_string())?;
        deny.insert(
            libc::SYS_clone,
            vec![SeccompRule::new(vec![not_thread]).map_err(|e| e.to_string())?],
        );
        apply(deny, libc::EPERM)?;

        // clone3 passes its flags in memory, out of seccomp's reach. Make it
        // look unimplemented so libc falls back to clone, which is checked.
        let mut enosys = BTreeMap::new();
        enosys.insert(libc::SYS_clone3, Vec::new());
        apply(enosys, libc::ENOSYS)
    }
}
