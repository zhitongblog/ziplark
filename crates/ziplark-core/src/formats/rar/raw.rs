//! A direct binding to libunrar.
//!
//! The obvious thing to use here is the `unrar` crate, and this module used to.
//! Three problems made that untenable for an archiver that wants RAR to be its
//! strongest format:
//!
//! 1. **Multi-volume archives read out of bounds.** Its volume-change callback
//!    copies a fixed 2048 wide characters out of a `std::wstring` that is
//!    usually a few dozen long (`unrar` 0.5.8, `open_archive.rs:519`), which
//!    rustc's UB checks catch the moment an archive crosses a volume boundary.
//!    There is no newer release.
//! 2. **A failed entry takes the archive with it.** Its type-state API consumes
//!    the handle to process an entry, so an entry that fails to decompress
//!    leaves nothing to continue from — while libunrar itself is perfectly
//!    happy to read the next header. Getting what is readable out of a damaged
//!    archive is most of the point of reading RAR at all.
//! 3. **The struct layouts underneath are wrong.** libunrar's `dll.hpp` is
//!    compiled under `#pragma pack(push, 1)`, so its structs have no padding;
//!    `unrar_sys` 0.5.8 declares them as plain `#[repr(C)]`, which agrees only
//!    up to the first pointer that wants aligning. From there every offset is
//!    four bytes out, and its copy of the header struct is missing four fields
//!    the current library has. That is why its comment buffer silently never
//!    worked, and why the wrapper exposes nothing past `file_attr`: reading
//!    further reads the wrong bytes, and *writing* further — a comment buffer,
//!    a link-target buffer — hands libunrar a garbage pointer to `memcpy` into.
//!
//! So the structs are declared here, packed, matching `dll.hpp` field for
//! field; only the function symbols come from `unrar_sys`. That also opens up
//! everything the wrapper dropped: per-entry packed size, 100-nanosecond
//! timestamps instead of two-second MS-DOS ones, the target of a symlink, names
//! longer than 1023 characters, and which volume libunrar is asking for when
//! one is missing.

use crate::error::{Error, Result};
use std::os::raw::{c_char, c_int};
use std::path::{Path, PathBuf};
use unrar_sys as sys;

/// libunrar's callback: `(message, user data, param1, param2) -> continue?`
type Callback = extern "C" fn(sys::UINT, sys::LPARAM, sys::LPARAM, sys::LPARAM) -> c_int;

/// `RAROpenArchiveDataEx` from `dll.hpp`, packed.
#[repr(C, packed)]
struct OpenData {
    arc_name: *const c_char,
    arc_name_w: *const sys::WCHAR,
    open_mode: u32,
    open_result: u32,
    cmt_buf: *mut c_char,
    cmt_buf_size: u32,
    cmt_size: u32,
    cmt_state: u32,
    flags: u32,
    callback: Option<Callback>,
    user_data: sys::LPARAM,
    op_flags: u32,
    cmt_buf_w: *mut sys::WCHAR,
    reserved: [u32; 25],
}

/// `RARHeaderDataEx` from `dll.hpp`, packed. 14 KiB of it, so it is allocated
/// once per archive and reused for every entry.
#[repr(C, packed)]
struct HeaderData {
    arc_name: [c_char; 1024],
    arc_name_w: [sys::WCHAR; 1024],
    file_name: [c_char; 1024],
    file_name_w: [sys::WCHAR; 1024],
    flags: u32,
    pack_size: u32,
    pack_size_high: u32,
    unp_size: u32,
    unp_size_high: u32,
    host_os: u32,
    file_crc: u32,
    file_time: u32,
    unp_ver: u32,
    method: u32,
    file_attr: u32,
    cmt_buf: *mut c_char,
    cmt_buf_size: u32,
    cmt_size: u32,
    cmt_state: u32,
    dict_size: u32,
    hash_type: u32,
    hash: [u8; 32],
    redir_type: u32,
    redir_name: *mut sys::WCHAR,
    redir_name_size: u32,
    dir_target: u32,
    mtime_low: u32,
    mtime_high: u32,
    ctime_low: u32,
    ctime_high: u32,
    atime_low: u32,
    atime_high: u32,
    arc_name_ex: *mut sys::WCHAR,
    arc_name_ex_size: u32,
    file_name_ex: *mut sys::WCHAR,
    file_name_ex_size: u32,
    reserved: [u32; 982],
}

// Packed, these are exactly the sum of their fields — which is what the C side
// computes too. If either ever fails, a layout has drifted from `dll.hpp` again
// and every field past the drift is being read from the wrong offset.
const _: () = assert!(std::mem::size_of::<OpenData>() == 176);
const _: () = assert!(std::mem::size_of::<HeaderData>() == 14_340);

/// libunrar's redirection kinds, from `dll.hpp`.
const FSREDIR_UNIXSYMLINK: u32 = 1;
const FSREDIR_WINSYMLINK: u32 = 2;
const FSREDIR_JUNCTION: u32 = 3;
const FSREDIR_HARDLINK: u32 = 4;
const FSREDIR_FILECOPY: u32 = 5;

/// What the archive is opened for. Listing skips entry data, which makes it
/// much faster; processing can test or extract each entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    List,
    Process,
}

/// What a link entry points at, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// A symlink: the target is a string the OS resolves when the link is used.
    Symlink,
    /// A hard link to another entry of this same archive.
    HardLink,
    /// RAR5 deduplication: this entry's data *is* another entry's data.
    Copy,
}

/// One entry's header.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub packed_size: u64,
    /// CRC32, when that is the checksum this entry carries. RAR5 uses BLAKE2sp
    /// for larger files, which does not fit in 32 bits and is reported as
    /// absent rather than faked.
    pub crc32: Option<u32>,
    /// Modification time as unix seconds.
    pub modified: Option<i64>,
    /// The entry's attribute word, as the host OS that wrote it meant it.
    pub attr: u32,
    /// Which kind of system wrote the entry — unix mode bits in `attr` only
    /// mean anything if it was a unix-like one.
    pub host_os: u32,
    pub encrypted: bool,
    /// This entry's data began in an earlier volume.
    pub split_before: bool,
    /// This entry's data runs on into the next volume.
    pub split_after: bool,
    /// Where this entry points, when it is a link rather than a file.
    pub link: Option<(LinkKind, String)>,
}

impl Entry {
    pub fn split(&self) -> bool {
        self.split_before || self.split_after
    }

    /// The unix permission bits this entry was stored with, if it was written
    /// on a system that has them.
    pub fn unix_mode(&self) -> Option<u32> {
        // HOST_UNIX = 3, HOST_BEOS = 5 in RAR's numbering. A Windows host puts
        // DOS attribute flags in this word instead, and reading those as a mode
        // produces nonsense — 0 among other things, which would extract files
        // nobody can open.
        if !matches!(self.host_os, 3 | 5) {
            return None;
        }
        // RAR5 stores the mode as-is; RAR4 stores it in the high half, with DOS
        // attributes in the low one. Whichever half carries the file-type bits
        // (`S_IFMT`) is the one that is a mode.
        [self.attr, self.attr >> 16]
            .into_iter()
            .find(|half| half & 0o170000 != 0)
            .map(|mode| mode & 0o7777)
    }
}

/// Mutable state libunrar reaches back into through the callback.
///
/// Boxed so its address is stable: libunrar keeps the pointer we hand it at
/// open time for the life of the archive.
struct State {
    /// The password, wide and NUL-terminated, handed over when libunrar asks.
    password: Option<Vec<sys::WCHAR>>,
    /// The volume libunrar asked for and could not find.
    missing_volume: Option<String>,
}

/// An open RAR archive.
pub struct Archive {
    handle: *const sys::Handle,
    flags: u32,
    comment: Option<String>,
    state: Box<State>,
    header: Box<HeaderData>,
    /// Buffers libunrar writes into: a link target, and a name too long for the
    /// fixed 1024-character field.
    link_buf: Vec<sys::WCHAR>,
    name_buf: Vec<sys::WCHAR>,
    /// Set once a header has been read and not yet processed; libunrar wants
    /// exactly one `RARProcessFile` between reads.
    pending: bool,
}

// The handle is only ever used from the thread that owns the `Archive` — there
// is no `&self` method that touches it — and libunrar keeps no thread-local
// state for it. That is what lets an extraction run on a worker thread.
unsafe impl Send for Archive {}

impl Archive {
    pub fn open(
        path: &Path,
        mode: Mode,
        password: Option<&str>,
        keep_broken: bool,
    ) -> Result<Self> {
        let name = platform::archive_name(path)
            .ok_or_else(|| Error::other(format!("{}: path is not usable", path.display())))?;

        let mut state = Box::new(State {
            password: password.map(|p| {
                let mut wide: Vec<sys::WCHAR> = p.chars().map(|c| c as u32 as sys::WCHAR).collect();
                wide.push(0);
                wide
            }),
            missing_volume: None,
        });

        // 64 KiB of characters, the ceiling RAR itself puts on a comment.
        let mut comment_buf = vec![0 as sys::WCHAR; 64 * 1024];
        let mut data = OpenData {
            arc_name: std::ptr::null(),
            arc_name_w: std::ptr::null(),
            open_mode: match mode {
                Mode::List => sys::RAR_OM_LIST,
                Mode::Process => sys::RAR_OM_EXTRACT,
            },
            open_result: 0,
            cmt_buf: std::ptr::null_mut(),
            cmt_buf_size: comment_buf.len() as u32,
            cmt_size: 0,
            cmt_state: 0,
            flags: 0,
            // The callback has to be in place *before* the open: a
            // header-encrypted archive needs the password to read its file
            // names at all.
            callback: Some(callback),
            user_data: &mut *state as *mut State as sys::LPARAM,
            // Keep the part of a damaged file that did decompress instead of
            // deleting it on failure.
            op_flags: if keep_broken { sys::ROADOF_KEEPBROKEN } else { 0 },
            cmt_buf_w: comment_buf.as_mut_ptr(),
            reserved: [0; 25],
        };
        platform::set_name(&mut data, &name);

        // SAFETY: `data` points at buffers that outlive the call, and libunrar
        // copies everything it keeps except the comment buffer, which it has
        // filled by the time the call returns.
        let handle = unsafe { sys::RAROpenArchiveEx(&mut data as *mut OpenData as *const _) };
        let (open_result, flags, cmt_state, cmt_size) =
            (data.open_result, data.flags, data.cmt_state, data.cmt_size);
        if handle.is_null() {
            return Err(open_error(open_result as i32, path));
        }

        let comment = (cmt_state == 1 && cmt_size > 1)
            .then(|| {
                let chars = (cmt_size as usize - 1).min(comment_buf.len());
                platform::wide_to_string(&comment_buf[..chars])
            })
            .map(|c| c.trim_end_matches(['\r', '\n', '\0']).to_string())
            .filter(|c| !c.is_empty());

        Ok(Self {
            handle,
            flags,
            comment,
            state,
            header: new_header(),
            link_buf: vec![0 as sys::WCHAR; 2048],
            name_buf: vec![0 as sys::WCHAR; 4096],
            pending: false,
        })
    }

    /// Entries share one compression stream.
    pub fn is_solid(&self) -> bool {
        self.flags & sys::ROADF_SOLID != 0
    }

    /// Carries a recovery record.
    pub fn has_recovery_record(&self) -> bool {
        self.flags & sys::ROADF_RECOVERY != 0
    }

    /// Even the file names are encrypted.
    pub fn has_encrypted_headers(&self) -> bool {
        self.flags & sys::ROADF_ENCHEADERS != 0
    }

    /// Locked against modification.
    pub fn is_locked(&self) -> bool {
        self.flags & sys::ROADF_LOCK != 0
    }

    pub fn comment(&self) -> Option<&str> {
        self.comment.as_deref()
    }

    /// Read the next entry's header, or `None` at the end of the archive.
    pub fn read_header(&mut self) -> Result<Option<Entry>> {
        debug_assert!(!self.pending, "read_header called twice without processing");
        // Hand libunrar somewhere to put a link target and an over-long name.
        self.header.redir_name = self.link_buf.as_mut_ptr();
        self.header.redir_name_size = self.link_buf.len() as u32;
        self.header.file_name_ex = self.name_buf.as_mut_ptr();
        self.header.file_name_ex_size = self.name_buf.len() as u32;
        self.header.redir_type = 0;
        self.link_buf[0] = 0;
        self.name_buf[0] = 0;

        // SAFETY: the header and the buffers it points at are owned by `self`
        // and outlive the call.
        let code = unsafe {
            sys::RARReadHeaderEx(self.handle, &mut *self.header as *mut HeaderData as *const _)
        };
        match code {
            sys::ERAR_SUCCESS => {
                self.pending = true;
                Ok(Some(self.entry()))
            }
            sys::ERAR_END_ARCHIVE => Ok(None),
            other => Err(self.error(other)),
        }
    }

    /// Skip the entry whose header was just read.
    pub fn skip(&mut self) -> Result<()> {
        self.process(sys::RAR_SKIP, None)
    }

    /// Decompress the entry and check its checksum, writing nothing.
    pub fn test(&mut self) -> Result<()> {
        self.process(sys::RAR_TEST, None)
    }

    /// Extract the entry to exactly this path.
    pub fn extract_to(&mut self, dest: &Path) -> Result<()> {
        let dest = platform::archive_name(dest)
            .ok_or_else(|| Error::other(format!("{}: path is not usable", dest.display())))?;
        self.process(sys::RAR_EXTRACT, Some(&dest))
    }

    fn process(&mut self, op: i32, dest: Option<&platform::Name>) -> Result<()> {
        self.pending = false;
        let code = platform::process_file(self.handle, op, dest);
        if code == sys::ERAR_SUCCESS {
            Ok(())
        } else {
            Err(self.error(code))
        }
    }

    /// Turn a libunrar status into an engine error, preferring the more
    /// specific "a volume is missing" whenever the callback saw that happen.
    fn error(&self, code: i32) -> Error {
        if let Some(missing) = &self.state.missing_volume {
            return Error::MissingVolume(PathBuf::from(missing));
        }
        match code {
            sys::ERAR_MISSING_PASSWORD => Error::PasswordRequired,
            sys::ERAR_BAD_PASSWORD => Error::BadPassword,
            sys::ERAR_BAD_DATA => Error::corrupt("checksum failed — this entry's data is damaged"),
            sys::ERAR_BAD_ARCHIVE => Error::corrupt("the archive's headers are damaged"),
            sys::ERAR_UNKNOWN_FORMAT => Error::UnsupportedFormat(None),
            sys::ERAR_EOPEN => Error::other("could not open the next volume of this archive"),
            sys::ERAR_ECREATE => Error::other("could not create the output file"),
            sys::ERAR_EREAD => Error::corrupt("the archive is truncated"),
            sys::ERAR_EWRITE => Error::other("could not write the output file"),
            sys::ERAR_EREFERENCE => Error::other(
                "this entry only references another file in the archive, which was not extracted",
            ),
            sys::ERAR_NO_MEMORY => Error::other("out of memory unpacking this entry"),
            other => Error::other(format!("libunrar error {other}")),
        }
    }

    /// Read the header buffer libunrar just filled.
    fn entry(&self) -> Entry {
        let h = &*self.header;
        let flags = h.flags;
        // `file_name_ex` carries the name when it does not fit the fixed field.
        let name = match wide_at(self.name_buf.as_ptr(), self.name_buf.len()) {
            n if !n.is_empty() => n,
            _ => wide_at(
                std::ptr::addr_of!(h.file_name_w) as *const sys::WCHAR,
                1024,
            ),
        };
        let hash_type = h.hash_type;
        let crc = h.file_crc;
        Entry {
            name: name.replace('\\', "/"),
            is_dir: flags & sys::RHDF_DIRECTORY != 0,
            size: pair(h.unp_size, h.unp_size_high),
            packed_size: pair(h.pack_size, h.pack_size_high),
            crc32: (hash_type == sys::RAR_HASH_CRC32 || hash_type == sys::RAR_HASH_NONE)
                .then_some(crc),
            // RAR5 records 100-nanosecond Windows time; RAR4 only the MS-DOS
            // stamp, which has two-second resolution and no time zone.
            modified: filetime_to_unix(h.mtime_low, h.mtime_high)
                .or_else(|| dos_to_unix(h.file_time)),
            attr: h.file_attr,
            host_os: h.host_os,
            encrypted: flags & sys::RHDF_ENCRYPTED != 0,
            split_before: flags & sys::RHDF_SPLITBEFORE != 0,
            split_after: flags & sys::RHDF_SPLITAFTER != 0,
            link: self.link(),
        }
    }

    fn link(&self) -> Option<(LinkKind, String)> {
        let kind = match self.header.redir_type {
            FSREDIR_UNIXSYMLINK | FSREDIR_WINSYMLINK | FSREDIR_JUNCTION => LinkKind::Symlink,
            FSREDIR_HARDLINK => LinkKind::HardLink,
            FSREDIR_FILECOPY => LinkKind::Copy,
            _ => return None,
        };
        let target = wide_at(self.link_buf.as_ptr(), self.link_buf.len()).replace('\\', "/");
        (!target.is_empty()).then_some((kind, target))
    }
}

impl Drop for Archive {
    fn drop(&mut self) {
        // SAFETY: the handle came from RAROpenArchiveEx and is closed once.
        unsafe { sys::RARCloseArchive(self.handle) };
    }
}

/// A zeroed header buffer. 14 KiB of mostly-reserved space, so it is built on
/// the heap directly rather than on the stack and moved.
fn new_header() -> Box<HeaderData> {
    // SAFETY: every field of `HeaderData` is an integer, a pointer or an array
    // of those, for all of which an all-zero pattern is valid.
    unsafe { Box::new(std::mem::zeroed()) }
}

/// libunrar calls this while it works. It must not panic: unwinding through C++
/// frames is undefined behaviour, so everything here is bounded and
/// allocation-light.
extern "C" fn callback(
    msg: sys::UINT,
    user_data: sys::LPARAM,
    p1: sys::LPARAM,
    p2: sys::LPARAM,
) -> c_int {
    if user_data == 0 {
        return 0;
    }
    // SAFETY: `user_data` is the pointer handed to RAROpenArchiveEx, which
    // points at a boxed `State` owned by the archive that is still open.
    let state = unsafe { &mut *(user_data as *mut State) };

    match msg {
        sys::UCM_CHANGEVOLUMEW => {
            // SAFETY: libunrar passes a NUL-terminated wide string here.
            let name = wide_at(p1 as *const sys::WCHAR, 4096);
            match p2 {
                // RAR_VOL_ASK: the volume is not there and libunrar is asking
                // for it. Nothing can supply one, so record which it wanted and
                // stop, rather than leaving the caller with a bare `EOPEN`.
                sys::RAR_VOL_ASK => {
                    state.missing_volume = Some(name);
                    -1
                }
                // RAR_VOL_NOTIFY: it moved on to the next volume by itself,
                // which needs nothing from us.
                _ => 0,
            }
        }
        sys::UCM_NEEDPASSWORDW => {
            let Some(password) = &state.password else {
                // Nothing to give: cancel rather than let libunrar retry with
                // an empty password.
                return -1;
            };
            let capacity = p2 as usize;
            if p1 == 0 || capacity == 0 {
                return -1;
            }
            let buf = p1 as *mut sys::WCHAR;
            // Leave room for the terminator. A password that does not fit is
            // not a password that would have worked.
            let n = password.len().min(capacity) - 1;
            // SAFETY: libunrar gave us a buffer of `capacity` wide characters.
            unsafe {
                for (i, c) in password[..n].iter().enumerate() {
                    buf.add(i).write(*c);
                }
                buf.add(n).write(0);
            }
            0
        }
        _ => 0,
    }
}

/// Read a NUL-terminated wide string, stopping at the terminator and never
/// looking past `max` characters.
///
/// The bound is the point: this is the read the `unrar` crate gets wrong by
/// copying a fixed 2048 characters out of whatever the pointer refers to.
fn wide_at(ptr: *const sys::WCHAR, max: usize) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut out = String::new();
    for i in 0..max {
        // SAFETY: the caller guarantees `max` readable characters, and the loop
        // stops at the NUL libunrar always writes.
        let c = unsafe { ptr.add(i).read() };
        if c == 0 {
            break;
        }
        if let Some(c) = char::from_u32(c as u32) {
            out.push(c);
        }
    }
    out
}

/// libunrar splits 64-bit sizes over two 32-bit fields.
fn pair(low: u32, high: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

/// Windows FILETIME: 100-nanosecond ticks since 1601-01-01 UTC.
fn filetime_to_unix(low: u32, high: u32) -> Option<i64> {
    const TICKS_TO_UNIX_EPOCH: i64 = 116_444_736_000_000_000;
    let ticks = pair(low, high) as i64;
    if ticks == 0 {
        return None;
    }
    Some((ticks - TICKS_TO_UNIX_EPOCH) / 10_000_000)
}

/// MS-DOS timestamp: two-second resolution, no time zone, nothing before 1980.
/// With no zone recorded there is nothing to convert *from*, so the fields are
/// read as UTC — the same choice the ZIP path makes.
fn dos_to_unix(dos: u32) -> Option<i64> {
    if dos == 0 {
        return None;
    }
    let second = ((dos & 0x1F) * 2) as u8;
    let minute = ((dos >> 5) & 0x3F) as u8;
    let hour = ((dos >> 11) & 0x1F) as u8;
    let day = ((dos >> 16) & 0x1F) as u8;
    let month = ((dos >> 21) & 0x0F) as u8;
    let year = ((dos >> 25) & 0x7F) as i32 + 1980;

    let month = time::Month::try_from(month).ok()?;
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    let clock = time::Time::from_hms(hour, minute, second).ok()?;
    Some(date.with_time(clock).assume_utc().unix_timestamp())
}

/// An error from the open call, which has no handle to consult for context.
fn open_error(code: i32, path: &Path) -> Error {
    match code {
        sys::ERAR_MISSING_PASSWORD => Error::PasswordRequired,
        sys::ERAR_BAD_PASSWORD => Error::BadPassword,
        sys::ERAR_BAD_ARCHIVE | sys::ERAR_UNKNOWN_FORMAT => {
            Error::UnsupportedFormat(Some(path.to_path_buf()))
        }
        sys::ERAR_EOPEN => Error::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{}: cannot be opened", path.display()),
        )),
        sys::ERAR_NO_MEMORY => Error::other("out of memory opening this archive"),
        other => Error::corrupt(format!(
            "cannot open this RAR archive (libunrar error {other})"
        )),
    }
}

/// libunrar takes paths as wide strings everywhere except Linux, where the
/// build we link expects bytes.
#[cfg(any(target_os = "linux", target_os = "netbsd"))]
mod platform {
    use super::{sys, OpenData};
    use std::ffi::CString;
    use std::path::Path;

    pub type Name = CString;

    pub fn archive_name(path: &Path) -> Option<Name> {
        CString::new(path.as_os_str().as_encoded_bytes()).ok()
    }

    pub fn set_name(data: &mut OpenData, name: &Name) {
        data.arc_name = name.as_ptr();
    }

    pub fn process_file(handle: *const sys::Handle, op: i32, dest: Option<&Name>) -> i32 {
        let dest = dest.map(|d| d.as_ptr()).unwrap_or(std::ptr::null());
        // SAFETY: both pointers are valid for the call, or null.
        unsafe { sys::RARProcessFile(handle, op, std::ptr::null(), dest) }
    }

    pub fn wide_to_string(buf: &[sys::WCHAR]) -> String {
        buf.iter().filter_map(|c| char::from_u32(*c as u32)).collect()
    }
}

#[cfg(not(any(target_os = "linux", target_os = "netbsd")))]
mod platform {
    use super::{sys, OpenData};
    use std::path::Path;
    use widestring::WideCString;

    pub type Name = WideCString;

    pub fn archive_name(path: &Path) -> Option<Name> {
        WideCString::from_os_str(path).ok()
    }

    pub fn set_name(data: &mut OpenData, name: &Name) {
        data.arc_name_w = name.as_ptr() as *const sys::WCHAR;
    }

    pub fn process_file(handle: *const sys::Handle, op: i32, dest: Option<&Name>) -> i32 {
        let dest = dest
            .map(|d| d.as_ptr() as *const sys::WCHAR)
            .unwrap_or(std::ptr::null());
        // SAFETY: both pointers are valid for the call, or null.
        unsafe { sys::RARProcessFileW(handle, op, std::ptr::null(), dest) }
    }

    /// `wchar_t` is 16 bits on Windows, so that buffer is UTF-16; everywhere
    /// else it is 32 bits and holds code points directly.
    #[cfg(windows)]
    pub fn wide_to_string(buf: &[sys::WCHAR]) -> String {
        let units: Vec<u16> = buf.iter().map(|c| *c as u16).collect();
        String::from_utf16_lossy(&units)
    }

    #[cfg(not(windows))]
    pub fn wide_to_string(buf: &[sys::WCHAR]) -> String {
        buf.iter().filter_map(|c| char::from_u32(*c as u32)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_reassembled_from_their_halves() {
        assert_eq!(pair(1464303715, 1), 5_759_271_011);
        assert_eq!(pair(0, 0), 0);
    }

    #[test]
    fn windows_filetime_converts() {
        // 2026-09-15 20:36:10 UTC, in FILETIME ticks.
        let ticks = (1_789_504_570i64 * 10_000_000) + 116_444_736_000_000_000;
        let (low, high) = (ticks as u32, (ticks >> 32) as u32);
        assert_eq!(filetime_to_unix(low, high), Some(1_789_504_570));
        assert_eq!(filetime_to_unix(0, 0), None);
    }

    #[test]
    fn dos_timestamps_decode() {
        let dos = ((2026 - 1980) << 25) | (9 << 21) | (15 << 16) | (20 << 11) | (36 << 5) | 5;
        let secs = dos_to_unix(dos).unwrap();
        let dt = time::OffsetDateTime::from_unix_timestamp(secs).unwrap();
        assert_eq!((dt.year(), dt.month() as u8, dt.day()), (2026, 9, 15));
        assert_eq!((dt.hour(), dt.minute(), dt.second()), (20, 36, 10));
        assert_eq!(dos_to_unix(0), None);
    }

    #[test]
    fn unix_modes_are_only_read_from_unix_hosts() {
        let mut e = Entry {
            name: "x".into(),
            is_dir: false,
            size: 0,
            packed_size: 0,
            crc32: None,
            modified: None,
            attr: 0o100755,
            host_os: 3,
            encrypted: false,
            split_before: false,
            split_after: false,
            link: None,
        };
        // RAR5 keeps the mode as it is.
        assert_eq!(e.unix_mode(), Some(0o755));
        // RAR4 keeps it in the high half, with DOS attributes below.
        e.attr = (0o100644 << 16) | 0x20;
        assert_eq!(e.unix_mode(), Some(0o644));
        // A Windows host puts DOS attribute bits there, not a mode at all.
        e.host_os = 2;
        assert_eq!(e.unix_mode(), None);
    }
}
