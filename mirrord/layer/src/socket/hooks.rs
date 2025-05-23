use alloc::ffi::CString;
use core::{cmp, ffi::CStr};
use std::{
    collections::HashSet,
    os::unix::io::RawFd,
    sync::{LazyLock, Mutex},
};

use libc::{c_char, c_int, c_void, hostent, size_t, sockaddr, socklen_t, ssize_t};
use mirrord_config::experimental::ExperimentalConfig;
use mirrord_layer_macro::{hook_fn, hook_guard_fn};
use nix::errno::Errno;

#[cfg(target_os = "macos")]
use super::apple_dnsinfo::*;
use super::ops::*;
use crate::{detour::DetourGuard, hooks::HookManager, replace};

/// Here we keep addr infos that we allocated so we'll know when to use the original
/// freeaddrinfo function and when to use our implementation
pub(crate) static MANAGED_ADDRINFO: LazyLock<Mutex<HashSet<usize>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

#[hook_guard_fn]
pub(crate) unsafe extern "C" fn socket_detour(
    domain: c_int,
    type_: c_int,
    protocol: c_int,
) -> c_int {
    socket(domain, type_, protocol).unwrap_or_bypass_with(|_| FN_SOCKET(domain, type_, protocol))
}

#[hook_guard_fn]
pub(crate) unsafe extern "C" fn bind_detour(
    sockfd: c_int,
    raw_address: *const sockaddr,
    address_length: socklen_t,
) -> c_int {
    bind(sockfd, raw_address, address_length)
        .unwrap_or_bypass_with(|_| FN_BIND(sockfd, raw_address, address_length))
}

#[hook_guard_fn]
pub(crate) unsafe extern "C" fn listen_detour(sockfd: RawFd, backlog: c_int) -> c_int {
    listen(sockfd, backlog).unwrap_or_bypass_with(|_| FN_LISTEN(sockfd, backlog))
}

#[hook_guard_fn]
pub(crate) unsafe extern "C" fn connect_detour(
    sockfd: RawFd,
    raw_address: *const sockaddr,
    address_length: socklen_t,
) -> c_int {
    connect(sockfd, raw_address, address_length)
        .map(From::from)
        .unwrap_or_bypass_with(|_| FN_CONNECT(sockfd, raw_address, address_length))
}

/// Hook for `_connect$NOCANCEL` (for macos, see
/// [this](https://opensource.apple.com/source/xnu/xnu-4570.41.2/libsyscall/Platforms/MacOSX/x86_64/syscall.map.auto.html)).
#[hook_guard_fn]
pub(super) unsafe extern "C" fn _connect_nocancel_detour(
    sockfd: RawFd,
    raw_address: *const sockaddr,
    address_length: socklen_t,
) -> c_int {
    connect(sockfd, raw_address, address_length)
        .map(From::from)
        .unwrap_or_bypass_with(|_| FN__CONNECT_NOCANCEL(sockfd, raw_address, address_length))
}

#[hook_guard_fn]
pub(super) unsafe extern "C" fn getpeername_detour(
    sockfd: RawFd,
    address: *mut sockaddr,
    address_len: *mut socklen_t,
) -> c_int {
    getpeername(sockfd, address, address_len)
        .unwrap_or_bypass_with(|_| FN_GETPEERNAME(sockfd, address, address_len))
}

#[hook_guard_fn]
pub(crate) unsafe extern "C" fn getsockname_detour(
    sockfd: RawFd,
    address: *mut sockaddr,
    address_len: *mut socklen_t,
) -> c_int {
    getsockname(sockfd, address, address_len)
        .unwrap_or_bypass_with(|_| FN_GETSOCKNAME(sockfd, address, address_len))
}

/// Hook for `libc::gethostname`.
///
/// Reads remote hostname bytes into `raw_name`, will rais EINVAL errno and return -1 if hostname
/// read more than `name_length`
#[hook_guard_fn]
pub(crate) unsafe extern "C" fn gethostname_detour(
    raw_name: *mut c_char,
    name_length: usize,
) -> c_int {
    gethostname()
        .map(|host| {
            let host_len = host.as_bytes_with_nul().len();
            raw_name.copy_from_nonoverlapping(host.as_ptr(), cmp::min(name_length, host_len));

            if host_len > name_length {
                Errno::EINVAL.set();

                -1
            } else {
                0
            }
        })
        .unwrap_or_bypass_with(|_| FN_GETHOSTNAME(raw_name, name_length))
}

/// Hook for `libc::gethostbyname` (you won't find this in rust's `libc` as it's been deprecated and
/// removed).
///
/// Resolves DNS `raw_name` and allocates a `static` [`libc::hostent`] that we change the
/// inner values whenever this function is called. The address itself of `*mut hostent` has to
/// remain the same (thus why it's a `static`).
#[hook_guard_fn]
unsafe extern "C" fn gethostbyname_detour(raw_name: *const c_char) -> *mut hostent {
    let rawish_name = (!raw_name.is_null()).then(|| CStr::from_ptr(raw_name));
    gethostbyname(rawish_name).unwrap_or_bypass_with(|_| FN_GETHOSTBYNAME(raw_name))
}

#[hook_guard_fn]
pub(crate) unsafe extern "C" fn accept_detour(
    sockfd: c_int,
    address: *mut sockaddr,
    address_len: *mut socklen_t,
) -> c_int {
    let accept_result = FN_ACCEPT(sockfd, address, address_len);

    if accept_result == -1 {
        accept_result
    } else {
        accept(sockfd, address, address_len, accept_result).unwrap_or_bypass(accept_result)
    }
}

#[cfg(target_os = "linux")]
#[hook_guard_fn]
pub(crate) unsafe extern "C" fn accept4_detour(
    sockfd: c_int,
    address: *mut sockaddr,
    address_len: *mut socklen_t,
    flags: c_int,
) -> c_int {
    let accept_result = FN_ACCEPT4(sockfd, address, address_len, flags);

    if accept_result == -1 {
        accept_result
    } else {
        accept(sockfd, address, address_len, accept_result).unwrap_or_bypass(accept_result)
    }
}

#[cfg(target_os = "linux")]
#[hook_guard_fn]
#[allow(non_snake_case)]
pub(super) unsafe extern "C" fn uv__accept4_detour(
    sockfd: c_int,
    address: *mut sockaddr,
    address_len: *mut socklen_t,
    flags: c_int,
) -> c_int {
    tracing::trace!("uv__accept4_detour -> sockfd {:#?}", sockfd);

    accept4_detour(sockfd, address, address_len, flags)
}

/// Hook for `_accept$NOCANCEL` (for macos, see
/// [this](https://opensource.apple.com/source/xnu/xnu-4570.41.2/libsyscall/Platforms/MacOSX/x86_64/syscall.map.auto.html)).
#[hook_guard_fn]
pub(super) unsafe extern "C" fn _accept_nocancel_detour(
    sockfd: c_int,
    address: *mut sockaddr,
    address_len: *mut socklen_t,
) -> c_int {
    let accept_result = FN__ACCEPT_NOCANCEL(sockfd, address, address_len);

    if accept_result == -1 {
        accept_result
    } else {
        accept(sockfd, address, address_len, accept_result).unwrap_or_bypass(accept_result)
    }
}

/// <https://github.com/metalbear-co/mirrord/issues/184>
#[hook_fn]
pub(crate) unsafe extern "C" fn fcntl_detour(fd: c_int, cmd: c_int, mut arg: ...) -> c_int {
    let arg = arg.arg::<usize>();
    let fcntl_result = FN_FCNTL(fd, cmd, arg);
    let guard = DetourGuard::new();
    if guard.is_none() {
        return fcntl_result;
    }

    if fcntl_result == -1 {
        fcntl_result
    } else {
        match fcntl(fd, cmd, fcntl_result) {
            Ok(()) => fcntl_result,
            Err(e) => e.into(),
        }
    }
}

#[hook_guard_fn]
pub(super) unsafe extern "C" fn dup_detour(fd: c_int) -> c_int {
    let dup_result = FN_DUP(fd);

    if dup_result == -1 {
        dup_result
    } else {
        match dup::<false>(fd, dup_result) {
            Ok(()) => dup_result,
            Err(e) => e.into(),
        }
    }
}

#[hook_guard_fn]
pub(super) unsafe extern "C" fn dup2_detour(oldfd: c_int, newfd: c_int) -> c_int {
    if oldfd == newfd {
        return newfd;
    }

    let dup2_result = FN_DUP2(oldfd, newfd);

    if dup2_result == -1 {
        dup2_result
    } else {
        match dup::<true>(oldfd, dup2_result) {
            Ok(()) => dup2_result,
            Err(e) => e.into(),
        }
    }
}

#[cfg(target_os = "linux")]
#[hook_guard_fn]
pub(super) unsafe extern "C" fn dup3_detour(oldfd: c_int, newfd: c_int, flags: c_int) -> c_int {
    let dup3_result = FN_DUP3(oldfd, newfd, flags);

    if dup3_result == -1 {
        dup3_result
    } else {
        match dup::<true>(oldfd, dup3_result) {
            Ok(()) => dup3_result,
            Err(e) => e.into(),
        }
    }
}

/// Turns the raw pointer parameters into Rust types and calls `ops::getaddrinfo`.
///
/// # Warning:
/// - `raw_hostname`, `raw_servname`, and/or `raw_hints` might be null!
#[hook_guard_fn]
unsafe extern "C" fn getaddrinfo_detour(
    raw_node: *const c_char,
    raw_service: *const c_char,
    raw_hints: *const libc::addrinfo,
    out_addr_info: *mut *mut libc::addrinfo,
) -> c_int {
    let rawish_node = (!raw_node.is_null()).then(|| CStr::from_ptr(raw_node));
    let rawish_service = (!raw_service.is_null()).then(|| CStr::from_ptr(raw_service));
    let rawish_hints = raw_hints.as_ref();

    getaddrinfo(rawish_node, rawish_service, rawish_hints)
        .map(|c_addr_info_ptr| {
            out_addr_info.copy_from_nonoverlapping(&c_addr_info_ptr, 1);
            MANAGED_ADDRINFO
                .lock()
                .expect("MANAGED_ADDRINFO lock failed")
                .insert(c_addr_info_ptr as usize);
            0
        })
        .unwrap_or_bypass_with(|_| FN_GETADDRINFO(raw_node, raw_service, raw_hints, out_addr_info))
}

/// Deallocates a `*mut libc::addrinfo` that was previously allocated with `Box::new` in
/// `getaddrinfo_detour` and converted into a raw pointer by `Box::into_raw`. Same thing must also
/// be done for `addrinfo.ai_addr`.
///
/// Also follows the `addr_info.ai_next` pointer, deallocating the next pointers in the linked list.
///
/// # Protocol
///
/// No need to send any sort of `free` message to `mirrord-agent`, as the `addrinfo` there is not
/// kept around.
///
/// # Warning
///
/// The `addrinfo` pointer has to be allocated respecting the `Box`'s
/// [memory layout](https://doc.rust-lang.org/std/boxed/index.html#memory-layout).
///
/// This needs to support trimmed linked lists, but at the moment if someone does that
/// it will call the original freeaddrinfo which might cause UB or crash.
/// if crashes occur on getaddrinfo - check this case.
/// This can be solved probably by adding each pointer in the linked list to our HashSet.
#[hook_guard_fn]
unsafe extern "C" fn freeaddrinfo_detour(addrinfo: *mut libc::addrinfo) {
    let mut managed_addr_info = MANAGED_ADDRINFO
        .lock()
        .expect("MANAGED_ADDRINFO lock failed");
    if managed_addr_info.remove(&(addrinfo as usize)) {
        // Iterate over `addrinfo` linked list dropping it.
        let mut current = addrinfo;
        while !current.is_null() {
            let current_box = Box::from_raw(current);
            let ai_addr = Box::from_raw(current_box.ai_addr);
            let ai_canonname = CString::from_raw(current_box.ai_canonname);

            current = (*current).ai_next;

            drop(ai_addr);
            drop(ai_canonname);
            drop(current_box);
            managed_addr_info.remove(&(current as usize));
        }
    } else {
        FN_FREEADDRINFO(addrinfo);
    }
}

/// Not a faithful reproduction of what [`libc::recvmsg`] is supposed to do, see [`recv_from`].
#[hook_guard_fn]
pub(super) unsafe extern "C" fn recv_from_detour(
    sockfd: i32,
    out_buffer: *mut c_void,
    buffer_length: size_t,
    flags: c_int,
    raw_source: *mut sockaddr,
    source_length: *mut socklen_t,
) -> ssize_t {
    // Equivalent to just calling `recv`.
    if raw_source.is_null() {
        libc::recv(sockfd, out_buffer, buffer_length, flags)
    } else {
        let recv_from_result = unsafe {
            FN_RECV_FROM(
                sockfd,
                out_buffer,
                buffer_length,
                flags,
                raw_source,
                source_length,
            )
        };

        if recv_from_result == -1 {
            recv_from_result
        } else {
            recv_from(sockfd, recv_from_result, raw_source, source_length)
                .unwrap_or_bypass(recv_from_result)
        }
    }
}

/// Not a faithful reproduction of what [`libc::sendto`] is supposed to do, see [`send_to`].
#[hook_guard_fn]
pub(super) unsafe extern "C" fn send_to_detour(
    sockfd: RawFd,
    raw_message: *const c_void,
    message_length: size_t,
    flags: c_int,
    raw_destination: *const sockaddr,
    destination_length: socklen_t,
) -> ssize_t {
    // Equivalent to just calling `send`.
    if raw_destination.is_null() {
        libc::send(sockfd, raw_message, message_length, flags)
    } else {
        send_to(
            sockfd,
            raw_message,
            message_length,
            flags,
            raw_destination,
            destination_length,
        )
        .unwrap_or_bypass_with(|_| {
            FN_SEND_TO(
                sockfd,
                raw_message,
                message_length,
                flags,
                raw_destination,
                destination_length,
            )
        })
    }
}

/// Not a faithful reproduction of what [`libc::recvmsg`] is supposed to do, see [`recv_from`].
///
/// TODO(alex): We are ignoring the control message header [`libc::cmsghdr`].
#[hook_guard_fn]
pub(super) unsafe extern "C" fn recvmsg_detour(
    sockfd: i32,
    message_header: *mut libc::msghdr,
    flags: c_int,
) -> ssize_t {
    let recvmsg_result = FN_RECVMSG(sockfd, message_header, flags);

    if recvmsg_result == -1 {
        recvmsg_result
    } else {
        // Fills the address, similar to how `recv_from` works.
        recv_from(
            sockfd,
            recvmsg_result,
            (*message_header).msg_name as *mut _,
            &mut (*message_header).msg_namelen,
        )
        .unwrap_or_bypass(recvmsg_result)
    }
}

/// Not a faithful reproduction of what [`libc::sendmsg`] is supposed to do, see [`sendmsg`].
//
// TODO(alex): We are ignoring the control message header `libc::cmsghdr`.
#[hook_guard_fn]
pub(super) unsafe extern "C" fn sendmsg_detour(
    sockfd: RawFd,
    message_header: *const libc::msghdr,
    flags: c_int,
) -> ssize_t {
    // When the whole header is null, the operation happens, but does basically nothing (afaik).
    //
    // If you ever hit an issue with this, maybe null here is meant to `libc::send` a 0-sized
    // message?
    //
    // When `msg_name` is null, this is equivalent to `send`.
    if message_header.is_null() || (*message_header).msg_name.is_null() {
        FN_SENDMSG(sockfd, message_header, flags)
    } else {
        sendmsg(sockfd, message_header, flags)
            .unwrap_or_bypass_with(|_| FN_SENDMSG(sockfd, message_header, flags))
    }
}

/// Not a faithful reproduction of what [`FN_DNS_CONFIGURATION_COPY`] is supposed to do, see
/// [`remote_dns_configuration_copy`].
#[cfg(target_os = "macos")]
#[hook_guard_fn]
unsafe extern "C" fn dns_configuration_copy_detour() -> *mut dns_config_t {
    remote_dns_configuration_copy().unwrap_or_bypass_with(|_| {
        Box::into_raw(Box::new(dns_config_t {
            n_resolver: 0,
            resolver: std::ptr::null_mut(),
            n_scoped_resolver: 0,
            scoped_resolver: std::ptr::null_mut(),
            reserved: [0; 5],
        }))
    })
}

/// Because we create our pointers with boxes and not alloc ourselfs the easies way to safely
/// drop everthing is just to recreate back the boxes that we casted into C structs as part of
/// [`dns_configuration_copy_detour`]
#[cfg(target_os = "macos")]
#[hook_guard_fn]
unsafe extern "C" fn dns_configuration_free_detour(config: *mut dns_config_t) {
    // It should drop it automatically after recreating the boxes and vecs

    let config = Box::from_raw(config);

    Vec::from_raw_parts(config.resolver, config.n_resolver as usize, 0)
        .into_iter()
        .for_each(|resolver| free_dns_resolver_t(resolver));
}

#[hook_guard_fn]
pub(crate) unsafe extern "C" fn getifaddrs_detour(ifaddrs: *mut *mut libc::ifaddrs) -> c_int {
    match getifaddrs() {
        Ok(got_ifaddrs) => {
            *ifaddrs = got_ifaddrs;
            0
        }
        Err(error) => error.into(),
    }
}

#[cfg(target_os = "macos")]
#[allow(non_snake_case)]
#[hook_guard_fn]
unsafe extern "C-unwind" fn CFNetworkCopySystemProxySettings_detour(
) -> Option<objc2_core_foundation::CFRetained<objc2_core_foundation::CFDictionary>> {
    None
}

pub(crate) unsafe fn enable_socket_hooks(
    hook_manager: &mut HookManager,
    enabled_remote_dns: bool,
    experimental: &ExperimentalConfig,
) {
    replace!(hook_manager, "socket", socket_detour, FnSocket, FN_SOCKET);

    replace!(
        hook_manager,
        "recvfrom",
        recv_from_detour,
        FnRecv_from,
        FN_RECV_FROM
    );
    replace!(
        hook_manager,
        "sendto",
        send_to_detour,
        FnSend_to,
        FN_SEND_TO
    );
    replace!(
        hook_manager,
        "recvmsg",
        recvmsg_detour,
        FnRecvmsg,
        FN_RECVMSG
    );
    replace!(
        hook_manager,
        "sendmsg",
        sendmsg_detour,
        FnSendmsg,
        FN_SENDMSG
    );

    replace!(hook_manager, "bind", bind_detour, FnBind, FN_BIND);
    replace!(hook_manager, "listen", listen_detour, FnListen, FN_LISTEN);

    replace!(
        hook_manager,
        "connect",
        connect_detour,
        FnConnect,
        FN_CONNECT
    );
    replace!(
        hook_manager,
        "_connect$NOCANCEL",
        _connect_nocancel_detour,
        Fn_connect_nocancel,
        FN__CONNECT_NOCANCEL
    );

    replace!(hook_manager, "fcntl", fcntl_detour, FnFcntl, FN_FCNTL);
    replace!(hook_manager, "dup", dup_detour, FnDup, FN_DUP);
    replace!(hook_manager, "dup2", dup2_detour, FnDup2, FN_DUP2);

    replace!(
        hook_manager,
        "getpeername",
        getpeername_detour,
        FnGetpeername,
        FN_GETPEERNAME
    );

    replace!(
        hook_manager,
        "getsockname",
        getsockname_detour,
        FnGetsockname,
        FN_GETSOCKNAME
    );

    replace!(
        hook_manager,
        "gethostname",
        gethostname_detour,
        FnGethostname,
        FN_GETHOSTNAME
    );

    #[cfg(target_os = "linux")]
    {
        // Here we replace a function of libuv and not libc, so we pass None as the .
        replace!(
            hook_manager,
            "uv__accept4",
            uv__accept4_detour,
            FnUv__accept4,
            FN_UV__ACCEPT4
        );

        replace!(
            hook_manager,
            "accept4",
            accept4_detour,
            FnAccept4,
            FN_ACCEPT4
        );

        replace!(hook_manager, "dup3", dup3_detour, FnDup3, FN_DUP3);
    }

    replace!(hook_manager, "accept", accept_detour, FnAccept, FN_ACCEPT);
    replace!(
        hook_manager,
        "_accept$NOCANCEL",
        _accept_nocancel_detour,
        Fn_accept_nocancel,
        FN__ACCEPT_NOCANCEL
    );

    if enabled_remote_dns {
        replace!(
            hook_manager,
            "gethostbyname",
            gethostbyname_detour,
            FnGethostbyname,
            FN_GETHOSTBYNAME
        );

        replace!(
            hook_manager,
            "getaddrinfo",
            getaddrinfo_detour,
            FnGetaddrinfo,
            FN_GETADDRINFO
        );

        replace!(
            hook_manager,
            "freeaddrinfo",
            freeaddrinfo_detour,
            FnFreeaddrinfo,
            FN_FREEADDRINFO
        );
        #[cfg(target_os = "macos")]
        {
            replace!(
                hook_manager,
                "dns_configuration_copy",
                dns_configuration_copy_detour,
                FnDns_configuration_copy,
                FN_DNS_CONFIGURATION_COPY
            );
            replace!(
                hook_manager,
                "dns_configuration_free",
                dns_configuration_free_detour,
                FnDns_configuration_free,
                FN_DNS_CONFIGURATION_FREE
            );
            if experimental.ignore_system_proxy_config {
                replace!(
                    hook_manager,
                    "CFNetworkCopySystemProxySettings",
                    CFNetworkCopySystemProxySettings_detour,
                    FnCFNetworkCopySystemProxySettings,
                    FN_CFNETWORKCOPYSYSTEMPROXYSETTINGS
                )
            }
        }
    }

    if experimental.hide_ipv6_interfaces {
        replace!(
            hook_manager,
            "getifaddrs",
            getifaddrs_detour,
            FnGetifaddrs,
            FN_GETIFADDRS
        );
    }
}
