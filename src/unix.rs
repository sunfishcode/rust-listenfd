use rustix::fd::{AsRawFd, FromFd, OwnedFd};
use rustix::fs::{FdFlags, FileType};
use rustix::net::{AddressFamily, SocketType};
use std::env;
use std::io;
use std::net::{TcpListener, UdpSocket};
use std::os::unix::io::FromRawFd;
use std::os::unix::net::{UnixDatagram, UnixListener};

pub type FdType = OwnedFd;

fn is_sock(fd: &FdType) -> bool {
    rustix::fs::fstat(fd)
        .map(|stat| FileType::from_raw_mode(stat.st_mode) == FileType::Socket)
        .unwrap_or(false)
}

fn validate_socket(
    fd: FdType,
    sock_fam: AddressFamily,
    sock_type: SocketType,
    hint: &str,
) -> Result<FdType, (io::Error, FdType)> {
    if !is_sock(&fd) {
        return Err((
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("fd {:?} is not a socket", fd),
            ),
            fd,
        ));
    }

    let is_valid = rustix::net::getsockname(&fd)
        .map(|sockaddr| {
            rustix::net::sockopt::get_socket_type(&fd) == Ok(sock_type)
                && (sockaddr.address_family() == sock_fam
                    || (sockaddr.address_family() == AddressFamily::INET6
                        && sock_fam == AddressFamily::INET))
        })
        .unwrap_or(false);

    if !is_valid {
        return Err((
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("fd {:?} is not a valid {}", fd, hint),
            ),
            fd,
        ));
    }

    Ok(fd)
}

fn mark_cloexec(fd: FdType) -> Result<FdType, (io::Error, FdType)> {
    match rustix::fs::fcntl_setfd(&fd, FdFlags::CLOEXEC) {
        Ok(()) => Ok(fd),
        Err(err) => Err((
            io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "fd {} cannot be marked as FD_CLOEXEC ({:?})",
                    fd.as_raw_fd(),
                    err
                ),
            ),
            fd,
        )),
    }
}

pub fn make_tcp_listener(fd: FdType) -> Result<TcpListener, (io::Error, FdType)> {
    validate_socket(
        fd,
        AddressFamily::INET,
        SocketType::STREAM,
        "unix stream socket",
    )
    .and_then(mark_cloexec)
    .map(|fd| FromFd::from_fd(fd))
}

pub fn make_unix_listener(fd: FdType) -> Result<UnixListener, (io::Error, FdType)> {
    let fd = validate_socket(fd, AddressFamily::UNIX, SocketType::STREAM, "unix socket")?;
    Ok(FromFd::from_fd(mark_cloexec(fd)?))
}

pub fn make_udp_socket(fd: FdType) -> Result<UdpSocket, (io::Error, FdType)> {
    let fd = validate_socket(fd, AddressFamily::INET, SocketType::DGRAM, "udp socket")?;
    Ok(FromFd::from_fd(mark_cloexec(fd)?))
}

pub fn make_unix_datagram(fd: FdType) -> Result<UnixDatagram, (io::Error, FdType)> {
    validate_socket(
        fd,
        AddressFamily::UNIX,
        SocketType::DGRAM,
        "unix datagram socket",
    )
    .and_then(mark_cloexec)
    .map(|fd| FromFd::from_fd(fd))
}

pub fn make_custom<T: FromFd>(
    fd: FdType,
    sock_fam: AddressFamily,
    sock_type: SocketType,
    hint: &str,
) -> Result<T, (io::Error, FdType)> {
    validate_socket(fd, sock_fam, sock_type, hint)
        .and_then(mark_cloexec)
        .map(|fd| FromFd::from_fd(fd))
}

pub fn get_fds() -> Option<Vec<FdType>> {
    // modified systemd protocol
    if let Some(count) = env::var("LISTEN_FDS").ok().and_then(|x| x.parse().ok()) {
        let ok = match env::var("LISTEN_PID").as_ref().map(|x| x.as_str()) {
            Err(env::VarError::NotPresent) | Ok("") => true,
            Ok(val) if val.parse().ok() == Some(rustix::process::getpid().as_raw_nonzero()) => true,
            _ => false,
        };

        env::remove_var("LISTEN_PID");
        env::remove_var("LISTEN_FDS");
        if ok {
            return Some(
                (0..count)
                    .map(|offset| unsafe { OwnedFd::from_raw_fd(3 + offset) })
                    .collect(),
            );
        }
    }

    None
}
