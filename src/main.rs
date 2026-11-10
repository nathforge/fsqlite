//! Mount a SQLite database read-only as a directory tree:
//!   <mount>/<table>/<primary key>/<column>   (file contents = cell value)
//! Composite keys are percent-encoded per part and joined with ',': <mount>/<table>/<k1>,<k2>/<column>

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use fuser::{
    Config, Errno, FileAttr, FileHandle, FileType, Filesystem, Generation, INodeNo, LockOwner,
    MountOption, OpenFlags, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use rusqlite::{types::ValueRef, Connection, OpenFlags as SqlFlags};

/// How long the kernel may cache name lookups and file attributes (size, mode)
/// before asking us again. Also bounds how stale a changed row can appear.
const KERNEL_CACHE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, PartialEq, Eq, Hash)]
enum Node {
    Root,
    Table(String),
    Row(String, Vec<String>),
    Col(String, Vec<String>, String),
}

#[derive(Default)]
struct Inodes {
    nodes: Vec<Node>,
    index: HashMap<Node, u64>,
}

impl Inodes {
    fn id(&mut self, n: Node) -> u64 {
        if let Some(&i) = self.index.get(&n) {
            return i;
        }
        self.nodes.push(n.clone());
        let i = self.nodes.len() as u64; // root is 1
        self.index.insert(n, i);
        i
    }
    fn get(&self, ino: u64) -> Option<Node> {
        self.nodes.get(ino.checked_sub(1)? as usize).cloned()
    }
}

struct SqliteFs {
    db: Mutex<Connection>,
    inodes: Mutex<Inodes>,
    uid: u32,
    gid: u32,
    /// Pretty-print JSON cells no larger than this many bytes; `None` = off.
    pretty_json_max: Option<usize>,
}

fn q(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn value_bytes(v: ValueRef, pretty_json_max: Option<usize>) -> Vec<u8> {
    let mut b = match v {
        ValueRef::Null => return Vec::new(),
        ValueRef::Integer(i) => i.to_string().into_bytes(),
        ValueRef::Real(f) => f.to_string().into_bytes(),
        ValueRef::Text(t) => {
            if let Some(max) = pretty_json_max {
                if t.len() <= max {
                    if let Some(p) = pretty(t) {
                        return p;
                    }
                }
            }
            t.to_vec()
        }
        ValueRef::Blob(b) => return b.to_vec(), // raw, no newline
    };
    b.push(b'\n');
    b
}

/// If `text` is a JSON object or array, return it pretty-printed (with a
/// trailing newline). Scalars are left alone: a text cell like `42` or `true`
/// is far more likely to be plain text than JSON.
fn pretty(text: &[u8]) -> Option<Vec<u8>> {
    let first = text.iter().copied().find(|b| !b.is_ascii_whitespace())?;
    if first != b'{' && first != b'[' {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(text).ok()?;
    let mut out = serde_json::to_vec_pretty(&v).ok()?;
    out.push(b'\n');
    Some(out)
}

fn value_name(v: ValueRef) -> String {
    match v {
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        ValueRef::Blob(b) => b.iter().map(|x| format!("{x:02x}")).collect(),
        ValueRef::Null => "NULL".into(),
    }
}

/// Everything except RFC 3986 unreserved characters is percent-encoded.
const UNSAFE: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

fn encode_name(s: &str) -> String {
    match s {
        "." => "%2E".into(),
        ".." => "%2E%2E".into(),
        _ => utf8_percent_encode(s, UNSAFE).to_string(),
    }
}

fn join_key(parts: &[String]) -> String {
    parts.iter().map(|p| encode_name(p)).collect::<Vec<_>>().join(",")
}

fn decode_name(s: &str) -> Option<String> {
    Some(percent_decode_str(s).decode_utf8().ok()?.into_owned())
}

impl SqliteFs {
    fn tables(&self) -> Vec<String> {
        let db = self.db.lock().unwrap();
        let mut st = db
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap();
        st.query_map([], |r| r.get(0)).unwrap().flatten().collect()
    }

    fn columns(&self, table: &str) -> Vec<String> {
        let db = self.db.lock().unwrap();
        let Ok(mut st) = db.prepare(&format!("PRAGMA table_info({})", q(table))) else {
            return vec![];
        };
        st.query_map([], |r| r.get(1)).unwrap().flatten().collect()
    }

    /// Primary key columns in key order, or rowid if there is no declared key.
    fn pk_cols(&self, table: &str) -> Vec<String> {
        let db = self.db.lock().unwrap();
        let Ok(mut st) = db.prepare(&format!("PRAGMA table_info({})", q(table))) else {
            return vec!["rowid".into()];
        };
        let mut pks: Vec<(i64, String)> = st
            .query_map([], |r| Ok((r.get::<_, i64>(5)?, r.get::<_, String>(1)?)))
            .unwrap()
            .flatten()
            .filter(|(pk, _)| *pk > 0)
            .collect();
        pks.sort();
        if pks.is_empty() { vec!["rowid".into()] } else { pks.into_iter().map(|(_, n)| n).collect() }
    }

    fn pks(&self, table: &str) -> Vec<Vec<String>> {
        let cols = self.pk_cols(table);
        let list = cols.iter().map(|c| q(c)).collect::<Vec<_>>().join(", ");
        let db = self.db.lock().unwrap();
        let Ok(mut st) = db.prepare(&format!("SELECT {list} FROM {}", q(table))) else {
            return vec![];
        };
        st.query_map([], |r| {
            (0..cols.len()).map(|i| Ok(value_name(r.get_ref(i)?))).collect()
        })
        .unwrap()
        .flatten()
        .collect()
    }

    fn row_exists(&self, table: &str, key: &[String]) -> bool {
        self.cell(table, key, None).is_some()
    }

    /// Cell contents; with `col == None` just tests row existence (empty vec).
    fn cell(&self, table: &str, key: &[String], col: Option<&str>) -> Option<Vec<u8>> {
        let pks = self.pk_cols(table);
        if pks.len() != key.len() {
            return None;
        }
        let cond = pks
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{} = ?{}", q(c), i + 1))
            .collect::<Vec<_>>()
            .join(" AND ");
        let db = self.db.lock().unwrap();
        let sel = col.map(q).unwrap_or_else(|| "1".into());
        let sql = format!("SELECT {sel} FROM {} WHERE {cond}", q(table));
        let mut st = db.prepare(&sql).ok()?;
        let mut rows = st.query(rusqlite::params_from_iter(key)).ok()?;
        let row = rows.next().ok()??;
        Some(if col.is_some() { value_bytes(row.get_ref(0).ok()?, self.pretty_json_max) } else { vec![] })
    }

    fn exists(&self, n: &Node) -> bool {
        match n {
            Node::Root => true,
            Node::Table(t) => self.tables().contains(t),
            Node::Row(t, k) => self.tables().contains(t) && self.row_exists(t, k),
            Node::Col(t, k, c) => self.columns(t).contains(c) && self.row_exists(t, k),
        }
    }

    fn attr(&self, ino: u64, n: &Node) -> FileAttr {
        let (kind, perm, size) = match n {
            Node::Col(t, k, c) => (
                FileType::RegularFile,
                0o444,
                self.cell(t, k, Some(c)).map_or(0, |v| v.len() as u64),
            ),
            _ => (FileType::Directory, 0o555, 0),
        };
        FileAttr {
            ino: INodeNo(ino),
            size,
            blocks: size.div_ceil(512),
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind,
            perm,
            nlink: if kind == FileType::Directory { 2 } else { 1 },
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    fn children(&self, n: &Node) -> Vec<(String, Node)> {
        match n {
            Node::Root => self.tables().into_iter().map(|t| (encode_name(&t), Node::Table(t))).collect(),
            Node::Table(t) => {
                self.pks(t).into_iter().map(|k| (join_key(&k), Node::Row(t.clone(), k))).collect()
            }
            Node::Row(t, k) => self
                .columns(t)
                .into_iter()
                .map(|c| (encode_name(&c), Node::Col(t.clone(), k.clone(), c)))
                .collect(),
            Node::Col(..) => vec![],
        }
    }
}

impl Filesystem for SqliteFs {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let Some(parent) = self.inodes.lock().unwrap().get(parent.0) else {
            return reply.error(Errno::ENOENT);
        };
        let Some(raw) = name.to_str() else { return reply.error(Errno::ENOENT) };
        let decoded = decode_name(raw);
        let child = match (&parent, decoded) {
            (Node::Root, Some(n)) => Node::Table(n),
            (Node::Table(t), _) => {
                let parts: Option<Vec<String>> = raw.split(',').map(decode_name).collect();
                match parts {
                    Some(p) => Node::Row(t.clone(), p),
                    None => return reply.error(Errno::ENOENT),
                }
            }
            (Node::Row(t, k), Some(n)) => Node::Col(t.clone(), k.clone(), n),
            (Node::Col(..), _) => return reply.error(Errno::ENOTDIR),
            _ => return reply.error(Errno::ENOENT),
        };
        if !self.exists(&child) {
            return reply.error(Errno::ENOENT);
        }
        let ino = self.inodes.lock().unwrap().id(child.clone());
        reply.entry(&KERNEL_CACHE_TIMEOUT, &self.attr(ino, &child), Generation(0));
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match self.inodes.lock().unwrap().get(ino.0) {
            Some(n) if self.exists(&n) => reply.attr(&KERNEL_CACHE_TIMEOUT, &self.attr(ino.0, &n)),
            _ => reply.error(Errno::ENOENT),
        }
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        match self.inodes.lock().unwrap().get(ino.0) {
            Some(Node::Col(t, k, c)) => match self.cell(&t, &k, Some(&c)) {
                Some(v) => {
                    let start = (offset as usize).min(v.len());
                    let end = (start + size as usize).min(v.len());
                    reply.data(&v[start..end]);
                }
                None => reply.error(Errno::ENOENT),
            },
            Some(_) => reply.error(Errno::EISDIR),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let Some(node) = self.inodes.lock().unwrap().get(ino.0) else {
            return reply.error(Errno::ENOENT);
        };
        if matches!(node, Node::Col(..)) {
            return reply.error(Errno::ENOTDIR);
        }
        let mut entries = vec![
            (ino.0, FileType::Directory, ".".to_string()),
            (ino.0, FileType::Directory, "..".to_string()),
        ];
        for (name, child) in self.children(&node) {
            let kind = if matches!(child, Node::Col(..)) { FileType::RegularFile } else { FileType::Directory };
            let id = self.inodes.lock().unwrap().id(child);
            entries.push((id, kind, name));
        }
        for (i, (id, kind, name)) in entries.into_iter().enumerate().skip(offset as usize) {
            if reply.add(INodeNo(id), (i + 1) as u64, kind, name) {
                break;
            }
        }
        reply.ok();
    }
}

const DEFAULT_PRETTY_JSON_MAX: usize = 1 << 20;

/// Parse "1048576", "512K", "1M", "2G" (powers of 1024).
fn parse_size(s: &str) -> Option<usize> {
    let (num, mult) = match s.chars().last()?.to_ascii_uppercase() {
        'K' => (&s[..s.len() - 1], 1usize << 10),
        'M' => (&s[..s.len() - 1], 1 << 20),
        'G' => (&s[..s.len() - 1], 1 << 30),
        _ => (s, 1),
    };
    num.parse::<usize>().ok()?.checked_mul(mult)
}

const USAGE: &str = "usage: fsqlite [--pretty-json[=true|false]] [--pretty-json-max-size=<size>] \
<database.sqlite> <mountpoint>

options:
  -h, --help               print this help and exit
  -V, --version            print version and exit
  --pretty-json            indent JSON objects/arrays found in text cells (default: off)
  --pretty-json-max-size   largest cell to prettify, in bytes; K/M/G suffixes allowed
                           (default: 1M). Larger cells are shown as stored.";

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2);
}

fn main() {
    let mut pretty_json = false;
    let mut pretty_json_max = DEFAULT_PRETTY_JSON_MAX;
    let mut positional = Vec::new();
    for arg in std::env::args_os().skip(1) {
        match arg.to_str() {
            Some("-h" | "--help") => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            Some("-V" | "--version") => {
                println!("fsqlite {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            Some("--pretty-json" | "--pretty-json=true") => pretty_json = true,
            Some("--pretty-json=false") => pretty_json = false,
            Some(a) if a.starts_with("--pretty-json-max-size=") => {
                let v = &a["--pretty-json-max-size=".len()..];
                pretty_json_max = parse_size(v).unwrap_or_else(|| {
                    eprintln!("fsqlite: invalid size: {v}");
                    usage()
                });
            }
            Some(a) if a.starts_with("--") => {
                eprintln!("fsqlite: unknown or invalid option: {a}");
                usage();
            }
            _ => positional.push(arg),
        }
    }
    let [db_path, mount] = <[_; 2]>::try_from(positional).unwrap_or_else(|_| usage());
    let db = Connection::open_with_flags(
        PathBuf::from(&db_path),
        SqlFlags::SQLITE_OPEN_READ_ONLY | SqlFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("failed to open database");

    let mut inodes = Inodes::default();
    inodes.id(Node::Root);
    let fs = SqliteFs {
        db: Mutex::new(db),
        inodes: Mutex::new(inodes),
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        pretty_json_max: pretty_json.then_some(pretty_json_max),
    };

    let mut cfg = Config::default();
    cfg.mount_options.extend([MountOption::RO, MountOption::FSName("fsqlite".into())]);
    #[cfg(target_os = "macos")]
    let mount_cpath = std::ffi::CString::new(
        std::fs::canonicalize(&mount).expect("mountpoint").into_os_string().into_encoded_bytes(),
    )
    .unwrap();
    #[cfg(target_os = "macos")]
    let session = {
        // macFUSE >= 5.3.3 defers the mount, which breaks the fd-only
        // `fuse_mount_compat25` that fuser uses. Mount via `fuse_mount` instead.
        let fd = macfuse::mount(&mount, &["ro", "fsname=fsqlite"]).expect("mount failed");
        let acl = cfg.acl;
        fuser::Session::from_fd(fs, fd, acl, cfg).expect("mount failed")
    };
    #[cfg(not(target_os = "macos"))]
    let mut session = fuser::Session::new(fs, &mount, &cfg).expect("mount failed");

    // Unmount on Ctrl+C / SIGTERM / SIGHUP. Without this the process dies and
    // leaves a dead mount behind ("Device not configured").
    #[cfg(target_os = "macos")]
    let unmount = {
        move || {
            if unsafe { libc::unmount(mount_cpath.as_ptr(), 0) } != 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    };
    #[cfg(not(target_os = "macos"))]
    let unmount = {
        let mut unmounter = session.unmount_callable();
        move || unmounter.unmount()
    };
    ctrlc::set_handler(move || {
        if let Err(e) = unmount() {
            eprintln!("fsqlite: unmount failed: {e} (is something using the mount?)");
            std::process::exit(1);
        }
    })
    .expect("failed to install signal handler");

    eprintln!(
        "Mounted {} to {}. Press Ctrl+C to exit and unmount",
        PathBuf::from(&db_path).display(),
        PathBuf::from(&mount).display(),
    );
    session.run().expect("filesystem error");
    eprintln!("Unmounted {}", PathBuf::from(&mount).display());
}

#[cfg(target_os = "macos")]
mod macfuse {
    use std::ffi::{c_char, c_int, c_void, CString};
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    #[repr(C)]
    struct FuseArgs {
        argc: c_int,
        argv: *const *const c_char,
        allocated: c_int,
    }

    unsafe extern "C" {
        fn fuse_mount(mountpoint: *const c_char, args: *const FuseArgs) -> *mut c_void;
        fn fuse_chan_fd(ch: *mut c_void) -> c_int;
    }

    /// Mount via libfuse's channel API and return a dup of the device fd.
    pub fn mount(mountpoint: &std::ffi::OsStr, options: &[&str]) -> std::io::Result<OwnedFd> {
        let mp = CString::new(mountpoint.as_bytes()).unwrap();
        let mut owned = vec![CString::new("fsqlite").unwrap()];
        for o in options {
            owned.push(CString::new("-o").unwrap());
            owned.push(CString::new(*o).unwrap());
        }
        let argv: Vec<*const c_char> = owned.iter().map(|s| s.as_ptr()).collect();
        let args = FuseArgs { argc: argv.len() as c_int, argv: argv.as_ptr(), allocated: 0 };
        let ch = unsafe { fuse_mount(mp.as_ptr(), &args) };
        if ch.is_null() {
            return Err(std::io::Error::other("fuse_mount failed"));
        }
        let fd = unsafe { libc::dup(fuse_chan_fd(ch)) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}
