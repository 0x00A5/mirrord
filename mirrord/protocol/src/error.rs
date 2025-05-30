use std::{
    fmt::{self, Formatter},
    io,
    net::AddrParseError,
    path::StripPrefixError,
    sync::LazyLock,
};

use bincode::{Decode, Encode};
use semver::VersionReq;
use thiserror::Error;

use crate::{
    outgoing::SocketAddress,
    tcp::{Filter, HttpFilter, StealType},
    Port,
};

#[derive(Encode, Decode, Debug, PartialEq, Clone, Eq, Error)]
pub enum SerializationError {
    #[error("Could not convert the socket address into a supported owned address type.")]
    SocketAddress,
}

#[derive(Encode, Decode, Debug, PartialEq, Clone, Eq, Error)]
pub enum ResponseError {
    #[error("File/connection ids exhausted, operation `{0}` failed!")]
    IdsExhausted(String),

    #[error("Failed to find resource `{0}`!")]
    NotFound(u64),

    #[error("Remote operation expected fd `{0}` to be a directory, but it's a file!")]
    NotDirectory(u64),

    #[error("Remote operation expected fd `{0}` to be a file, but it's a directory!")]
    NotFile(u64),

    #[error("IO failed for remote operation: `{0}!")]
    RemoteIO(RemoteIOError),

    #[error(transparent)]
    DnsLookup(DnsLookupError),

    #[error("Remote operation failed with `{0}`")]
    Remote(#[from] RemoteError),

    #[error("Could not subscribe to port `{0}`, as it is being stolen by another mirrord client.")]
    PortAlreadyStolen(Port),

    #[error("Operation is not yet supported by mirrord.")]
    NotImplemented,

    #[error("{blocked_action} is forbidden by {} for this target (your organization does not allow you to use this mirrord feature with the chosen target).", policy_name_string(.policy_name.as_deref()))]
    Forbidden {
        blocked_action: BlockedAction,
        policy_name: Option<String>,
    },

    #[error("Failed stripping path with `{0}`!")]
    StripPrefix(String),

    #[error("File has to be opened locally!")]
    OpenLocal,

    #[error("{blocked_action} is forbidden by {} for this target ({reason}).", policy_name_string(.policy_name.as_deref()))]
    ForbiddenWithReason {
        blocked_action: BlockedAction,
        policy_name: Option<String>,
        reason: String,
    },
}

impl From<StripPrefixError> for ResponseError {
    fn from(fail: StripPrefixError) -> Self {
        Self::StripPrefix(fail.to_string())
    }
}

/// If some then the name with a trailing space, else empty string.
fn policy_name_string(policy_name: Option<&str>) -> String {
    if let Some(name) = policy_name {
        format!("the mirrord policy \"{name}\"")
    } else {
        "a mirrord policy".to_string()
    }
}

/// Minimal mirrord-protocol version that allows [`BlockedAction::Mirror`].
pub static MIRROR_BLOCK_VERSION: LazyLock<VersionReq> =
    LazyLock::new(|| ">=1.12.0".parse().expect("Bad Identifier"));

/// Minimal mirrord-protocol version that allows [`ResponseError::Forbidden`] to have `reason`
/// member.
pub static MIRROR_POLICY_REASON_VERSION: LazyLock<VersionReq> =
    LazyLock::new(|| ">=1.17.0".parse().expect("Bad Identifier"));

/// All the actions that can be blocked by the operator, to identify the blocked feature in a
/// [`ResponseError::Forbidden`] or [`ResponseError::ForbiddenWithReason`] message.
#[derive(Encode, Decode, Debug, PartialEq, Clone, Eq, Error)]
pub enum BlockedAction {
    Steal(StealType),
    Mirror(Port),
}

/// Determines how a blocked action will be displayed to the user in an error.
impl fmt::Display for BlockedAction {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            BlockedAction::Steal(StealType::All(port)) => {
                write!(f, "Stealing traffic from port {port}")
            }
            BlockedAction::Steal(StealType::FilteredHttp(port, filter)) => {
                write!(
                    f,
                    "Stealing traffic from port {port} with http request filter: {filter}"
                )
            }
            BlockedAction::Steal(StealType::FilteredHttpEx(port, filter)) => {
                write!(
                    f,
                    "Stealing traffic from port {port} with http request filter: {filter}"
                )
            }
            BlockedAction::Mirror(port) => {
                write!(f, "Mirroring traffic from port {port}")
            }
        }
    }
}

#[derive(Encode, Decode, Debug, PartialEq, Clone, Eq, Error)]
pub enum RemoteError {
    #[error("Failed to find a nameserver when resolving DNS!")]
    NameserverNotFound,

    #[error("Failed parsing address into a `SocketAddr` with `{0}`!")]
    AddressParsing(String),

    #[error("Failed operation to `SocketAddr` with `{0}`!")]
    InvalidAddress(SocketAddress),

    /// Especially relevant for the outgoing traffic feature, when `golang` attempts to connect
    /// on both IPv6 and IPv4.
    #[error("Connect call to `SocketAddress` `{0}` timed out!")]
    ConnectTimedOut(SocketAddress),

    #[error(r#"Got bad regex "{0}" for http filter subscriptions. Regex error: `{1}`."#)]
    BadHttpFilterRegex(Filter, String),

    #[error(r#"Got bad regex "{0:?}" for http filter subscriptions. Regex error: `{1}`."#)]
    BadHttpFilterExRegex(HttpFilter, String),
}

impl From<AddrParseError> for RemoteError {
    fn from(fail: AddrParseError) -> Self {
        Self::AddressParsing(fail.to_string())
    }
}

/// Our internal version of Rust's `std::io::Error` that can be passed between mirrord-layer and
/// mirrord-agent.
///
/// ### `Display`
///
/// We manually implement `Display` as this error is mostly seen from a [`ResponseError`] context.
#[derive(Encode, Decode, Debug, PartialEq, Clone, Eq)]
pub struct RemoteIOError {
    pub raw_os_error: Option<i32>,
    pub kind: ErrorKindInternal,
}

impl core::fmt::Display for RemoteIOError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.kind)?;
        if let Some(code) = self.raw_os_error {
            write!(f, " (error code {code})")?;
        }
        Ok(())
    }
}

/// Our internal version of Rust's `std::io::Error` that can be passed between mirrord-layer and
/// mirrord-agent.
///
/// [`ResolveErrorKindInternal`] has a nice [`core::fmt::Display`] implementation that
/// should be user friendly, and can be appended to the generic error message here.
#[derive(Encode, Decode, Debug, PartialEq, Clone, Eq, Error)]
#[error("Failed performing `getaddrinfo`: {kind}")]
pub struct DnsLookupError {
    pub kind: ResolveErrorKindInternal,
}

impl From<io::Error> for ResponseError {
    fn from(io_error: io::Error) -> Self {
        Self::RemoteIO(RemoteIOError {
            raw_os_error: io_error.raw_os_error(),
            kind: From::from(io_error.kind()),
        })
    }
}

/// Alternative to `std::io::ErrorKind`, used to implement `bincode::Encode` and `bincode::Decode`.
#[derive(Encode, Decode, Debug, PartialEq, Clone, Eq)]
pub enum ErrorKindInternal {
    NotFound,
    PermissionDenied,
    ConnectionRefused,
    ConnectionReset,
    HostUnreachable,
    NetworkUnreachable,
    ConnectionAborted,
    NotConnected,
    AddrInUse,
    AddrNotAvailable,
    NetworkDown,
    BrokenPipe,
    AlreadyExists,
    WouldBlock,
    NotADirectory,
    IsADirectory,
    DirectoryNotEmpty,
    ReadOnlyFilesystem,
    FilesystemLoop,
    StaleNetworkFileHandle,
    InvalidInput,
    InvalidData,
    TimedOut,
    WriteZero,
    StorageFull,
    NotSeekable,
    FilesystemQuotaExceeded,
    FileTooLarge,
    ResourceBusy,
    ExecutableFileBusy,
    Deadlock,
    CrossesDevices,
    TooManyLinks,
    InvalidFilename,
    ArgumentListTooLong,
    Interrupted,
    Unsupported,
    UnexpectedEof,
    OutOfMemory,
    Other,
    // Unknown is for uncovered cases (enum is non-exhaustive)
    Unknown(String),
}

/// Alternative to `std::io::ErrorKind`, used to implement `bincode::Encode` and `bincode::Decode`.
#[derive(Encode, Decode, Debug, PartialEq, Clone, Eq)]
pub enum ResolveErrorKindInternal {
    Message(String),
    NoConnections,
    NoRecordsFound(u16),
    Proto,
    Timeout,
    // Unknown is for uncovered cases (enum is non-exhaustive)
    Unknown,
    NotFound,
    PermissionDenied,
}

impl core::fmt::Display for ResolveErrorKindInternal {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ResolveErrorKindInternal::Message(message) => write!(f, "{message}"),
            ResolveErrorKindInternal::NoConnections => write!(f, "no connections"),
            ResolveErrorKindInternal::NoRecordsFound(records) => {
                write!(f, "no records found {records}")
            }
            ResolveErrorKindInternal::Proto => write!(f, "protocol error"),
            ResolveErrorKindInternal::Timeout => write!(f, "timeout"),
            ResolveErrorKindInternal::Unknown => write!(f, "unknown error"),
            ResolveErrorKindInternal::NotFound => write!(
                f,
                "the agent could not find a DNS related file, such as \
                `/etc/resolv.conf` or `/etc/hosts`"
            ),
            ResolveErrorKindInternal::PermissionDenied => write!(
                f,
                "the agent lacks sufficient permissions to open or read a DNS related \
                file, such as `/etc/resolv.conf` or `/etc/hosts`"
            ),
        }
    }
}

impl From<io::ErrorKind> for ErrorKindInternal {
    fn from(error_kind: io::ErrorKind) -> Self {
        match error_kind {
            io::ErrorKind::NotFound => ErrorKindInternal::NotFound,
            io::ErrorKind::PermissionDenied => ErrorKindInternal::PermissionDenied,
            io::ErrorKind::ConnectionRefused => ErrorKindInternal::ConnectionRefused,
            io::ErrorKind::ConnectionReset => ErrorKindInternal::ConnectionReset,
            io::ErrorKind::HostUnreachable => ErrorKindInternal::HostUnreachable,
            io::ErrorKind::NetworkUnreachable => ErrorKindInternal::NetworkUnreachable,
            io::ErrorKind::ConnectionAborted => ErrorKindInternal::ConnectionAborted,
            io::ErrorKind::NotConnected => ErrorKindInternal::NotConnected,
            io::ErrorKind::AddrInUse => ErrorKindInternal::AddrInUse,
            io::ErrorKind::AddrNotAvailable => ErrorKindInternal::AddrNotAvailable,
            io::ErrorKind::NetworkDown => ErrorKindInternal::NetworkDown,
            io::ErrorKind::BrokenPipe => ErrorKindInternal::BrokenPipe,
            io::ErrorKind::AlreadyExists => ErrorKindInternal::AlreadyExists,
            io::ErrorKind::WouldBlock => ErrorKindInternal::WouldBlock,
            io::ErrorKind::NotADirectory => ErrorKindInternal::NotADirectory,
            io::ErrorKind::IsADirectory => ErrorKindInternal::IsADirectory,
            io::ErrorKind::DirectoryNotEmpty => ErrorKindInternal::DirectoryNotEmpty,
            io::ErrorKind::ReadOnlyFilesystem => ErrorKindInternal::ReadOnlyFilesystem,
            io::ErrorKind::FilesystemLoop => ErrorKindInternal::FilesystemLoop,
            io::ErrorKind::StaleNetworkFileHandle => ErrorKindInternal::StaleNetworkFileHandle,
            io::ErrorKind::InvalidInput => ErrorKindInternal::InvalidInput,
            io::ErrorKind::InvalidData => ErrorKindInternal::InvalidData,
            io::ErrorKind::TimedOut => ErrorKindInternal::TimedOut,
            io::ErrorKind::WriteZero => ErrorKindInternal::WriteZero,
            io::ErrorKind::StorageFull => ErrorKindInternal::StorageFull,
            io::ErrorKind::NotSeekable => ErrorKindInternal::NotSeekable,
            io::ErrorKind::QuotaExceeded => ErrorKindInternal::FilesystemQuotaExceeded,
            io::ErrorKind::FileTooLarge => ErrorKindInternal::FileTooLarge,
            io::ErrorKind::ResourceBusy => ErrorKindInternal::ResourceBusy,
            io::ErrorKind::ExecutableFileBusy => ErrorKindInternal::ExecutableFileBusy,
            io::ErrorKind::Deadlock => ErrorKindInternal::Deadlock,
            io::ErrorKind::CrossesDevices => ErrorKindInternal::CrossesDevices,
            io::ErrorKind::TooManyLinks => ErrorKindInternal::TooManyLinks,
            io::ErrorKind::InvalidFilename => ErrorKindInternal::InvalidFilename,
            io::ErrorKind::ArgumentListTooLong => ErrorKindInternal::ArgumentListTooLong,
            io::ErrorKind::Interrupted => ErrorKindInternal::Interrupted,
            io::ErrorKind::Unsupported => ErrorKindInternal::Unsupported,
            io::ErrorKind::UnexpectedEof => ErrorKindInternal::UnexpectedEof,
            io::ErrorKind::OutOfMemory => ErrorKindInternal::OutOfMemory,
            io::ErrorKind::Other => ErrorKindInternal::Other,
            _ => ErrorKindInternal::Unknown(error_kind.to_string()),
        }
    }
}
