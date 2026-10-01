//! Static system information for the About page.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AboutInfo {
    pub os_name: String,
    pub os_version: String,
    pub kernel: String,
    pub hostname: String,
    pub cpu: String,
    pub gpu: String,
    pub ram_total_kb: u64,
    pub compositor_version: String,
    pub edex_version: String,
    pub uptime_secs: u64,
}

pub fn query(gpu: Option<String>, compositor_version: String) -> AboutInfo {
    let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let field = |k: &str| {
        os_release
            .lines()
            .find_map(|l| {
                l.strip_prefix(&format!("{k}="))
                    .map(|v| v.trim_matches('"').to_string())
            })
            .unwrap_or_default()
    };
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|c| {
            c.lines().find_map(|l| {
                l.strip_prefix("model name")
                    .map(|v| v.trim_start_matches([' ', '\t', ':']).trim().to_string())
            })
        })
        .unwrap_or_default();
    let mem = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|m| {
            m.lines().find_map(|l| {
                l.strip_prefix("MemTotal:")
                    .and_then(|v| v.split_whitespace().next().and_then(|n| n.parse().ok()))
            })
        })
        .unwrap_or(0);
    AboutInfo {
        os_name: field("PRETTY_NAME"),
        os_version: field("VERSION_ID"),
        kernel: std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        hostname: std::fs::read_to_string("/etc/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        cpu,
        gpu: gpu.unwrap_or_else(|| "unknown".into()),
        ram_total_kb: mem,
        compositor_version,
        edex_version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|u| {
                u.split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<f64>().ok())
            })
            .unwrap_or(0.0) as u64,
    }
}
