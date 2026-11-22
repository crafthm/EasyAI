//! Local network discovery.
//!
//! The scan is deliberately credential-optional. Without credentials it probes
//! a handful of common TCP ports and reads the SSH banner, which is enough to
//! find live hosts and guess their operating system. With credentials it logs
//! in over SSH and runs a single read-only command to report the real hostname,
//! OS, CPU model, and core count.
//!
//! Only the current /24 around the primary IPv4 address is scanned, so the
//! whole sweep stays in the few-seconds range.

use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Ports that identify a live host and hint at what it is.
const PROBE_PORTS: &[u16] = &[22, 445, 3389, 5985, 80, 443];
const PROBE_TIMEOUT: Duration = Duration::from_millis(250);
const BANNER_TIMEOUT: Duration = Duration::from_millis(500);
const PROBE_CONCURRENCY: usize = 64;
const SSH_CONCURRENCY: usize = 8;
const SSH_CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const SSH_SESSION_TIMEOUT_MS: u32 = 8000;

/// Credentials entered for one scan. They are never persisted or logged.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshCredentials {
    pub username: String,
    pub password: Option<String>,
    pub key_path: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanDevice {
    pub ip: String,
    pub hostname: Option<String>,
    /// Best-effort OS name. Without SSH this comes from the SSH banner.
    pub os: Option<String>,
    pub arch: Option<String>,
    pub cpu: Option<String>,
    pub cores: Option<u32>,
    pub ports: Vec<u16>,
    /// True when SSH is reachable, so credentials can unlock CPU details.
    pub ssh_ready: bool,
    /// Populated when an SSH login was attempted but failed.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanScanResult {
    pub subnet: String,
    pub scanned: usize,
    pub devices: Vec<LanDevice>,
}

struct HostProbe {
    ports: Vec<u16>,
    banner: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct SshInfo {
    hostname: Option<String>,
    os: Option<String>,
    arch: Option<String>,
    cpu: Option<String>,
    cores: Option<u32>,
}

/// Sweep the current /24 and describe every host that responds.
pub fn scan(credentials: Option<SshCredentials>) -> Result<LanScanResult, String> {
    let local = local_ipv4()?;
    let octets = local.octets();
    let subnet = [octets[0], octets[1], octets[2]];
    let subnet_label = format!("{}.{}.{}.0/24", subnet[0], subnet[1], subnet[2]);

    let ips: Vec<Ipv4Addr> = (1..=254)
        .map(|host| Ipv4Addr::new(subnet[0], subnet[1], subnet[2], host))
        .filter(|ip| *ip != local)
        .collect();

    let probed = probe_all(&ips, ssh_port(&credentials));
    let mut by_ip = probed;
    for ip in neighbor_ips(subnet) {
        if ip == local {
            continue;
        }
        by_ip.entry(ip).or_insert_with(|| HostProbe {
            ports: Vec::new(),
            banner: None,
        });
    }

    let mut targets: Vec<(Ipv4Addr, u16)> = Vec::new();
    let ssh_port = ssh_port(&credentials);
    if let Some(_creds) = credentials
        .as_ref()
        .filter(|creds| !creds.username.trim().is_empty())
    {
        for (ip, probe) in by_ip.iter() {
            if probe.ports.contains(&ssh_port) {
                targets.push((*ip, ssh_port));
            }
        }
    }

    let ssh_results = if targets.is_empty() {
        Vec::new()
    } else {
        ssh_all(
            &targets,
            credentials.as_ref().expect("targets require credentials"),
        )
    };

    let mut devices: Vec<LanDevice> = by_ip
        .iter()
        .map(|(ip, probe)| {
            let ssh = ssh_results
                .iter()
                .find(|(target, _)| target == ip)
                .and_then(|(_, result)| result.as_ref().ok().cloned());
            let note = ssh_results
                .iter()
                .find(|(target, _)| target == ip)
                .and_then(|(_, result)| result.as_ref().err().cloned());
            let info = ssh.unwrap_or_default();
            let os = info
                .os
                .or_else(|| probe.banner.as_deref().and_then(os_from_banner))
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            LanDevice {
                ip: ip.to_string(),
                hostname: info.hostname,
                os,
                arch: info.arch,
                cpu: info.cpu,
                cores: info.cores,
                ports: probe.ports.clone(),
                ssh_ready: probe.ports.contains(&ssh_port),
                note,
            }
        })
        .collect();
    devices.sort_by_key(|device| {
        device
            .ip
            .parse::<Ipv4Addr>()
            .map(u32::from)
            .unwrap_or_default()
    });

    Ok(LanScanResult {
        subnet: subnet_label,
        scanned: ips.len(),
        devices,
    })
}

/// Pick the primary interface's IPv4 address. Connecting a UDP socket sends no
/// packets; it only asks the kernel which route would be used.
fn local_ipv4() -> Result<Ipv4Addr, String> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|err| format!("Failed to open a socket for the network scan: {err}"))?;
    for probe in ["8.8.8.8:80", "1.1.1.1:80"] {
        if socket.connect(probe).is_ok() {
            if let Ok(SocketAddr::V4(addr)) = socket.local_addr() {
                if !addr.ip().is_loopback() && !addr.ip().is_unspecified() {
                    return Ok(*addr.ip());
                }
            }
        }
    }
    Err("Could not determine the local network address.".to_string())
}

fn probe_all(ips: &[Ipv4Addr], extra_port: u16) -> std::collections::BTreeMap<Ipv4Addr, HostProbe> {
    let found = Mutex::new(std::collections::BTreeMap::new());
    let next = AtomicUsize::new(0);

    thread::scope(|scope| {
        for _ in 0..PROBE_CONCURRENCY.min(ips.len().max(1)) {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(ip) = ips.get(index).copied() else {
                    break;
                };
                if let Some(probe) = probe_host(ip, extra_port) {
                    found.lock().unwrap().insert(ip, probe);
                }
            });
        }
    });

    found.into_inner().unwrap()
}

fn ssh_port(credentials: &Option<SshCredentials>) -> u16 {
    credentials
        .as_ref()
        .and_then(|creds| creds.port)
        .unwrap_or(22)
}

fn probe_host(ip: Ipv4Addr, extra_port: u16) -> Option<HostProbe> {
    let mut ports = Vec::new();
    let mut banner = None;

    for &port in PROBE_PORTS.iter().chain(std::iter::once(&extra_port)) {
        if ports.contains(&port) {
            continue;
        }
        if let Ok(mut stream) =
            TcpStream::connect_timeout(&SocketAddr::new(ip.into(), port), PROBE_TIMEOUT)
        {
            ports.push(port);
            if banner.is_none() && (port == 22 || port == extra_port) {
                banner = read_banner(&mut stream);
            }
        }
    }

    if ports.is_empty() && banner.is_none() {
        None
    } else {
        Some(HostProbe { ports, banner })
    }
}

fn read_banner(stream: &mut TcpStream) -> Option<String> {
    stream.set_read_timeout(Some(BANNER_TIMEOUT)).ok()?;
    let mut buffer = [0u8; 255];
    let read = stream.read(&mut buffer).ok()?;
    if read == 0 {
        return None;
    }
    let line = String::from_utf8_lossy(&buffer[..read]);
    Some(line.lines().next().unwrap_or_default().to_string())
}

/// Reads the kernel ARP table on Linux. Hosts that responded at some point but
/// expose no probed port still show up here.
fn neighbor_ips(subnet: [u8; 3]) -> Vec<Ipv4Addr> {
    let Ok(text) = std::fs::read_to_string("/proc/net/arp") else {
        return Vec::new();
    };
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let ip: Ipv4Addr = fields.next()?.parse().ok()?;
            let _hw = fields.next()?;
            let flags = u32::from_str_radix(fields.next()?, 16).ok()?;
            if flags & 0x2 == 0 {
                return None;
            }
            let octets = ip.octets();
            (octets[..3] == subnet).then_some(ip)
        })
        .collect()
}

fn ssh_all(
    targets: &[(Ipv4Addr, u16)],
    credentials: &SshCredentials,
) -> Vec<(Ipv4Addr, Result<SshInfo, String>)> {
    let results = Mutex::new(Vec::with_capacity(targets.len()));
    let next = AtomicUsize::new(0);

    thread::scope(|scope| {
        for _ in 0..SSH_CONCURRENCY.min(targets.len()) {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(&(ip, port)) = targets.get(index) else {
                    break;
                };
                let result = ssh_probe(ip, port, credentials);
                results.lock().unwrap().push((ip, result));
            });
        }
    });

    results.into_inner().unwrap()
}

fn ssh_probe(ip: Ipv4Addr, port: u16, credentials: &SshCredentials) -> Result<SshInfo, String> {
    let stream = TcpStream::connect_timeout(&SocketAddr::new(ip.into(), port), SSH_CONNECT_TIMEOUT)
        .map_err(|err| format!("SSH connection failed: {err}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .map_err(|err| err.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(8)))
        .map_err(|err| err.to_string())?;

    let mut session = ssh2::Session::new().map_err(|err| err.to_string())?;
    session.set_timeout(SSH_SESSION_TIMEOUT_MS);
    session.set_tcp_stream(stream);
    session.handshake().map_err(|err| err.to_string())?;
    verify_host_key(&session, ip, port)?;

    let username = credentials.username.trim();
    let password = credentials
        .password
        .as_deref()
        .filter(|value| !value.is_empty());
    let key_path = credentials
        .key_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(expand_home);

    let authenticated = if let Some(key_path) = key_path {
        session
            .userauth_pubkey_file(username, None, &key_path, password)
            .is_ok()
    } else if let Some(password) = password {
        session.userauth_password(username, password).is_ok()
    } else {
        return Err("No SSH password or private key was provided.".to_string());
    };

    if !authenticated || !session.authenticated() {
        return Err("SSH authentication failed.".to_string());
    }

    let mut channel = session.channel_session().map_err(|err| err.to_string())?;
    channel.exec(REMOTE_INFO).map_err(|err| err.to_string())?;
    let mut output = String::new();
    channel
        .read_to_string(&mut output)
        .map_err(|err| err.to_string())?;
    let _ = channel.wait_close();

    Ok(parse_remote_info(&output))
}

/// Trust-on-first-use against `~/.ssh/known_hosts`, mirroring OpenSSH's
/// `accept-new`: unknown hosts are added, changed keys are refused.
fn verify_host_key(session: &ssh2::Session, ip: Ipv4Addr, port: u16) -> Result<(), String> {
    let (key, key_type) = session
        .host_key()
        .ok_or_else(|| "The host did not present an SSH key.".to_string())?;
    let path = known_hosts_path();
    let mut known = session.known_hosts().map_err(|err| err.to_string())?;
    if path.exists() {
        let _ = known.read_file(&path, ssh2::KnownHostFileKind::OpenSSH);
    }
    let host = ip.to_string();
    match known.check_port(&host, port, key) {
        ssh2::CheckResult::Match => Ok(()),
        ssh2::CheckResult::Mismatch => Err(
            "The host key does not match ~/.ssh/known_hosts; refusing to send credentials."
                .to_string(),
        ),
        ssh2::CheckResult::Failure => Err("The host key could not be verified.".to_string()),
        ssh2::CheckResult::NotFound => {
            known
                .add(&host, key, "easyai", key_type.into())
                .map_err(|err| err.to_string())?;
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            known
                .write_file(&path, ssh2::KnownHostFileKind::OpenSSH)
                .map_err(|err| format!("Failed to update known_hosts: {err}"))
        }
    }
}

fn known_hosts_path() -> PathBuf {
    home_dir().join(".ssh").join("known_hosts")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home_dir().join(rest),
        None => PathBuf::from(path),
    }
}

/// One round trip that works on Linux, macOS, and BSD. Unsupported commands
/// just print nothing, so each section is parsed independently.
const REMOTE_INFO: &str = concat!(
    "echo @@hostname; hostname 2>/dev/null; ",
    "echo @@uname; uname -srm 2>/dev/null; ",
    "echo @@os; cat /etc/os-release 2>/dev/null | grep -E '^(PRETTY_NAME|NAME)='; ",
    "echo @@cores; nproc 2>/dev/null; getconf _NPROCESSORS_ONLN 2>/dev/null; ",
    "sysctl -n hw.ncpu 2>/dev/null; ",
    "echo @@cpu; grep -m1 -i 'model name' /proc/cpuinfo 2>/dev/null; ",
    "lscpu 2>/dev/null | grep -m1 -i 'model name'; ",
    "sysctl -n machdep.cpu.brand_string 2>/dev/null; ",
    "echo @@macos; sw_vers -productName 2>/dev/null; sw_vers -productVersion 2>/dev/null"
);

fn parse_remote_info(output: &str) -> SshInfo {
    let mut info = SshInfo::default();
    let mut section = "";

    for line in output.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("@@") {
            section = name;
            continue;
        }
        if line.is_empty() {
            continue;
        }

        match section {
            "hostname" => {
                if info.hostname.is_none() {
                    info.hostname = Some(line.to_string());
                }
            }
            "uname" => {
                let mut parts = line.split_whitespace();
                if let Some(os) = parts.next() {
                    info.os = Some(match os {
                        "Darwin" => "macOS".to_string(),
                        other => other.to_string(),
                    });
                    let _release = parts.next();
                    if let Some(arch) = parts.next() {
                        info.arch = Some(arch.to_string());
                    }
                }
            }
            "os" => {
                let (key, value) = line.split_once('=').unwrap_or((line, ""));
                let value = value.trim().trim_matches('"');
                if value.is_empty() {
                    continue;
                }
                // PRETTY_NAME is more descriptive than NAME; line order in
                // /etc/os-release puts NAME first, so only let PRETTY_NAME win.
                if key == "PRETTY_NAME" || info.os.is_none() {
                    info.os = Some(value.to_string());
                }
            }
            "cores" => {
                if info.cores.is_none() {
                    info.cores = line.parse().ok();
                }
            }
            "cpu" => {
                let value = line
                    .split_once(':')
                    .map(|(_, value)| value.trim())
                    .unwrap_or(line);
                if !value.is_empty() && !value.eq_ignore_ascii_case("model name") {
                    info.cpu = Some(value.to_string());
                }
            }
            "macos" => {
                // Two lines: product name, then version. `uname` already
                // produced "macOS", so skip a duplicate product name but append
                // the version.
                match info.os.take() {
                    None => info.os = Some(line.to_string()),
                    Some(os) if os == line => info.os = Some(os),
                    Some(os) if os == "macOS" => {
                        info.os = Some(format!("{os} {line}"));
                    }
                    Some(os) => info.os = Some(os),
                }
            }
            _ => {}
        }
    }

    info
}

fn os_from_banner(banner: &str) -> Option<String> {
    const HINTS: &[&str] = &[
        "Ubuntu",
        "Debian",
        "Raspbian",
        "Fedora",
        "CentOS",
        "Rocky",
        "AlmaLinux",
        "openSUSE",
        "FreeBSD",
        "OpenBSD",
        "NetBSD",
        "Windows",
    ];
    HINTS
        .iter()
        .find(|hint| banner.contains(**hint))
        .map(|hint| hint.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_linux_remote_info() {
        let output = "\
@@hostname
build-box
@@uname
Linux 6.8.0-45-generic x86_64
@@os
NAME=\"Ubuntu\"
PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"
@@cores
16
@@cpu
model name\t: AMD Ryzen 9 7950X 16-Core Processor
@@macos
";
        let info = parse_remote_info(output);
        assert_eq!(info.hostname.as_deref(), Some("build-box"));
        assert_eq!(info.os.as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(info.arch.as_deref(), Some("x86_64"));
        assert_eq!(info.cores, Some(16));
        assert_eq!(
            info.cpu.as_deref(),
            Some("AMD Ryzen 9 7950X 16-Core Processor")
        );
    }

    #[test]
    fn parses_macos_remote_info() {
        let output = "\
@@hostname
mac-mini
@@uname
Darwin 23.6.0 arm64
@@os
@@cores
10
@@cpu
Apple M2 Pro
@@macos
macOS
14.6.1
";
        let info = parse_remote_info(output);
        assert_eq!(info.os.as_deref(), Some("macOS 14.6.1"));
        assert_eq!(info.cpu.as_deref(), Some("Apple M2 Pro"));
        assert_eq!(info.arch.as_deref(), Some("arm64"));
    }

    #[test]
    fn reads_os_hints_from_ssh_banners() {
        let banner = "SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.5";
        assert_eq!(os_from_banner(banner).as_deref(), Some("Ubuntu"));
    }
}
