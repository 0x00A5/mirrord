use core::fmt;
#[cfg(target_os = "linux")]
use std::fs::DirEntry;
#[cfg(target_os = "linux")]
use std::io;
#[cfg(target_os = "linux")]
use std::os::unix::fs::DirEntryExt;
use std::{
    fs::Metadata, io::SeekFrom, os::unix::prelude::MetadataExt, path::PathBuf, sync::LazyLock,
};

use bincode::{Decode, Encode};
#[cfg(target_os = "linux")]
use nix::sys::statfs::Statfs;
use semver::VersionReq;

use crate::Payload;

/// Minimal mirrord-protocol version that allows [`ReadLinkFileRequest`].
pub static READLINK_VERSION: LazyLock<VersionReq> =
    LazyLock::new(|| ">=1.6.0".parse().expect("Bad Identifier"));

/// Minimal mirrord-protocol version that allows [`ReadDirBatchRequest`].
pub static READDIR_BATCH_VERSION: LazyLock<VersionReq> =
    LazyLock::new(|| ">=1.9.0".parse().expect("Bad Identifier"));

/// Minimal mirrord-protocol version that allows [`MakeDirRequest`] and [`MakeDirAtRequest`].
pub static MKDIR_VERSION: LazyLock<VersionReq> =
    LazyLock::new(|| ">=1.13.0".parse().expect("Bad Identifier"));

/// Minimal mirrord-protocol version that allows [`RemoveDirRequest`], [`UnlinkRequest`] and
/// [`UnlinkAtRequest`]..
pub static RMDIR_VERSION: LazyLock<VersionReq> =
    LazyLock::new(|| ">=1.14.0".parse().expect("Bad Identifier"));

pub static OPEN_LOCAL_VERSION: LazyLock<VersionReq> =
    LazyLock::new(|| ">=1.13.3".parse().expect("Bad Identifier"));

pub static STATFS_VERSION: LazyLock<VersionReq> =
    LazyLock::new(|| ">=1.16.0".parse().expect("Bad Identifier"));

/// Allows for V2 statfs requests/responses. The V2 responses contain more fields with statfs data.
pub static STATFS_V2_VERSION: LazyLock<VersionReq> =
    LazyLock::new(|| ">=1.18.0".parse().expect("Bad Identifier"));

/// Internal version of Metadata across operating system (macOS, Linux)
/// Only mutual attributes
#[derive(Encode, Decode, Debug, PartialEq, Clone, Copy, Eq, Default)]
pub struct MetadataInternal {
    /// dev_id, st_dev
    pub device_id: u64,
    /// inode, st_ino
    pub inode: u64,
    /// file type, st_mode
    pub mode: u32,
    /// number of hard links, st_nlink
    pub hard_links: u64,
    /// user id, st_uid
    pub user_id: u32,
    /// group id, st_gid
    pub group_id: u32,
    /// rdevice id, st_rdev (special file)
    pub rdevice_id: u64,
    /// file size, st_size
    pub size: u64,
    /// time is in nano seconds, can be converted to seconds by dividing by 1e9
    /// access time, st_atime_ns
    pub access_time: i64,
    /// modification time, st_mtime_ns
    pub modification_time: i64,
    /// creation time, st_ctime_ns
    pub creation_time: i64,
    /// block size, st_blksize
    pub block_size: u64,
    /// number of blocks, st_blocks
    pub blocks: u64,
}

impl From<Metadata> for MetadataInternal {
    fn from(metadata: Metadata) -> Self {
        Self {
            device_id: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            hard_links: metadata.nlink(),
            user_id: metadata.uid(),
            group_id: metadata.gid(),
            rdevice_id: metadata.rdev(),
            size: metadata.size(),
            access_time: metadata.atime_nsec(),
            modification_time: metadata.mtime_nsec(),
            creation_time: metadata.ctime_nsec(),
            block_size: metadata.blksize(),
            blocks: metadata.blocks(),
        }
    }
}

#[derive(Encode, Decode, Debug, PartialEq, Clone, Copy, Eq, Default)]
pub struct FsMetadataInternal {
    /// f_type
    pub filesystem_type: i64,
    /// f_bsize
    pub block_size: i64,
    /// f_blocks
    pub blocks: u64,
    /// f_bfree
    pub blocks_free: u64,
    /// f_bavail
    pub blocks_available: u64,
    /// f_files
    pub files: u64,
    /// f_ffree
    pub files_free: u64,
}

#[cfg(target_os = "linux")]
impl From<Statfs> for FsMetadataInternal {
    fn from(stat: Statfs) -> Self {
        FsMetadataInternal {
            filesystem_type: stat.filesystem_type().0,
            block_size: stat.block_size(),
            blocks: stat.blocks(),
            blocks_free: stat.blocks_free(),
            blocks_available: stat.blocks_available(),
            files: stat.files(),
            files_free: stat.files_free(),
        }
    }
}

#[derive(Encode, Decode, Debug, PartialEq, Clone, Copy, Eq, Default)]
pub struct FsMetadataInternalV2 {
    /// f_type
    pub filesystem_type: i64,
    /// f_bsize
    pub block_size: i64,
    /// f_blocks
    pub blocks: u64,
    /// f_bfree
    pub blocks_free: u64,
    /// f_bavail
    pub blocks_available: u64,
    /// f_files
    pub files: u64,
    /// f_ffree
    pub files_free: u64,
    /// f_fsid
    pub filesystem_id: [i32; 2],
    /// f_namelen
    pub name_len: i64,
    /// f_frsize
    pub fragment_size: i64,
    /// f_flags
    pub flags: i64,
}

impl From<FsMetadataInternal> for FsMetadataInternalV2 {
    fn from(
        FsMetadataInternal {
            filesystem_type,
            block_size,
            blocks,
            blocks_free,
            blocks_available,
            files,
            files_free,
        }: FsMetadataInternal,
    ) -> Self {
        Self {
            filesystem_type,
            block_size,
            blocks,
            blocks_free,
            blocks_available,
            files,
            files_free,
            ..Default::default()
        }
    }
}

impl From<FsMetadataInternalV2> for FsMetadataInternal {
    fn from(
        FsMetadataInternalV2 {
            filesystem_type,
            block_size,
            blocks,
            blocks_free,
            blocks_available,
            files,
            files_free,
            ..
        }: FsMetadataInternalV2,
    ) -> Self {
        FsMetadataInternal {
            filesystem_type,
            block_size,
            blocks,
            blocks_free,
            blocks_available,
            files,
            files_free,
        }
    }
}

#[cfg(target_os = "linux")]
impl From<Statfs> for FsMetadataInternalV2 {
    fn from(stat: Statfs) -> Self {
        // The reason we need the inner `statfs64` struct is that nix's `Statfs` just does not give
        // any access to some fields, like `f_frsize` and the value of `f_fsid`.
        // SAFETY: Statfs is just a `#[repr(transparent)]` wrapper around `libc::statfs64` on Linux.
        let inner: libc::statfs64 = unsafe { std::mem::transmute(stat) };
        Self {
            filesystem_type: inner.f_type,
            block_size: inner.f_bsize,
            blocks: inner.f_blocks,
            blocks_free: inner.f_bfree,
            blocks_available: inner.f_bavail,
            files: inner.f_files,
            files_free: inner.f_ffree,
            // SAFETY: `statfs64.f_fsid` is `libc::fsid_t`, which has `#[repr(C)]` (even though
            // you can't see that at first glance because it's added via the `s!` macro), and only
            // contains 1 field which has the same type as `Self.filesystem_id`.
            filesystem_id: unsafe { std::mem::transmute::<libc::fsid_t, [i32; 2]>(inner.f_fsid) },
            name_len: inner.f_namelen,
            fragment_size: inner.f_frsize,
            flags: inner.f_flags,
        }
    }
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct DirEntryInternal {
    pub inode: u64,
    pub position: u64,
    pub name: String,
    pub file_type: u8,
}

#[cfg(target_os = "linux")]
impl TryFrom<(usize, io::Result<DirEntry>)> for DirEntryInternal {
    type Error = io::Error;

    fn try_from(offset_entry_pair: (usize, io::Result<DirEntry>)) -> Result<Self, Self::Error> {
        let (offset, entry) = offset_entry_pair;
        let entry = entry?;

        let mode = entry.metadata()?.mode();

        let file_type = match mode & libc::S_IFMT {
            libc::S_IFLNK => libc::DT_LNK,
            libc::S_IFREG => libc::DT_REG,
            libc::S_IFBLK => libc::DT_BLK,
            libc::S_IFDIR => libc::DT_DIR,
            libc::S_IFCHR => libc::DT_CHR,
            libc::S_IFIFO => libc::DT_FIFO,
            libc::S_IFSOCK => libc::DT_SOCK,
            _ => libc::DT_UNKNOWN,
        };

        Ok(DirEntryInternal {
            inode: entry.ino(),
            position: offset as u64,
            name: entry.file_name().to_string_lossy().into(),
            file_type,
        })
    }
}

impl DirEntryInternal {
    /// Calculate the `d_reclen` field of a the kernel's `linux_dirent64` struct.
    /// The actual size of an instance is not `sizeof(linux_dirent64)`, since it contains a flexible
    /// array member.
    /// This functions calculates the expected `d_reclen` assuming:
    /// ```C
    /// sizeof(ino64_t) == 8
    /// sizeof(off64_t) == 8
    /// sizeof(unsigned short) == 2
    /// sizeof(unsinged char) == 1
    /// ```
    pub fn get_d_reclen64(&self) -> u16 {
        // The 20 is for 19 bytes of fixed size members + the terminating null of the string.
        Self::round_up_to_next_multiple_of_8(20 + self.name.len() as u16)
    }

    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(round_up_to_next_multiple_of_8(0), 0);
    /// assert_eq!(round_up_to_next_multiple_of_8(1), 8);
    /// assert_eq!(round_up_to_next_multiple_of_8(8), 8);
    /// assert_eq!(round_up_to_next_multiple_of_8(21), 24);
    /// ```
    fn round_up_to_next_multiple_of_8(x: u16) -> u16 {
        (x + 7) & !7
    }
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct OpenFileRequest {
    pub path: PathBuf,
    pub open_options: OpenOptionsInternal,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct OpenFileResponse {
    pub fd: u64,
}

// TODO: We're not handling `custom_flags` here, if we ever need to do so, add them here (it's an OS
// specific thing).
//
// TODO: Should probably live in a separate place (same reasoning as `AddrInfoHint`).
#[derive(Encode, Decode, Debug, PartialEq, Clone, Copy, Eq, Default)]
pub struct OpenOptionsInternal {
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub truncate: bool,
    pub create: bool,
    pub create_new: bool,
}

impl OpenOptionsInternal {
    pub fn is_read_only(&self) -> bool {
        self.read && !(self.write || self.append || self.truncate || self.create || self.create_new)
    }

    pub fn is_write(&self) -> bool {
        self.write || self.append || self.truncate || self.create || self.create_new
    }
}

impl From<OpenOptionsInternal> for std::fs::OpenOptions {
    fn from(internal: OpenOptionsInternal) -> Self {
        let OpenOptionsInternal {
            read,
            write,
            append,
            truncate,
            create,
            create_new,
        } = internal;

        std::fs::OpenOptions::new()
            .read(read)
            .write(write)
            .append(append)
            .truncate(truncate)
            .create(create)
            .create_new(create_new)
            .to_owned()
    }
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct OpenRelativeFileRequest {
    pub relative_fd: u64,
    pub path: PathBuf,
    pub open_options: OpenOptionsInternal,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct ReadFileRequest {
    pub remote_fd: u64,
    pub buffer_size: u64,
}

#[derive(Encode, Decode, PartialEq, Eq, Clone)]
pub struct ReadFileResponse {
    pub bytes: Payload,
    pub read_amount: u64,
}

impl fmt::Debug for ReadFileResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadFileResponse")
            .field("bytes (length)", &self.bytes.len())
            .field("read_amount", &self.read_amount)
            .finish()
    }
}

/// The contents of the symbolic link.
#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct ReadLinkFileResponse {
    pub path: PathBuf,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct MakeDirRequest {
    pub pathname: PathBuf,
    pub mode: u32,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct MakeDirAtRequest {
    pub dirfd: u64,
    pub pathname: PathBuf,
    pub mode: u32,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct RemoveDirRequest {
    pub pathname: PathBuf,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct UnlinkRequest {
    pub pathname: PathBuf,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct UnlinkAtRequest {
    pub dirfd: Option<u64>,
    pub pathname: PathBuf,
    pub flags: u32,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct ReadLimitedFileRequest {
    pub remote_fd: u64,
    pub buffer_size: u64,
    pub start_from: u64,
}

/// `path` of the symbolic link we want to resolve.
#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct ReadLinkFileRequest {
    pub path: PathBuf,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct SeekFileRequest {
    pub fd: u64,
    pub seek_from: SeekFromInternal,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct SeekFileResponse {
    pub result_offset: u64,
}

/// Alternative to `std::io::SeekFrom`, used to implement `bincode::Encode` and `bincode::Decode`.
#[derive(Encode, Decode, Debug, PartialEq, Clone, Copy, Eq)]
pub enum SeekFromInternal {
    Start(u64),
    End(i64),
    Current(i64),
}

impl From<SeekFromInternal> for SeekFrom {
    fn from(seek_from: SeekFromInternal) -> Self {
        match seek_from {
            SeekFromInternal::Start(start) => SeekFrom::Start(start),
            SeekFromInternal::End(end) => SeekFrom::End(end),
            SeekFromInternal::Current(current) => SeekFrom::Current(current),
        }
    }
}

impl From<SeekFrom> for SeekFromInternal {
    fn from(seek_from: SeekFrom) -> Self {
        match seek_from {
            SeekFrom::Start(start) => SeekFromInternal::Start(start),
            SeekFrom::End(end) => SeekFromInternal::End(end),
            SeekFrom::Current(current) => SeekFromInternal::Current(current),
        }
    }
}

#[derive(Encode, Decode, PartialEq, Eq, Clone)]
pub struct WriteFileRequest {
    pub fd: u64,
    pub write_bytes: Payload,
}

impl fmt::Debug for WriteFileRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WriteFileRequest")
            .field("fd", &self.fd)
            .field("write_bytes (length)", &self.write_bytes.len())
            .finish()
    }
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct WriteFileResponse {
    pub written_amount: u64,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct WriteLimitedFileRequest {
    pub remote_fd: u64,
    pub start_from: u64,
    pub write_bytes: Payload,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct CloseFileRequest {
    pub fd: u64,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct AccessFileRequest {
    pub pathname: PathBuf,
    pub mode: u8,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct AccessFileResponse;

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct XstatRequest {
    pub path: Option<PathBuf>,
    pub fd: Option<u64>,
    pub follow_symlink: bool,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct XstatFsRequest {
    pub fd: u64,
}

/// Same as `XstatFsRequest`, but results in the V2 version of the response.
#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct XstatFsRequestV2 {
    pub fd: u64,
}

impl From<XstatFsRequestV2> for XstatFsRequest {
    fn from(XstatFsRequestV2 { fd }: XstatFsRequestV2) -> Self {
        Self { fd }
    }
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct StatFsRequest {
    pub path: PathBuf,
}

/// Same as `StatFsRequest`, but results in the V2 version of the response.
#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct StatFsRequestV2 {
    pub path: PathBuf,
}

impl From<StatFsRequestV2> for StatFsRequest {
    fn from(StatFsRequestV2 { path }: StatFsRequestV2) -> Self {
        Self { path }
    }
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct XstatResponse {
    pub metadata: MetadataInternal,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct XstatFsResponse {
    pub metadata: FsMetadataInternal,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct XstatFsResponseV2 {
    // I'm not sure why not directly have the fields in this struct, but this is how it is in
    // `XstatFsResponse` and `XstatResponse`, so I'm staying with that pattern it for consistency.
    pub metadata: FsMetadataInternalV2,
}

impl From<XstatFsResponse> for XstatFsResponseV2 {
    fn from(value: XstatFsResponse) -> Self {
        XstatFsResponseV2 {
            metadata: value.metadata.into(),
        }
    }
}

impl From<XstatFsResponseV2> for XstatFsResponse {
    fn from(value: XstatFsResponseV2) -> Self {
        XstatFsResponse {
            metadata: value.metadata.into(),
        }
    }
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct FdOpenDirRequest {
    pub remote_fd: u64,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct OpenDirResponse {
    pub fd: u64,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct ReadDirRequest {
    pub remote_fd: u64,
}

/// `readdir` message that requests an iterable with `amount` items from the agent.
#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct ReadDirBatchRequest {
    /// The fd of the dir in the agent.
    pub remote_fd: u64,
    /// Max amount to take from the agent's iterator of dirs.
    pub amount: usize,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct ReadDirResponse {
    pub direntry: Option<DirEntryInternal>,
}

/// `readdir` response with the list of items (length depends on the [`ReadDirBatchRequest`]'s
/// `amount`), and the `remote_fd` of the dir (for convenience).
#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct ReadDirBatchResponse {
    /// Remote fd of the dir.
    pub fd: u64,
    /// The list of [`DirEntryInternal`] where `length` is, at max, the `amount` we took
    /// from the agent's read dir iterator.
    pub dir_entries: Vec<DirEntryInternal>,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct CloseDirRequest {
    pub remote_fd: u64,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct GetDEnts64Request {
    pub remote_fd: u64,
    pub buffer_size: u64,
}

#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct GetDEnts64Response {
    pub fd: u64,
    pub entries: Vec<DirEntryInternal>,
    pub result_size: u64,
}
