//! Hosts falsos prontos para teste: CachyOS realista, sem cgroup v2 e hostil/truncado.

use crate::sistema::fonte::FonteFalsa;

/// Caminho do cgroup do processo na fixture CachyOS.
pub const CGROUP_CACHYOS: &str =
    "/user.slice/user-1000.slice/user@1000.service/app.slice/app-x.scope";
const R: &str = "/sys/fs/cgroup";

/// O quê: host CachyOS típico (oomd + zram zstd + ananicy, cgroup v2, PSI, 32 GiB).
/// Onde: testes de `identificar` e de `margem_segura`.
pub fn cachyos() -> FonteFalsa {
    let u = "/user.slice/user-1000.slice/user@1000.service";
    let mut f = FonteFalsa::nova()
        .cpus_n(16)
        .arquivo("/etc/os-release", "NAME=\"CachyOS Linux\"\nID=cachyos\nID_LIKE=arch\n")
        .arquivo("/proc/version", "Linux version 6.12.1-1-cachyos (linux-cachyos@cachyos) #1 SMP PREEMPT_DYNAMIC\n")
        .arquivo("/proc/sys/kernel/osrelease", "6.12.1-1-cachyos\n")
        .arquivo("/proc/self/status", "Name:\ttest\nUid:\t1000\t1000\t1000\t1000\n")
        .arquivo("/proc/self/oom_score", "667\n")
        .arquivo("/proc/self/oom_score_adj", "200\n")
        .arquivo("/proc/meminfo", "MemTotal:       32768000 kB\nMemFree:         1000000 kB\nMemAvailable:   16384000 kB\nSwapTotal:      16777212 kB\nSwapFree:       16000000 kB\n")
        .arquivo("/proc/swaps", "Filename\tType\tSize\tUsed\tPriority\n/dev/zram0 partition 16777212 777212 100\n")
        .arquivo("/sys/block/zram0/disksize", "17179869184\n")
        .arquivo("/sys/block/zram0/comp_algorithm", "lzo lzo-rle lz4 lz4hc [zstd]\n")
        .arquivo("/proc/sys/vm/swappiness", "150\n")
        .arquivo("/proc/sys/vm/overcommit_memory", "0\n")
        .arquivo("/proc/sys/vm/min_free_kbytes", "67584\n")
        .arquivo("/proc/sys/vm/page-cluster", "0\n")
        .arquivo("/proc/sys/vm/vfs_cache_pressure", "50\n")
        .arquivo("/proc/sys/vm/watermark_scale_factor", "125\n")
        .arquivo("/proc/sys/vm/oom_kill_allocating_task", "0\n")
        .arquivo("/proc/pressure/memory", "some avg10=0.50 avg60=0.30 avg300=0.10 total=123456\nfull avg10=0.10 avg60=0.05 avg300=0.01 total=2345\n")
        .arquivo("/proc/pressure/cpu", "some avg10=1.00 avg60=1.50 avg300=1.20 total=999999\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n")
        .arquivo("/proc/pressure/io", "some avg10=0.00 avg60=0.10 avg300=0.20 total=555\nfull avg10=0.00 avg60=0.05 avg300=0.10 total=444\n")
        .arquivo("/proc/100/comm", "systemd-oomd\n")
        .arquivo("/proc/200/comm", "ananicy-cpp\n")
        .arquivo("/proc/300/comm", "power-profiles-\n")
        .arquivo("/proc/400/comm", "bash\n")
        .arquivo("/etc/systemd/oomd.conf.d/10-cachy.conf", "[OOM]\nSwapUsedLimit=90%\nDefaultMemoryPressureLimit=60%\nDefaultMemoryPressureDurationSec=20s\n")
        .arquivo("/etc/systemd/system.conf", "[Manager]\n#DefaultMemoryAccounting=yes\nDefaultTasksAccounting=yes\n")
        .arquivo("/usr/lib/systemd/system/user.slice.d/10-oomd.conf", "[Slice]\nManagedOOMMemoryPressure=kill\nManagedOOMMemoryPressureLimit=50%\n")
        .arquivo(&format!("{R}/cgroup.controllers"), "cpuset cpu io memory hugetlb pids rdma misc\n")
        .arquivo("/proc/self/cgroup", &format!("0::{CGROUP_CACHYOS}\n"))
        .arquivo(&format!("{R}{u}/cgroup.controllers"), "cpu memory pids\n")
        .arquivo(&format!("{R}{CGROUP_CACHYOS}/memory.events"), "low 0\nhigh 0\nmax 0\noom 0\noom_kill 0\noom_group_kill 0\n")
        .comando("systemctl show -p ActiveState,SubState systemd-oomd", "ActiveState=active\nSubState=running\n")
        .comando("systemctl show -p ActiveState,SubState ananicy-cpp", "ActiveState=active\nSubState=running\n")
        .comando("systemctl show -p ActiveState,SubState earlyoom", "ActiveState=inactive\nSubState=dead\n")
        .comando("systemctl show -p ActiveState,SubState nohang", "ActiveState=inactive\nSubState=dead\n")
        .comando("systemctl show -p ActiveState,SubState power-profiles-daemon", "ActiveState=active\nSubState=running\n")
        .comando("systemctl show -p ManagedOOMMemoryPressure,ManagedOOMSwap user.slice", "ManagedOOMMemoryPressure=kill\nManagedOOMSwap=auto\n");
    for (rel, max, cur) in [
        (CGROUP_CACHYOS, "8589934592", "1073741824"),
        (&*format!("{u}/app.slice"), "max", "2147483648"),
        (u, "max", "4294967296"),
        ("/user.slice/user-1000.slice", "max", "4294967296"),
        ("/user.slice", "max", "5000000000"),
    ] {
        f = f
            .arquivo(&format!("{R}{rel}/memory.max"), &format!("{max}\n"))
            .arquivo(&format!("{R}{rel}/memory.high"), "max\n")
            .arquivo(&format!("{R}{rel}/memory.swap.max"), "max\n")
            .arquivo(&format!("{R}{rel}/pids.max"), "max\n")
            .arquivo(&format!("{R}{rel}/memory.current"), &format!("{cur}\n"));
    }
    f
}

/// O quê: host sem cgroup v2 (v1 puro), sem PSI, sem swap/zram. Onde: testes de degradação.
pub fn sem_cgroup_v2() -> FonteFalsa {
    FonteFalsa::nova()
        .cpus_n(4)
        .arquivo("/etc/os-release", "ID=debian\n")
        .arquivo("/proc/self/cgroup", "12:memory:/user.slice\n1:name=systemd:/user.slice\n")
        .arquivo("/proc/self/status", "Uid:\t1000\t1000\t1000\t1000\n")
        .arquivo(
            "/proc/meminfo",
            "MemTotal: 8000000 kB\nMemAvailable: 4000000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n",
        )
        .arquivo("/proc/sys/vm/swappiness", "60\n")
}

/// O quê: host com arquivos truncados, não numéricos, unicode e byte nulo. Onde: testes hostis.
pub fn hostil() -> FonteFalsa {
    FonteFalsa::nova()
        .arquivo("/etc/os-release", "ID=\"cachy\u{0}os\u{202e}\"\nID_LIKE=ar\u{0}ch ünï\n")
        .arquivo("/proc/version", "Linux\u{0} version \"quebrado\n")
        .arquivo("/proc/self/status", "Uid:\n")
        .arquivo(
            "/proc/meminfo",
            "MemTotal:\nMemAvailable: abc kB\nSwapTotal: 99999999999999999999999 kB\n",
        )
        .arquivo("/proc/swaps", "Filename\nzram0 partition 12\n\u{0}\u{0}\n")
        .arquivo("/proc/sys/vm/swappiness", "\u{0}\u{0}9\n")
        .arquivo("/proc/sys/vm/overcommit_memory", "-0x2\n")
        .arquivo("/proc/sys/vm/min_free_kbytes", "")
        .arquivo(
            "/proc/pressure/memory",
            "some avg10=NaN avg60=0.0 avg300=0.0 total=1\nfull avg10=inf\n",
        )
        .arquivo("/proc/pressure/cpu", "some avg10=1.0 avg60=2")
        .arquivo("/sys/fs/cgroup/cgroup.controllers", "memory \u{0} pids ünï\n")
        .arquivo("/proc/self/cgroup", "0::/../../etc\n")
        .arquivo("/proc/self/oom_score_adj", "1000000000000000000000\n")
        .arquivo("/proc/123/comm", "\u{0}\n")
        .arquivo("/proc/abc/comm", "earlyoom\n")
        .arquivo(
            "/etc/systemd/oomd.conf",
            "[OOM\nSwapUsedLimit\n=\n[OOM]\nSwapUsedLimit=\u{0}95%\n",
        )
        .arquivo("/sys/block/zram9x/disksize", "1\n")
        .arquivo("/sys/block/zram0/disksize", "lixo\n")
        .arquivo("/sys/block/zram0/comp_algorithm", "[]\n")
        .comando(
            "systemctl show -p ActiveState,SubState systemd-oomd",
            "sem igual\n=\nActiveState=\n",
        )
}
