//! llama.cpp's server inside the app, on a thread of its own, for the iPad: iPadOS lets an app start no other
//! program, so the `llama-server` the desktop starts as a child process cannot run there (docs/ipad-plan.md,
//! option B). It is the same server, built from our fork of llama.cpp, so it takes the same arguments and answers
//! the same HTTP on 127.0.0.1, and what talks to it does not change.
//!
//! Unlike a child process, a crash in it takes the app with it, and the operating system ends an app that uses
//! more memory than it may without a word: see [`available_memory`] for checking that first.

use anyhow::{bail, Context, Result};
use std::ffi::{c_char, c_int, CString};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// How long a stop may take before it is given up on: a reply being written finishes first.
const STOP_WAIT: Duration = Duration::from_secs(30);

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

    /// Stop the server and wait until it has, at most [`STOP_WAIT`]: a model still loading stops loading, and the
    /// model's memory is given back. Returns what it exited with.
    pub fn stop(mut self) -> Result<i32> {
        self.stop_and_wait()
    }

    fn stop_and_wait(&mut self) -> Result<i32> {
        let Some(thread) = self.thread.take() else { return Ok(self.exit.unwrap_or(0)) };
        let started = Instant::now();
        // Asked again until it ends: a stop that comes just as the server starts its loop can be missed once.
        while !thread.is_finished() {
            if started.elapsed() > STOP_WAIT {
                // Its thread is left to end by itself; joining could hang the app.
                bail!("the local model did not stop within {} s", STOP_WAIT.as_secs());
            }
            shutdown();
            std::thread::sleep(Duration::from_millis(50));
        }
        let code = thread.join().unwrap_or(-1);
        self.exit = Some(code);
        Ok(code)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Err(e) = self.stop_and_wait() {
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
