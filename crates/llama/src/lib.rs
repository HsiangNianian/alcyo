//! llama.cpp's server inside the app, on a thread of its own, for the iPad: iPadOS lets an app start no other
//! program, so the `llama-server` the desktop starts as a child process cannot run there (docs/ipad-plan.md,
//! option B). It is the same server, built from our fork of llama.cpp, so it takes the same arguments and answers
//! the same HTTP on 127.0.0.1, and what talks to it does not change.
//!
//! Unlike a child process, a crash in it takes the app with it, and the operating system ends an app that uses
//! more memory than it may without a word: see [`available_memory`] for checking that first.
//!
//! The same libraries also run the meeting search's embedding model in the app, without a server ([`embed`]).

pub mod embed;

pub use embed::Embedder;

use anyhow::{bail, Context, Result};
use std::ffi::{c_char, c_int, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[cfg(not(llama_missing))]
extern "C" {
    fn llama_server_main(argc: c_int, argv: *mut *mut c_char) -> c_int;
    fn llama_server_shutdown();
}

/// Whether this build carries the server: always on the iPad.
pub const BUILT_IN: bool = !cfg!(llama_missing);

/// The server keeps its state in globals, so only one runs at a time.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// How long a stop may take before it is given up on. A reply being written is cut short and a decode under way
/// aborted (the fork's `llama_server_shutdown` from alcyo-b11954-3 on), so it takes a second or two; before that, a
/// reply being sent held the stop until its client gave up, minutes later.
const STOP_WAIT: Duration = Duration::from_secs(30);

/// How long a start waits for a server whose stop was given up on to end by itself, asking it again meanwhile.
const LINGER_WAIT: Duration = Duration::from_secs(60);

/// The thread of a server whose stop was given up on ([`StopTimedOut`]): it still holds the server's globals and
/// the model's memory, so nothing starts until it has ended ([`Server::start`] waits for it).
static LINGERING: Mutex<Option<JoinHandle<i32>>> = Mutex::new(None);

/// A stop that was given up on: the server's thread is left to end by itself, and the next start waits for it.
#[derive(Debug)]
pub struct StopTimedOut(pub Duration);

impl std::fmt::Display for StopTimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the local model did not stop within {} s", self.0.as_secs())
    }
}

impl std::error::Error for StopTimedOut {}

/// A start refused because a server whose stop was given up on is still ending: it can be tried again later.
#[derive(Debug)]
pub struct StillStopping;

impl std::fmt::Display for StillStopping {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the local model is still stopping")
    }
}

impl std::error::Error for StillStopping {}

/// Whether a server whose stop was given up on is still ending.
pub fn stopping() -> bool {
    LINGERING.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref().is_some_and(|t| !t.is_finished())
}

/// Wait at most `limit` for a server whose stop was given up on to end, asking it to again meanwhile. Whether none
/// is left.
pub fn wait_stopped(limit: Duration) -> bool {
    let started = Instant::now();
    loop {
        let mut lingering = LINGERING.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match lingering.take() {
            None => return true,
            Some(thread) if thread.is_finished() => {
                let _ = thread.join();
                return true;
            }
            Some(thread) => {
                *lingering = Some(thread);
                if started.elapsed() >= limit {
                    return false;
                }
            }
        }
        drop(lingering);
        shutdown();
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Set once the app is ending, when the system frees everything anyway: a stop then asks once and does not wait.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// The app is ending (iPadOS's `applicationWillTerminate`): from now on stopping a server does not wait for it, so
/// nothing holds up the end.
pub fn quitting() {
    QUITTING.store(true, Ordering::SeqCst);
    shutdown();
}

/// A server running on a thread of this process. Dropping it stops it.
pub struct Server {
    thread: Option<JoinHandle<i32>>,
    /// What it exited with, once it has.
    exit: Option<i32>,
}

impl Server {
    /// Start the server with `args` as `llama-server` takes them, such as `-m <model> --port <port>`. It returns at
    /// once: the server answers `/health` once it listens, and with 200 once the model has loaded.
    pub fn start<S: AsRef<str>>(args: &[S]) -> Result<Self> {
        if !BUILT_IN {
            bail!("this build has no local model server built in");
        }
        let argv: Vec<CString> = std::iter::once("llama-server")
            .chain(args.iter().map(AsRef::as_ref))
            .map(|a| CString::new(a).context("an argument holds a NUL byte"))
            .collect::<Result<_>>()?;
        // A server whose stop was given up on is waited for first: it holds the globals a new one would use.
        if !wait_stopped(LINGER_WAIT) {
            return Err(StillStopping.into());
        }
        if RUNNING.swap(true, Ordering::SeqCst) {
            bail!("a local model server is already running in this app");
        }
        let thread = std::thread::Builder::new()
            .name("llama-server".into())
            // Secondary threads get 512 KiB on Apple's systems, and the server's main runs deep.
            .stack_size(16 << 20)
            .spawn(move || {
                let mut pointers: Vec<*mut c_char> = argv.iter().map(|a| a.as_ptr().cast_mut()).collect();
                pointers.push(std::ptr::null_mut());
                let code = run(argv.len() as c_int, pointers.as_mut_ptr());
                drop(argv);
                RUNNING.store(false, Ordering::SeqCst);
                code
            });
        match thread {
            Ok(thread) => Ok(Self { thread: Some(thread), exit: None }),
            Err(e) => {
                RUNNING.store(false, Ordering::SeqCst);
                Err(e).context("starting the model server's thread")
            }
        }
    }

    /// What the server exited with, if it has: it does by itself only when it fails, as to load its model.
    pub fn exited(&mut self) -> Option<i32> {
        if self.exit.is_none() && self.thread.as_ref().is_some_and(JoinHandle::is_finished) {
            self.exit = Some(self.thread.take().map_or(-1, |t| t.join().unwrap_or(-1)));
        }
        self.exit
    }

    /// Stop the server and wait until it has, at most [`STOP_WAIT`]: a model still loading stops loading, a reply
    /// being written is cut short, and the model's memory is given back. Returns what it exited with. A stop given up
    /// on is a [`StopTimedOut`], and the next start waits for the server to end.
    pub fn stop(self) -> Result<i32> {
        self.stop_within(STOP_WAIT)
    }

    /// As [`Server::stop`], giving up after `limit`.
    pub fn stop_within(mut self, limit: Duration) -> Result<i32> {
        self.stop_and_wait(limit)
    }

    fn stop_and_wait(&mut self, limit: Duration) -> Result<i32> {
        let Some(thread) = self.thread.take() else { return Ok(self.exit.unwrap_or(0)) };
        if QUITTING.load(Ordering::SeqCst) {
            shutdown();
            return Ok(0);
        }
        let started = Instant::now();
        // Asked again until it ends: a stop that comes just as the server starts its loop can be missed once.
        loop {
            shutdown();
            if thread.is_finished() {
                break;
            }
            if started.elapsed() >= limit {
                // Its thread is left to end by itself, where the next start finds it; joining could hang the app.
                *LINGERING.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(thread);
                return Err(StopTimedOut(limit).into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let code = thread.join().unwrap_or(-1);
        self.exit = Some(code);
        Ok(code)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Err(e) = self.stop_and_wait(STOP_WAIT) {
            eprintln!("{e:#}");
        }
    }
}

#[cfg(not(llama_missing))]
fn run(argc: c_int, argv: *mut *mut c_char) -> i32 {
    // SAFETY: argv holds argc valid C strings and a final null, alive until this returns.
    unsafe { llama_server_main(argc, argv) }
}

#[cfg(llama_missing)]
fn run(_: c_int, _: *mut *mut c_char) -> i32 {
    1
}

fn shutdown() {
    // SAFETY: it may be called from any thread at any time, also when no server runs.
    #[cfg(not(llama_missing))]
    unsafe {
        llama_server_shutdown();
    }
}

/// How much more memory this app may use before iPadOS ends it, in bytes; `None` where that is not known (any
/// system but the iPad's). It depends on the device, what else runs, and whether the app has the
/// increased-memory-limit entitlement.
pub fn available_memory() -> Option<u64> {
    #[cfg(target_os = "ios")]
    {
        extern "C" {
            fn os_proc_available_memory() -> usize;
        }
        // SAFETY: no arguments, and it only reads this process's own limit.
        let bytes = unsafe { os_proc_available_memory() } as u64;
        (bytes > 0).then_some(bytes)
    }
    #[cfg(not(target_os = "ios"))]
    {
        None
    }
}

/// How much memory the device has, in bytes: `hw.memsize` on Apple's systems, `None` elsewhere.
pub fn physical_memory() -> Option<u64> {
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    {
        extern "C" {
            fn sysctlbyname(name: *const c_char, old: *mut std::ffi::c_void, old_len: *mut usize, new: *mut std::ffi::c_void, new_len: usize) -> c_int;
        }
        let mut bytes: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        // SAFETY: the name is a C string and the output a u64 of the length given.
        let ok = unsafe { sysctlbyname(c"hw.memsize".as_ptr(), (&raw mut bytes).cast(), &mut len, std::ptr::null_mut(), 0) } == 0;
        (ok && bytes > 0).then_some(bytes)
    }
    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    {
        None
    }
}
