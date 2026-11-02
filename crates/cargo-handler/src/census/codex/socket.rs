//! Current Unix socket relations between Codex processes.

use std::collections::HashMap;
use std::collections::HashSet;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::io::ErrorKind;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::os::fd::FromRawFd;
#[cfg(target_os = "linux")]
use std::os::fd::OwnedFd;
use std::path::Path;

#[cfg(target_os = "linux")]
use libc::AF_NETLINK;
#[cfg(target_os = "linux")]
use libc::AF_UNIX;
#[cfg(target_os = "linux")]
use libc::ENOENT;
#[cfg(target_os = "linux")]
use libc::NETLINK_SOCK_DIAG;
#[cfg(target_os = "linux")]
use libc::NLM_F_REQUEST;
#[cfg(target_os = "linux")]
use libc::SOCK_CLOEXEC;
#[cfg(target_os = "linux")]
use libc::SOCK_RAW;
#[cfg(target_os = "macos")]
use libproc::bsd_info::BSDInfo;
#[cfg(target_os = "macos")]
use libproc::file_info::ListFDs;
#[cfg(target_os = "macos")]
use libproc::file_info::ProcFDType;
#[cfg(target_os = "macos")]
use libproc::file_info::pidfdinfo;
#[cfg(target_os = "macos")]
use libproc::net_info::SocketFDInfo;
#[cfg(target_os = "macos")]
use libproc::net_info::SocketInfoKind;
#[cfg(target_os = "macos")]
use libproc::proc_pid::listpidinfo;
#[cfg(target_os = "macos")]
use libproc::proc_pid::pidinfo;

use super::ProcessEntry;
use super::session_home;
#[cfg(target_os = "linux")]
use crate::constants::CODEX_SOCK_DIAG_BY_FAMILY;
#[cfg(target_os = "linux")]
use crate::constants::CODEX_UNIX_DIAG_SHOW_PEER;

/// A peer read can fail without proving that a connection is absent.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug)]
enum SocketPeer {
    Connected(u64),
    NoPeer,
    Unreadable,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::census) struct SocketEndpoint {
    pub(in crate::census) pid:    u32,
    pub(in crate::census) socket: u64,
    pub(in crate::census) peer:   u64,
}

/// Only interactive TUI clients determine daemon ownership.
#[derive(Debug)]
enum DaemonClients {
    Sole(u32),
    Shared(Vec<u32>),
}

impl DaemonClients {
    fn insert(&mut self, client: u32) {
        match self {
            Self::Sole(pid) if *pid != client => *self = Self::Shared(vec![*pid, client]),
            Self::Shared(pids) if !pids.contains(&client) => pids.push(client),
            Self::Sole(_) | Self::Shared(_) => {},
        }
    }
}

/// Only a TUI's exclusive child daemon gives an exact process join.
#[derive(Clone, Copy, Debug)]
pub(super) enum ClientDaemonLink {
    NoDaemon,
    OwnChild,
    Unresolved,
}

/// Current socket ownership. Neither logs nor thread rows duplicate this fact.
#[derive(Debug, Default)]
pub(super) struct SocketSnapshot {
    daemons:  HashMap<u32, DaemonClients>,
    unjoined: HashSet<u32>,
    #[cfg(target_os = "linux")]
    peers:    HashMap<u64, SocketPeer>,
    #[cfg(target_os = "linux")]
    observed: HashSet<u64>,
    queries:  usize,
}

impl SocketSnapshot {
    pub(super) fn clients(&self, daemon: u32) -> &[u32] {
        match self.daemons.get(&daemon) {
            Some(DaemonClients::Sole(pid)) => std::slice::from_ref(pid),
            Some(DaemonClients::Shared(pids)) => pids,
            None => &[],
        }
    }

    pub(super) fn relation(
        &self,
        client: &ProcessEntry,
        processes: &[ProcessEntry],
    ) -> ClientDaemonLink {
        if self.unjoined.contains(&client.pid) {
            return ClientDaemonLink::Unresolved;
        }
        let mut relation = ClientDaemonLink::NoDaemon;
        for daemon in processes
            .iter()
            .filter(|p| self.clients(p.pid).contains(&client.pid))
        {
            if daemon.parent != Some(client.pid) || self.clients(daemon.pid) != [client.pid] {
                return ClientDaemonLink::Unresolved;
            }
            relation = ClientDaemonLink::OwnChild;
        }
        relation
    }

    pub(super) const fn query_count(&self) -> usize { self.queries }

    pub(super) fn replace(
        &mut self,
        processes: &[ProcessEntry],
        home: Option<&Path>,
        endpoints: &[SocketEndpoint],
        unreadable: &HashSet<u32>,
    ) {
        self.daemons.clear();
        self.unjoined.clear();
        for client in processes
            .iter()
            .filter(|p| p.is_codex() && !p.is_app_server())
        {
            let account = session_home(client, home);
            if account.is_none()
                || processes.iter().any(|p| {
                    unreadable.contains(&p.pid)
                        && p.is_codex()
                        && !p.is_app_server()
                        && session_home(p, home) == account
                })
            {
                self.unjoined.insert(client.pid);
                continue;
            }
            for endpoint in endpoints.iter().filter(|e| e.pid == client.pid) {
                if !endpoints
                    .iter()
                    .any(|e| e.socket == endpoint.peer && e.peer == endpoint.socket)
                {
                    self.unjoined.insert(client.pid);
                }
                for server in endpoints
                    .iter()
                    .filter(|e| e.socket == endpoint.peer && e.peer == endpoint.socket)
                {
                    if processes.iter().any(|p| {
                        p.pid == server.pid
                            && p.is_codex()
                            && p.is_app_server()
                            && session_home(p, home) == account
                    }) {
                        self.daemons
                            .entry(server.pid)
                            .and_modify(|clients| clients.insert(client.pid))
                            .or_insert(DaemonClients::Sole(client.pid));
                    }
                }
            }
        }
    }

    pub(super) fn refresh(&mut self, processes: &[ProcessEntry], home: Option<&Path>) {
        self.queries = 0;
        #[cfg(target_os = "linux")]
        {
            self.observed.clear();
        }
        let mut endpoints = Vec::new();
        let mut unreadable = HashSet::new();
        let table = super::classify::pid_table(processes);
        let clients = super::classify::interactive_codex(&table, processes);
        // Detached connections have no exact join. Their daemon descriptors add no evidence.
        for process in processes.iter().filter(|p| {
            clients.iter().any(|client| client.pid == p.pid)
                || (p.is_codex()
                    && p.is_app_server()
                    && clients.iter().any(|client| p.parent == Some(client.pid)))
        }) {
            #[cfg(target_os = "linux")]
            let sockets = self.process_endpoints(process.pid);
            #[cfg(not(target_os = "linux"))]
            let sockets = Self::process_endpoints(process.pid);
            match sockets {
                Ok(sockets) => endpoints.extend(sockets),
                Err(_) => {
                    unreadable.insert(process.pid);
                },
            }
        }
        #[cfg(target_os = "linux")]
        self.peers
            .retain(|socket, _| self.observed.contains(socket));
        #[cfg(target_os = "linux")]
        self.peers
            .retain(|_, peer| !matches!(peer, SocketPeer::Unreadable));
        self.replace(processes, home, &endpoints, &unreadable);
    }

    #[cfg(target_os = "linux")]
    fn process_endpoints(&mut self, pid: u32) -> std::io::Result<Vec<SocketEndpoint>> {
        let mut endpoints = Vec::new();
        for entry in fs::read_dir(format!("/proc/{pid}/fd"))? {
            let entry = entry?;
            let path = match fs::read_link(entry.path()) {
                Ok(path) => path,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let Some(socket) = path
                .to_str()
                .and_then(|s| s.strip_prefix("socket:["))
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|s| s.parse::<u64>().ok())
            else {
                continue;
            };
            self.observed.insert(socket);
            let peer = *self.peers.entry(socket).or_insert_with(|| {
                self.queries += 1;
                linux_socket_peer(socket)
            });
            match peer {
                SocketPeer::Connected(peer) => endpoints.push(SocketEndpoint { pid, socket, peer }),
                SocketPeer::NoPeer => {},
                SocketPeer::Unreadable => {
                    return Err(std::io::Error::other("socket peer is unreadable"));
                },
            }
        }
        Ok(endpoints)
    }

    #[cfg(target_os = "macos")]
    #[allow(
        unsafe_code,
        reason = "The socket kind selects the libproc union member"
    )]
    fn process_endpoints(pid: u32) -> std::io::Result<Vec<SocketEndpoint>> {
        let native = i32::try_from(pid).map_err(std::io::Error::other)?;
        let info = pidinfo::<BSDInfo>(native, 0).map_err(std::io::Error::other)?;
        let fds = listpidinfo::<ListFDs>(native, info.pbi_nfiles as usize)
            .map_err(std::io::Error::other)?;
        let mut endpoints = Vec::new();
        for fd in fds {
            if !matches!(ProcFDType::from(fd.proc_fdtype), ProcFDType::Socket) {
                continue;
            }
            let socket =
                pidfdinfo::<SocketFDInfo>(native, fd.proc_fd).map_err(std::io::Error::other)?;
            if matches!(
                SocketInfoKind::from(socket.psi.soi_kind),
                SocketInfoKind::Un
            ) {
                // SAFETY: The checked kind makes pri_un the active union member.
                let peer = unsafe { socket.psi.soi_proto.pri_un.unsi_conn_so };
                if peer != 0 {
                    endpoints.push(SocketEndpoint {
                        pid,
                        socket: socket.psi.soi_so,
                        peer,
                    });
                }
            }
        }
        Ok(endpoints)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn process_endpoints(_: u32) -> std::io::Result<Vec<SocketEndpoint>> {
        Err(std::io::Error::other("socket peers are unavailable"))
    }
}

/// Query one Unix socket through `sock_diag`. No child process runs.
#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "The kernel reads bounded byte buffers through the socket ABI"
)]
fn linux_socket_peer(inode: u64) -> SocketPeer {
    let Ok(inode) = u32::try_from(inode) else {
        return SocketPeer::Unreadable;
    };
    // SAFETY: These constants select a kernel socket and require no pointer arguments.
    let descriptor =
        unsafe { libc::socket(AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, NETLINK_SOCK_DIAG) };
    if descriptor < 0 {
        return SocketPeer::Unreadable;
    }
    // SAFETY: socket returned a valid descriptor. OwnedFd receives sole ownership.
    let socket = unsafe { OwnedFd::from_raw_fd(descriptor) };
    let mut request = [0_u8; 40];
    request[0..4].copy_from_slice(&40_u32.to_ne_bytes());
    request[4..6].copy_from_slice(&CODEX_SOCK_DIAG_BY_FAMILY.to_ne_bytes());
    request[6..8].copy_from_slice(
        &u16::try_from(NLM_F_REQUEST)
            .unwrap_or_default()
            .to_ne_bytes(),
    );
    request[16] = u8::try_from(AF_UNIX).unwrap_or_default();
    request[20..24].copy_from_slice(&u32::MAX.to_ne_bytes());
    request[24..28].copy_from_slice(&inode.to_ne_bytes());
    request[28..32].copy_from_slice(&CODEX_UNIX_DIAG_SHOW_PEER.to_ne_bytes()); // UDIAG_SHOW_PEER
    request[32..40].fill(255); // No socket cookie is required.
    // SAFETY: Zero is valid for every field of sockaddr_nl.
    let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    let Ok(family) = libc::sa_family_t::try_from(AF_NETLINK) else {
        return SocketPeer::Unreadable;
    };
    address.nl_family = family;
    let Ok(address_size) = libc::socklen_t::try_from(std::mem::size_of_val(&address)) else {
        return SocketPeer::Unreadable;
    };
    // SAFETY: Both pointers refer to live buffers with the supplied lengths.
    let sent = unsafe {
        libc::sendto(
            socket.as_raw_fd(),
            request.as_ptr().cast(),
            request.len(),
            0,
            (&raw const address).cast(),
            address_size,
        )
    };
    if sent < 0 {
        return SocketPeer::Unreadable;
    }
    let mut answer = [0_u8; 4096];
    // SAFETY: answer is writable for answer.len() bytes, and socket owns the descriptor.
    let received = unsafe {
        libc::recv(
            socket.as_raw_fd(),
            answer.as_mut_ptr().cast(),
            answer.len(),
            0,
        )
    };
    let Ok(length) = usize::try_from(received) else {
        return SocketPeer::Unreadable;
    };
    decode_socket_peer(&answer[..length])
}

#[cfg(target_os = "linux")]
fn decode_socket_peer(answer: &[u8]) -> SocketPeer {
    if answer.len() < 20 {
        return SocketPeer::Unreadable;
    }
    if u16::from_ne_bytes([answer[4], answer[5]]) == 2 {
        let error = i32::from_ne_bytes(answer[16..20].try_into().unwrap_or_default());
        return if error == -ENOENT {
            SocketPeer::NoPeer
        } else {
            SocketPeer::Unreadable
        };
    }
    let mut offset = 32;
    while offset + 4 <= answer.len() {
        let size = usize::from(u16::from_ne_bytes([answer[offset], answer[offset + 1]]));
        let kind = u16::from_ne_bytes([answer[offset + 2], answer[offset + 3]]);
        if size < 4 || offset + size > answer.len() {
            return SocketPeer::Unreadable;
        }
        if kind == 2 && size == 8 {
            // UNIX_DIAG_PEER
            let peer = u32::from_ne_bytes(
                answer[offset + 4..offset + 8]
                    .try_into()
                    .unwrap_or_default(),
            );
            return if peer == 0 {
                SocketPeer::NoPeer
            } else {
                SocketPeer::Connected(u64::from(peer))
            };
        }
        offset += (size + 3) & !3;
    }
    SocketPeer::NoPeer
}

#[cfg(test)]
#[path = "socket_tests.rs"]
mod tests;
