//! Filesystem operation request
//!
//! A request represents information about a filesystem operation the kernel driver wants us to
//! perform.
//!
//! TODO: This module is meant to go away soon in favor of `ll::Request`.

use fuse_abi::consts::*;
use fuse_abi::*;
use libc::{EIO, ENOSYS, EPROTO};
use log::{debug, error, warn};
use std::convert::TryFrom;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::channel::ChannelSender;
use crate::ll;
use crate::reply::{Reply, ReplyDirectory, ReplyEmpty, ReplyRaw};
use crate::session::{Session, MAX_WRITE_SIZE};
use crate::Filesystem;

/// We generally support async reads
#[cfg(not(target_os = "macos"))]
const INIT_FLAGS: u32 = FUSE_ASYNC_READ;
// TODO: Add FUSE_EXPORT_SUPPORT and FUSE_BIG_WRITES (requires ABI 7.10)

/// On macOS, we additionally support case insensitiveness, volume renames and xtimes
/// TODO: we should eventually let the filesystem implementation decide which flags to set
#[cfg(target_os = "macos")]
const INIT_FLAGS: u32 = FUSE_ASYNC_READ | FUSE_CASE_INSENSITIVE | FUSE_VOL_RENAME | FUSE_XTIMES;
// TODO: Add FUSE_EXPORT_SUPPORT and FUSE_BIG_WRITES (requires ABI 7.10)

/// Request data structure
#[derive(Debug)]
pub struct Request {
    /// Channel sender for sending the reply
    ch: ChannelSender,
    /// Parsed request
    request: ll::Request,
}

impl Request {
    /// Create a new request from the given data
    pub fn new(ch: ChannelSender, data: &[u8]) -> Option<Request> {
        let request = match ll::Request::try_from(data) {
            Ok(request) => request,
            Err(err) => {
                // FIXME: Reply with ENOSYS?
                error!("{}", err);
                return None;
            }
        };

        Some(Self { ch, request })
    }

    /// Dispatch request to the given filesystem.
    /// This calls the appropriate filesystem operation method for the
    /// request and sends back the returned reply to the kernel
    pub async fn dispatch<FS: Filesystem + Send + Sync + 'static>(self, se: Arc<Session<FS>>) {
        let req = &self;
        debug!("{}", req.request);

        match req.request.operation() {
            // Filesystem initialization
            ll::Operation::Init { arg } => {
                let reply: ReplyRaw<fuse_init_out> = req.reply();
                // We don't support ABI versions before 7.6
                if arg.major < 7 || (arg.major == 7 && arg.minor < 6) {
                    error!("Unsupported FUSE ABI version {}.{}", arg.major, arg.minor);
                    reply.error(EPROTO);
                    return;
                }
                // Remember ABI version supported by kernel
                se.proto_major.store(arg.major, Ordering::Relaxed);
                se.proto_minor.store(arg.minor, Ordering::Relaxed);

                // Call filesystem init method and give it a chance to return an error
                let res = se.filesystem.init(req).await;
                if let Err(err) = res {
                    reply.error(err);
                    return;
                }
                // Reply with our desired version and settings. If the kernel supports a
                // larger major version, it'll re-send a matching init message. If it
                // supports only lower major versions, we replied with an error above.
                let init = fuse_init_out {
                    major: FUSE_KERNEL_VERSION,
                    minor: FUSE_KERNEL_MINOR_VERSION,
                    max_readahead: arg.max_readahead, // accept any readahead size
                    flags: arg.flags & INIT_FLAGS, // use features given in INIT_FLAGS and reported as capable
                    #[cfg(not(feature = "abi-7-13"))]
                    unused: 0,
                    max_write: MAX_WRITE_SIZE as u32, // use a max write size that fits into the session's buffer

                    // Maximum number of pending "background" requests. A background request is any type of request for which the total number is not limited by other means. As of kernel 4.8, only two types of requests fall into this category:

                    // Read-ahead requests
                    // Asynchronous direct I/O requests
                    // Read-ahead requests are generated (if max_readahead is non-zero) by the kernel to preemptively fill its caches when it anticipates that userspace will soon read more data.

                    // Asynchronous direct I/O requests are generated if FUSE_CAP_ASYNC_DIO is enabled and userspace submits a large direct I/O request. In this case the kernel will internally split it up into multiple smaller requests and submit them to the filesystem concurrently.

                    // Note that the following requests are not background requests: writeback requests (limited by the kernel's flusher algorithm), regular (i.e., synchronous and buffered) userspace read/write requests (limited to one per thread), asynchronous read requests (Linux's io_submit(2) call actually blocks, so these are also limited to one per thread).
                    #[cfg(feature = "abi-7-13")]
                    max_background: 32,

                    // Kernel congestion threshold parameter. If the number of pending background requests exceeds this number, the FUSE kernel module will mark the filesystem as "congested". This instructs the kernel to expect that queued requests will take some time to complete, and to adjust its algorithms accordingly (e.g. by putting a waiting thread to sleep instead of using a busy-loop).
                    #[cfg(feature = "abi-7-13")]
                    congestion_threshold: 30,
                };
                debug!(
                    "INIT response: ABI {}.{}, flags {:#x}, max readahead {}, max write {}",
                    init.major, init.minor, init.flags, init.max_readahead, init.max_write
                );
                se.initialized.store(true, Ordering::Relaxed);
                reply.ok(&init);
            }
            // Any operation is invalid before initialization
            _ if !se.initialized.load(Ordering::Relaxed) => {
                warn!("Ignoring FUSE operation before init: {}", req.request);
                req.reply::<ReplyEmpty>().error(EIO);
            }
            // Filesystem destroyed
            ll::Operation::Destroy => {
                se.filesystem.destroy(req).await;
                se.destroyed.store(true, Ordering::Relaxed);
                req.reply::<ReplyEmpty>().ok();
            }
            // Any operation is invalid after destroy
            _ if se.destroyed.load(Ordering::Relaxed) => {
                warn!("Ignoring FUSE operation after destroy: {}", req.request);
                req.reply::<ReplyEmpty>().error(EIO);
            }

            ll::Operation::Interrupt { .. } => {
                // TODO: handle FUSE_INTERRUPT
                req.reply::<ReplyEmpty>().error(ENOSYS);
            }

            ll::Operation::Lookup { name } => {
                se.filesystem
                    .lookup(req, req.request.nodeid(), &name, req.reply())
                    .await;
            }
            ll::Operation::Forget { arg } => {
                se.filesystem
                    .forget(req, req.request.nodeid(), arg.nlookup)
                    .await; // no reply
            }
            ll::Operation::GetAttr => {
                se.filesystem
                    .getattr(req, req.request.nodeid(), req.reply())
                    .await;
            }
            ll::Operation::SetAttr { arg } => {
                let mode = match arg.valid & FATTR_MODE {
                    0 => None,
                    _ => Some(arg.mode),
                };
                let uid = match arg.valid & FATTR_UID {
                    0 => None,
                    _ => Some(arg.uid),
                };
                let gid = match arg.valid & FATTR_GID {
                    0 => None,
                    _ => Some(arg.gid),
                };
                let size = match arg.valid & FATTR_SIZE {
                    0 => None,
                    _ => Some(arg.size),
                };
                let atime = match arg.valid & FATTR_ATIME {
                    0 => None,
                    _ => Some(UNIX_EPOCH + Duration::new(arg.atime, arg.atimensec)),
                };
                let mtime = match arg.valid & FATTR_MTIME {
                    0 => None,
                    _ => Some(UNIX_EPOCH + Duration::new(arg.mtime, arg.mtimensec)),
                };
                let fh = match arg.valid & FATTR_FH {
                    0 => None,
                    _ => Some(arg.fh),
                };
                #[cfg(target_os = "macos")]
                #[inline]
                fn get_macos_setattr(
                    arg: &fuse_setattr_in,
                ) -> (
                    Option<SystemTime>,
                    Option<SystemTime>,
                    Option<SystemTime>,
                    Option<u32>,
                ) {
                    let crtime = match arg.valid & FATTR_CRTIME {
                        0 => None,
                        _ => Some(UNIX_EPOCH + Duration::new(arg.crtime, arg.crtimensec)),
                    };
                    let chgtime = match arg.valid & FATTR_CHGTIME {
                        0 => None,
                        _ => Some(UNIX_EPOCH + Duration::new(arg.chgtime, arg.chgtimensec)),
                    };
                    let bkuptime = match arg.valid & FATTR_BKUPTIME {
                        0 => None,
                        _ => Some(UNIX_EPOCH + Duration::new(arg.bkuptime, arg.bkuptimensec)),
                    };
                    let flags = match arg.valid & FATTR_FLAGS {
                        0 => None,
                        _ => Some(arg.flags),
                    };
                    (crtime, chgtime, bkuptime, flags)
                }
                #[cfg(not(target_os = "macos"))]
                #[inline]
                fn get_macos_setattr(
                    _arg: &fuse_setattr_in,
                ) -> (
                    Option<SystemTime>,
                    Option<SystemTime>,
                    Option<SystemTime>,
                    Option<u32>,
                ) {
                    (None, None, None, None)
                }
                let (crtime, chgtime, bkuptime, flags) = get_macos_setattr(arg);
                se.filesystem
                    .setattr(
                        req,
                        req.request.nodeid(),
                        mode,
                        uid,
                        gid,
                        size,
                        atime,
                        mtime,
                        fh,
                        crtime,
                        chgtime,
                        bkuptime,
                        flags,
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::ReadLink => {
                se.filesystem
                    .readlink(req, req.request.nodeid(), req.reply())
                    .await;
            }
            ll::Operation::MkNod { arg, name } => {
                se.filesystem
                    .mknod(
                        req,
                        req.request.nodeid(),
                        &name,
                        arg.mode,
                        arg.rdev,
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::MkDir { arg, name } => {
                se.filesystem
                    .mkdir(req, req.request.nodeid(), &name, arg.mode, req.reply())
                    .await;
            }
            ll::Operation::Unlink { name } => {
                se.filesystem
                    .unlink(req, req.request.nodeid(), &name, req.reply())
                    .await;
            }
            ll::Operation::RmDir { name } => {
                se.filesystem
                    .rmdir(req, req.request.nodeid(), &name, req.reply())
                    .await;
            }
            ll::Operation::SymLink { name, link } => {
                se.filesystem
                    .symlink(
                        req,
                        req.request.nodeid(),
                        &name,
                        &Path::new(link),
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::Rename { arg, name, newname } => {
                se.filesystem
                    .rename(
                        req,
                        req.request.nodeid(),
                        &name,
                        arg.newdir,
                        &newname,
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::Link { arg, name } => {
                se.filesystem
                    .link(req, arg.oldnodeid, req.request.nodeid(), &name, req.reply())
                    .await;
            }
            ll::Operation::Open { arg } => {
                se.filesystem
                    .open(req, req.request.nodeid(), arg.flags, req.reply())
                    .await;
            }
            ll::Operation::Read { arg } => {
                se.filesystem
                    .read(
                        req,
                        req.request.nodeid(),
                        arg.fh,
                        arg.offset as i64,
                        arg.size,
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::Write { arg, data } => {
                assert!(data.len() == arg.size as usize);
                se.filesystem
                    .write(
                        req,
                        req.request.nodeid(),
                        arg.fh,
                        arg.offset as i64,
                        data,
                        arg.write_flags,
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::Flush { arg } => {
                se.filesystem
                    .flush(
                        req,
                        req.request.nodeid(),
                        arg.fh,
                        arg.lock_owner,
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::Release { arg } => {
                let flush = match arg.release_flags & FUSE_RELEASE_FLUSH {
                    0 => false,
                    _ => true,
                };
                se.filesystem
                    .release(
                        req,
                        req.request.nodeid(),
                        arg.fh,
                        arg.flags,
                        arg.lock_owner,
                        flush,
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::FSync { arg } => {
                let datasync = match arg.fsync_flags & 1 {
                    0 => false,
                    _ => true,
                };
                se.filesystem
                    .fsync(req, req.request.nodeid(), arg.fh, datasync, req.reply())
                    .await;
            }
            ll::Operation::OpenDir { arg } => {
                se.filesystem
                    .opendir(req, req.request.nodeid(), arg.flags, req.reply())
                    .await;
            }
            ll::Operation::ReadDir { arg } => {
                se.filesystem
                    .readdir(
                        req,
                        req.request.nodeid(),
                        arg.fh,
                        arg.offset as i64,
                        ReplyDirectory::new(req.request.unique(), req.ch, arg.size as usize),
                    )
                    .await;
            }
            ll::Operation::ReleaseDir { arg } => {
                se.filesystem
                    .releasedir(req, req.request.nodeid(), arg.fh, arg.flags, req.reply())
                    .await;
            }
            ll::Operation::FSyncDir { arg } => {
                let datasync = match arg.fsync_flags & 1 {
                    0 => false,
                    _ => true,
                };
                se.filesystem
                    .fsyncdir(req, req.request.nodeid(), arg.fh, datasync, req.reply())
                    .await;
            }
            ll::Operation::StatFs => {
                se.filesystem
                    .statfs(req, req.request.nodeid(), req.reply())
                    .await;
            }
            ll::Operation::SetXAttr { arg, name, value } => {
                assert!(value.len() == arg.size as usize);
                #[cfg(target_os = "macos")]
                #[inline]
                fn get_position(arg: &fuse_setxattr_in) -> u32 {
                    arg.position
                }
                #[cfg(not(target_os = "macos"))]
                #[inline]
                fn get_position(_arg: &fuse_setxattr_in) -> u32 {
                    0
                }
                se.filesystem
                    .setxattr(
                        req,
                        req.request.nodeid(),
                        name,
                        value,
                        arg.flags,
                        get_position(arg),
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::GetXAttr { arg, name } => {
                se.filesystem
                    .getxattr(req, req.request.nodeid(), name, arg.size, req.reply())
                    .await;
            }
            ll::Operation::ListXAttr { arg } => {
                se.filesystem
                    .listxattr(req, req.request.nodeid(), arg.size, req.reply())
                    .await;
            }
            ll::Operation::RemoveXAttr { name } => {
                se.filesystem
                    .removexattr(req, req.request.nodeid(), name, req.reply())
                    .await;
            }
            ll::Operation::Access { arg } => {
                se.filesystem
                    .access(req, req.request.nodeid(), arg.mask, req.reply())
                    .await;
            }
            ll::Operation::Create { arg, name } => {
                se.filesystem
                    .create(
                        req,
                        req.request.nodeid(),
                        &name,
                        arg.mode,
                        arg.flags,
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::GetLk { arg } => {
                se.filesystem
                    .getlk(
                        req,
                        req.request.nodeid(),
                        arg.fh,
                        arg.owner,
                        arg.lk.start,
                        arg.lk.end,
                        arg.lk.typ,
                        arg.lk.pid,
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::SetLk { arg } => {
                se.filesystem
                    .setlk(
                        req,
                        req.request.nodeid(),
                        arg.fh,
                        arg.owner,
                        arg.lk.start,
                        arg.lk.end,
                        arg.lk.typ,
                        arg.lk.pid,
                        false,
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::SetLkW { arg } => {
                se.filesystem
                    .setlk(
                        req,
                        req.request.nodeid(),
                        arg.fh,
                        arg.owner,
                        arg.lk.start,
                        arg.lk.end,
                        arg.lk.typ,
                        arg.lk.pid,
                        true,
                        req.reply(),
                    )
                    .await;
            }
            ll::Operation::BMap { arg } => {
                se.filesystem
                    .bmap(
                        req,
                        req.request.nodeid(),
                        arg.blocksize,
                        arg.block,
                        req.reply(),
                    )
                    .await;
            }
            #[cfg(feature = "abi-7-11")]
            ll::Operation::IoCtl { .. } => {
                let reply: ReplyRaw<fuse_init_out> = req.reply();
                reply.error(libc::ENOSYS)
            }
            #[cfg(feature = "abi-7-11")]
            ll::Operation::Poll { .. } => {
                let reply: ReplyRaw<fuse_init_out> = req.reply();
                reply.error(libc::ENOSYS)
            }
            #[cfg(feature = "abi-7-15")]
            ll::Operation::NotifyReply { .. } => {
                let reply: ReplyRaw<fuse_init_out> = req.reply();
                reply.error(libc::ENOSYS)
            }
            #[cfg(feature = "abi-7-16")]
            ll::Operation::BatchForget { .. } => {
                let reply: ReplyRaw<fuse_init_out> = req.reply();
                reply.error(libc::ENOSYS)
            }
            #[cfg(feature = "abi-7-19")]
            ll::Operation::FAllocate { .. } => {
                let reply: ReplyRaw<fuse_init_out> = req.reply();
                reply.error(libc::ENOSYS)
            }
            #[cfg(feature = "abi-7-12")]
            ll::Operation::CuseInit { .. } => {
                let reply: ReplyRaw<fuse_init_out> = req.reply();
                reply.error(libc::ENOSYS)
            }
            #[cfg(target_os = "macos")]
            ll::Operation::SetVolName { name } => {
                se.filesystem.setvolname(req, name, req.reply()).await;
            }
            #[cfg(target_os = "macos")]
            ll::Operation::GetXTimes => {
                se.filesystem
                    .getxtimes(req, req.request.nodeid(), req.reply())
                    .await;
            }
            #[cfg(target_os = "macos")]
            ll::Operation::Exchange {
                arg,
                oldname,
                newname,
            } => {
                se.filesystem
                    .exchange(
                        req,
                        arg.olddir,
                        &oldname,
                        arg.newdir,
                        &newname,
                        arg.options,
                        req.reply(),
                    )
                    .await;
            }
        }
    }

    /// Create a reply object for this request that can be passed to the filesystem
    /// implementation and makes sure that a request is replied exactly once
    fn reply<T: Reply>(&self) -> T {
        Reply::new(self.request.unique(), self.ch)
    }

    /// Returns the unique identifier of this request
    #[inline]
    #[allow(dead_code)]
    pub fn unique(&self) -> u64 {
        self.request.unique()
    }

    /// Returns the uid of this request
    #[inline]
    #[allow(dead_code)]
    pub fn uid(&self) -> u32 {
        self.request.uid()
    }

    /// Returns the gid of this request
    #[inline]
    #[allow(dead_code)]
    pub fn gid(&self) -> u32 {
        self.request.gid()
    }

    /// Returns the pid of this request
    #[inline]
    #[allow(dead_code)]
    pub fn pid(&self) -> u32 {
        self.request.pid()
    }
}
