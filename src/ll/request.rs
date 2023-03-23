//! Low-level filesystem operation request.
//!
//! A request represents information about a filesystem operation the kernel driver wants us to
//! perform.

use fuse_abi::*;
use std::convert::TryFrom;
use std::ffi::OsString;
use std::{error, fmt, mem};

use super::argument::ArgumentIterator;

/// Error that may occur while reading and parsing a request from the kernel driver.
#[derive(Debug)]
pub enum RequestError {
    /// Not enough data for parsing header (short read).
    ShortReadHeader(usize),
    /// Kernel requested an unknown operation.
    UnknownOperation(u32),
    /// Not enough data for arguments (short read).
    ShortRead(usize, usize),
    /// Insufficient argument data.
    InsufficientData,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::ShortReadHeader(len) => write!(
                f,
                "Short read of FUSE request header ({} < {})",
                len,
                mem::size_of::<fuse_in_header>()
            ),
            RequestError::UnknownOperation(opcode) => write!(f, "Unknown FUSE opcode ({})", opcode),
            RequestError::ShortRead(len, total) => {
                write!(f, "Short read of FUSE request ({} < {})", len, total)
            }
            RequestError::InsufficientData => write!(f, "Insufficient argument data"),
        }
    }
}

impl error::Error for RequestError {}

/// Filesystem operation (and arguments) the kernel driver wants us to perform. The fields of each
/// variant needs to match the actual arguments the kernel driver sends for the specific operation.
#[derive(Debug)]
pub enum Operation {
    Lookup {
        name: OsString,
    },
    Forget {
        arg: fuse_forget_in,
    },
    GetAttr,
    SetAttr {
        arg: fuse_setattr_in,
    },
    ReadLink,
    SymLink {
        name: OsString,
        link: OsString,
    },
    MkNod {
        arg: fuse_mknod_in,
        name: OsString,
    },
    MkDir {
        arg: fuse_mkdir_in,
        name: OsString,
    },
    Unlink {
        name: OsString,
    },
    RmDir {
        name: OsString,
    },
    Rename {
        arg: fuse_rename_in,
        name: OsString,
        newname: OsString,
    },
    Link {
        arg: fuse_link_in,
        name: OsString,
    },
    Open {
        arg: fuse_open_in,
    },
    Read {
        arg: fuse_read_in,
    },
    Write {
        arg: fuse_write_in,
        data: Vec<u8>,
    },
    StatFs,
    Release {
        arg: fuse_release_in,
    },
    FSync {
        arg: fuse_fsync_in,
    },
    SetXAttr {
        arg: fuse_setxattr_in,
        name: OsString,
        value: Vec<u8>,
    },
    GetXAttr {
        arg: fuse_getxattr_in,
        name: OsString,
    },
    ListXAttr {
        arg: fuse_getxattr_in,
    },
    RemoveXAttr {
        name: OsString,
    },
    Flush {
        arg: fuse_flush_in,
    },
    Init {
        arg: fuse_init_in,
    },
    OpenDir {
        arg: fuse_open_in,
    },
    ReadDir {
        arg: fuse_read_in,
    },
    ReleaseDir {
        arg: fuse_release_in,
    },
    FSyncDir {
        arg: fuse_fsync_in,
    },
    GetLk {
        arg: fuse_lk_in,
    },
    SetLk {
        arg: fuse_lk_in,
    },
    SetLkW {
        arg: fuse_lk_in,
    },
    Access {
        arg: fuse_access_in,
    },
    Create {
        arg: fuse_create_in,
        name: OsString,
    },
    Interrupt {
        arg: fuse_interrupt_in,
    },
    BMap {
        arg: fuse_bmap_in,
    },
    Destroy,
    // TODO: FUSE_IOCTL since ABI 7.11
    #[cfg(feature = "abi-7-11")]
    IoCtl {
        arg: fuse_ioctl_in,
        data: Vec<u8>,
    },
    #[cfg(feature = "abi-7-11")]
    Poll {
        arg: fuse_poll_in,
    },
    #[cfg(feature = "abi-7-15")]
    NotifyReply {
        data: Vec<u8>,
    },
    #[cfg(feature = "abi-7-16")]
    BatchForget {
        arg: fuse_forget_in,
        nodes: Vec<fuse_forget_one>,
    },
    #[cfg(feature = "abi-7-19")]
    FAllocate {
        arg: fuse_fallocate_in,
    },
    #[cfg(target_os = "macos")]
    SetVolName {
        name: OsString,
    },
    #[cfg(target_os = "macos")]
    GetXTimes,
    #[cfg(target_os = "macos")]
    Exchange {
        arg: fuse_exchange_in,
        oldname: OsString,
        newname: OsString,
    },
    #[cfg(feature = "abi-7-12")]
    CuseInit {
        arg: fuse_init_in,
    },
}

impl<'a> fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operation::Lookup { name } => write!(f, "LOOKUP name {:?}", name),
            Operation::Forget { arg } => write!(f, "FORGET nlookup {}", arg.nlookup),
            Operation::GetAttr => write!(f, "GETATTR"),
            Operation::SetAttr { arg } => write!(f, "SETATTR valid {:#x}", arg.valid),
            Operation::ReadLink => write!(f, "READLINK"),
            Operation::SymLink { name, link } => write!(f, "SYMLINK name {:?}, link {:?}", name, link),
            Operation::MkNod { arg, name } => write!(f, "MKNOD name {:?}, mode {:#05o}, rdev {}", name, arg.mode, arg.rdev),
            Operation::MkDir { arg, name } => write!(f, "MKDIR name {:?}, mode {:#05o}", name, arg.mode),
            Operation::Unlink { name } => write!(f, "UNLINK name {:?}", name),
            Operation::RmDir { name } => write!(f, "RMDIR name {:?}", name),
            Operation::Rename { arg, name, newname } => write!(f, "RENAME name {:?}, newdir {:#018x}, newname {:?}", name, arg.newdir, newname),
            Operation::Link { arg, name } => write!(f, "LINK name {:?}, oldnodeid {:#018x}", name, arg.oldnodeid),
            Operation::Open { arg } => write!(f, "OPEN flags {:#x}", arg.flags),
            Operation::Read { arg } => write!(f, "READ fh {}, offset {}, size {}", arg.fh, arg.offset, arg.size),
            Operation::Write { arg, .. } => write!(f, "WRITE fh {}, offset {}, size {}, write flags {:#x}", arg.fh, arg.offset, arg.size, arg.write_flags),
            Operation::StatFs => write!(f, "STATFS"),
            Operation::Release { arg } => write!(f, "RELEASE fh {}, flags {:#x}, release flags {:#x}, lock owner {}", arg.fh, arg.flags, arg.release_flags, arg.lock_owner),
            Operation::FSync { arg } => write!(f, "FSYNC fh {}, fsync flags {:#x}", arg.fh, arg.fsync_flags),
            Operation::SetXAttr { arg, name, .. } => write!(f, "SETXATTR name {:?}, size {}, flags {:#x}", name, arg.size, arg.flags),
            Operation::GetXAttr { arg, name } => write!(f, "GETXATTR name {:?}, size {}", name, arg.size),
            Operation::ListXAttr { arg } => write!(f, "LISTXATTR size {}", arg.size),
            Operation::RemoveXAttr { name } => write!(f, "REMOVEXATTR name {:?}", name),
            Operation::Flush { arg } => write!(f, "FLUSH fh {}, lock owner {}", arg.fh, arg.lock_owner),
            Operation::Init { arg } => write!(f, "INIT kernel ABI {}.{}, flags {:#x}, max readahead {}", arg.major, arg.minor, arg.flags, arg.max_readahead),
            Operation::OpenDir { arg } => write!(f, "OPENDIR flags {:#x}", arg.flags),
            Operation::ReadDir { arg } => write!(f, "READDIR fh {}, offset {}, size {}", arg.fh, arg.offset, arg.size),
            Operation::ReleaseDir { arg } => write!(f, "RELEASEDIR fh {}, flags {:#x}, release flags {:#x}, lock owner {}", arg.fh, arg.flags, arg.release_flags, arg.lock_owner),
            Operation::FSyncDir { arg } => write!(f, "FSYNCDIR fh {}, fsync flags {:#x}", arg.fh, arg.fsync_flags),
            Operation::GetLk { arg } => write!(f, "GETLK fh {}, lock owner {}", arg.fh, arg.owner),
            Operation::SetLk { arg } => write!(f, "SETLK fh {}, lock owner {}", arg.fh, arg.owner),
            Operation::SetLkW { arg } => write!(f, "SETLKW fh {}, lock owner {}", arg.fh, arg.owner),
            Operation::Access { arg } => write!(f, "ACCESS mask {:#05o}", arg.mask),
            Operation::Create { arg, name } => write!(f, "CREATE name {:?}, mode {:#05o}, flags {:#x}", name, arg.mode, arg.flags),
            Operation::Interrupt { arg } => write!(f, "INTERRUPT unique {}", arg.unique),
            Operation::BMap { arg } => write!(f, "BMAP blocksize {}, ids {}", arg.blocksize, arg.block),
            Operation::Destroy => write!(f, "DESTROY"),
            #[cfg(feature = "abi-7-11")]
            Operation::IoCtl { arg, .. } => write!(f, "IOCTL fh {}", arg.fh),
            #[cfg(feature = "abi-7-11")]
            Operation::Poll {arg } => write!(f, "GETLK fh {}", arg.fh),
            #[cfg(feature = "abi-7-15")]
            Operation::NotifyReply{..}  => write!(f, "NOTIFY_REPLY"),
            #[cfg(feature = "abi-7-16")]
             Operation::BatchForget {..} => write!(f, "BATCH_FORGET fh "),
            #[cfg(feature = "abi-7-19")]
            Operation::FAllocate { .. }=> write!(f, "FALLOCATE fh"),
            #[cfg(feature = "abi-7-12")]
            Operation::CuseInit {..} => write!(f, "CUSEINIT fh"),
            #[cfg(target_os = "macos")]
            Operation::SetVolName { name } => write!(f, "SETVOLNAME name {:?}", name),
            #[cfg(target_os = "macos")]
            Operation::GetXTimes => write!(f, "GETXTIMES"),
            #[cfg(target_os = "macos")]
            Operation::Exchange { arg, oldname, newname } => write!(f, "EXCHANGE olddir {:#018x}, oldname {:?}, newdir {:#018x}, newname {:?}, options {:#x}", arg.olddir, oldname, arg.newdir, newname, arg.options),
        }
    }
}

impl Operation {
    fn parse(opcode: &fuse_opcode, data: &mut ArgumentIterator<'_>) -> Option<Self> {
        unsafe {
            Some(match opcode {
                fuse_opcode::FUSE_LOOKUP => Operation::Lookup {
                    name: data.fetch_str()?.into(),
                },
                fuse_opcode::FUSE_FORGET => Operation::Forget {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_GETATTR => Operation::GetAttr,
                fuse_opcode::FUSE_SETATTR => Operation::SetAttr {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_READLINK => Operation::ReadLink,
                fuse_opcode::FUSE_SYMLINK => Operation::SymLink {
                    name: data.fetch_str()?.into(),
                    link: data.fetch_str()?.into(),
                },
                fuse_opcode::FUSE_MKNOD => Operation::MkNod {
                    arg: *data.fetch()?,
                    name: data.fetch_str()?.into(),
                },
                fuse_opcode::FUSE_MKDIR => Operation::MkDir {
                    arg: *data.fetch()?,
                    name: data.fetch_str()?.into(),
                },
                fuse_opcode::FUSE_UNLINK => Operation::Unlink {
                    name: data.fetch_str()?.into(),
                },
                fuse_opcode::FUSE_RMDIR => Operation::RmDir {
                    name: data.fetch_str()?.into(),
                },
                fuse_opcode::FUSE_RENAME => Operation::Rename {
                    arg: *data.fetch()?,
                    name: data.fetch_str()?.into(),
                    newname: data.fetch_str()?.into(),
                },
                fuse_opcode::FUSE_LINK => Operation::Link {
                    arg: *data.fetch()?,
                    name: data.fetch_str()?.into(),
                },
                fuse_opcode::FUSE_OPEN => Operation::Open {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_READ => Operation::Read {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_WRITE => Operation::Write {
                    arg: *data.fetch()?,
                    data: data.fetch_all().to_vec(),
                },
                fuse_opcode::FUSE_STATFS => Operation::StatFs,
                fuse_opcode::FUSE_RELEASE => Operation::Release {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_FSYNC => Operation::FSync {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_SETXATTR => Operation::SetXAttr {
                    arg: *data.fetch()?,
                    name: data.fetch_str()?.into(),
                    value: data.fetch_all().to_vec(),
                },
                fuse_opcode::FUSE_GETXATTR => Operation::GetXAttr {
                    arg: *data.fetch()?,
                    name: data.fetch_str()?.into(),
                },
                fuse_opcode::FUSE_LISTXATTR => Operation::ListXAttr {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_REMOVEXATTR => Operation::RemoveXAttr {
                    name: data.fetch_str()?.into(),
                },
                fuse_opcode::FUSE_FLUSH => Operation::Flush {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_INIT => Operation::Init {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_OPENDIR => Operation::OpenDir {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_READDIR => Operation::ReadDir {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_RELEASEDIR => Operation::ReleaseDir {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_FSYNCDIR => Operation::FSyncDir {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_GETLK => Operation::GetLk {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_SETLK => Operation::SetLk {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_SETLKW => Operation::SetLkW {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_ACCESS => Operation::Access {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_CREATE => Operation::Create {
                    arg: *data.fetch()?,
                    name: data.fetch_str()?.into(),
                },
                fuse_opcode::FUSE_INTERRUPT => Operation::Interrupt {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_BMAP => Operation::BMap {
                    arg: *data.fetch()?,
                },
                fuse_opcode::FUSE_DESTROY => Operation::Destroy,

                #[cfg(feature = "abi-7-11")]
                fuse_opcode::FUSE_IOCTL => Operation::IoCtl {
                    arg: *data.fetch()?,
                    data: data.fetch_all().to_vec(),
                },
                #[cfg(feature = "abi-7-11")]
                fuse_opcode::FUSE_POLL => Operation::Poll {
                    arg: *data.fetch()?,
                },
                #[cfg(feature = "abi-7-15")]
                fuse_opcode::FUSE_NOTIFY_REPLY => Operation::NotifyReply {
                    data: data.fetch_all().to_vec(),
                },
                #[cfg(feature = "abi-7-16")]
                fuse_opcode::FUSE_BATCH_FORGET => {
                    let arg = *data.fetch()?;
                    let mut nodes: Vec<fuse_forget_one> = Vec::new();
                    while let Some(node) = data.fetch::<fuse_forget_one>() {
                        nodes.push(node.clone());
                    }
                    Operation::BatchForget {
                        arg: arg,
                        nodes: nodes,
                    }
                }
                #[cfg(feature = "abi-7-19")]
                fuse_opcode::FUSE_FALLOCATE => Operation::FAllocate {
                    arg: *data.fetch()?,
                },
                #[cfg(feature = "abi-7-12")]
                fuse_opcode::CUSE_INIT => Operation::CuseInit {
                    arg: *data.fetch()?,
                },

                #[cfg(target_os = "macos")]
                fuse_opcode::FUSE_SETVOLNAME => Operation::SetVolName {
                    name: data.fetch_str()?.into(),
                },
                #[cfg(target_os = "macos")]
                fuse_opcode::FUSE_GETXTIMES => Operation::GetXTimes,
                #[cfg(target_os = "macos")]
                fuse_opcode::FUSE_EXCHANGE => Operation::Exchange {
                    arg: *data.fetch()?,
                    oldname: data.fetch_str()?.into(),
                    newname: data.fetch_str()?.into(),
                },
            })
        }
    }
}

/// Low-level request of a filesystem operation the kernel driver wants to perform.
#[derive(Debug)]
pub struct Request {
    header: fuse_in_header,
    operation: Operation,
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FUSE({:3}) ino {:#018x}: {}",
            self.header.unique, self.header.nodeid, self.operation
        )
    }
}

impl TryFrom<&[u8]> for Request {
    type Error = RequestError;

    fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
        // Parse a raw packet as sent by the kernel driver into typed data. Every request always
        // begins with a `fuse_in_header` struct followed by arguments depending on the opcode.
        let data_len = data.len();
        let mut data = ArgumentIterator::new(data);
        // Parse header
        let header: &fuse_in_header =
            unsafe { data.fetch() }.ok_or_else(|| RequestError::ShortReadHeader(data.len()))?;
        // Parse/check opcode
        let opcode = fuse_opcode::try_from(header.opcode)
            .map_err(|_: InvalidOpcodeError| RequestError::UnknownOperation(header.opcode))?;
        // Check data size
        if data_len < header.len as usize {
            return Err(RequestError::ShortRead(data_len, header.len as usize));
        }
        // Parse/check operation arguments
        let operation =
            Operation::parse(&opcode, &mut data).ok_or_else(|| RequestError::InsufficientData)?;
        let header = *header;

        Ok(Self { header, operation })
    }
}

impl Request {
    /// Returns the unique identifier of this request.
    ///
    /// The FUSE kernel driver assigns a unique id to every concurrent request. This allows to
    /// distinguish between multiple concurrent requests. The unique id of a request may be
    /// reused in later requests after it has completed.
    #[inline]
    pub fn unique(&self) -> u64 {
        self.header.unique
    }

    /// Returns the node id of the inode this request is targeted to.
    #[inline]
    pub fn nodeid(&self) -> u64 {
        self.header.nodeid
    }

    /// Returns the UID that the process that triggered this request runs under.
    #[inline]
    pub fn uid(&self) -> u32 {
        self.header.uid
    }

    /// Returns the GID that the process that triggered this request runs under.
    #[inline]
    pub fn gid(&self) -> u32 {
        self.header.gid
    }

    /// Returns the PID of the process that triggered this request.
    #[inline]
    pub fn pid(&self) -> u32 {
        self.header.pid
    }

    /// Returns the filesystem operation (and its arguments) of this request.
    #[inline]
    pub fn operation(&self) -> &Operation {
        &self.operation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_endian = "big")]
    const INIT_REQUEST: [u8; 56] = [
        0x00, 0x00, 0x00, 0x38, 0x00, 0x00, 0x00, 0x1a, // len, opcode
        0xde, 0xad, 0xbe, 0xef, 0xba, 0xad, 0xd0, 0x0d, // unique
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, // nodeid
        0xc0, 0x01, 0xd0, 0x0d, 0xc0, 0x01, 0xca, 0xfe, // uid, gid
        0xc0, 0xde, 0xba, 0x5e, 0x00, 0x00, 0x00, 0x00, // pid, padding
        0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x08, // major, minor
        0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, // max_readahead, flags
    ];

    #[cfg(target_endian = "little")]
    const INIT_REQUEST: [u8; 56] = [
        0x38, 0x00, 0x00, 0x00, 0x1a, 0x00, 0x00, 0x00, // len, opcode
        0x0d, 0xf0, 0xad, 0xba, 0xef, 0xbe, 0xad, 0xde, // unique
        0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, // nodeid
        0x0d, 0xd0, 0x01, 0xc0, 0xfe, 0xca, 0x01, 0xc0, // uid, gid
        0x5e, 0xba, 0xde, 0xc0, 0x00, 0x00, 0x00, 0x00, // pid, padding
        0x07, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, // major, minor
        0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // max_readahead, flags
    ];

    #[cfg(target_endian = "big")]
    const MKNOD_REQUEST: [u8; 56] = [
        0x00, 0x00, 0x00, 0x38, 0x00, 0x00, 0x00, 0x08, // len, opcode
        0xde, 0xad, 0xbe, 0xef, 0xba, 0xad, 0xd0, 0x0d, // unique
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, // nodeid
        0xc0, 0x01, 0xd0, 0x0d, 0xc0, 0x01, 0xca, 0xfe, // uid, gid
        0xc0, 0xde, 0xba, 0x5e, 0x00, 0x00, 0x00, 0x00, // pid, padding
        0x00, 0x00, 0x01, 0xa4, 0x00, 0x00, 0x00, 0x00, // mode, rdev
        0x66, 0x6f, 0x6f, 0x2e, 0x74, 0x78, 0x74, 0x00, // name
    ];

    #[cfg(target_endian = "little")]
    const MKNOD_REQUEST: [u8; 56] = [
        0x38, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, // len, opcode
        0x0d, 0xf0, 0xad, 0xba, 0xef, 0xbe, 0xad, 0xde, // unique
        0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, // nodeid
        0x0d, 0xd0, 0x01, 0xc0, 0xfe, 0xca, 0x01, 0xc0, // uid, gid
        0x5e, 0xba, 0xde, 0xc0, 0x00, 0x00, 0x00, 0x00, // pid, padding
        0xa4, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // mode, rdev
        0x66, 0x6f, 0x6f, 0x2e, 0x74, 0x78, 0x74, 0x00, // name
    ];

    #[test]
    fn short_read_header() {
        match Request::try_from(&INIT_REQUEST[..20]) {
            Err(RequestError::ShortReadHeader(20)) => (),
            _ => panic!("Unexpected request parsing result"),
        }
    }

    #[test]
    fn short_read() {
        match Request::try_from(&INIT_REQUEST[..48]) {
            Err(RequestError::ShortRead(48, 56)) => (),
            _ => panic!("Unexpected request parsing result"),
        }
    }

    #[test]
    fn init() {
        let req = Request::try_from(&INIT_REQUEST[..]).unwrap();
        assert_eq!(req.header.len, 56);
        assert_eq!(req.header.opcode, 26);
        assert_eq!(req.unique(), 0xdead_beef_baad_f00d);
        assert_eq!(req.nodeid(), 0x1122_3344_5566_7788);
        assert_eq!(req.uid(), 0xc001_d00d);
        assert_eq!(req.gid(), 0xc001_cafe);
        assert_eq!(req.pid(), 0xc0de_ba5e);
        match req.operation() {
            Operation::Init { arg } => {
                assert_eq!(arg.major, 7);
                assert_eq!(arg.minor, 8);
                assert_eq!(arg.max_readahead, 4096);
            }
            _ => panic!("Unexpected request operation"),
        }
    }

    #[test]
    fn mknod() {
        let req = Request::try_from(&MKNOD_REQUEST[..]).unwrap();
        assert_eq!(req.header.len, 56);
        assert_eq!(req.header.opcode, 8);
        assert_eq!(req.unique(), 0xdead_beef_baad_f00d);
        assert_eq!(req.nodeid(), 0x1122_3344_5566_7788);
        assert_eq!(req.uid(), 0xc001_d00d);
        assert_eq!(req.gid(), 0xc001_cafe);
        assert_eq!(req.pid(), 0xc0de_ba5e);
        match req.operation() {
            Operation::MkNod { arg, name } => {
                assert_eq!(arg.mode, 0o644);
                assert_eq!(*name, "foo.txt");
            }
            _ => panic!("Unexpected request operation"),
        }
    }
}
