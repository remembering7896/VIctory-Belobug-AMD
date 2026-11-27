//! Views of title memory the layer keeps in its own books instead of asking
//! the kernel to map them.
//!
//! The title's batch page-mapping copies (`CopyPageMappingsBatchX`) ask for
//! thousands of pages at a time, a kernel `map` each: one call of 3,693 pages
//! cost 214 ms in a measured run. Most of those pages are never touched. A
//! soft view is noted here with the page it shows, mapped only when something
//! touches it, and let go when the title unmaps it.
//!
//! The touch usually arrives as an access violation on a thread that may
//! itself hold the title's view table (a non-reentrant mutex), so the faulting
//! thread never maps: it hands the page to the maker thread and parks on a
//! latch, which takes the table once the faulting thread lets it go. The wait
//! is bounded: a page that does not come back in [`PATIENCE`] is reported as
//! what it is, a thread waiting for a lock it holds, instead of a freeze with
//! no line in the log.
//!
//! `VICTORY_SOFT_VIEWS`: unset or `0` notes nothing. `copies` notes and maps
//! on touch.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};

use crate::watch::{Bits, PAGE};

/// How long a faulting thread waits for its page before saying so.
const PATIENCE: std::time::Duration = std::time::Duration::from_millis(500);

/// Where a soft view's bytes come from.
pub trait Books: Send + Sync {
    /// Reads `to` from the title's memory, where `at` is the page of the view
    /// and `from` the page it shows; false if that page is not the title's.
    fn read(&self, from: usize, at: usize, to: &mut [u8]) -> bool;
    /// Makes the kernel map `len` bytes at `at`, showing `from`. Takes the
    /// title's view table: only ever called on the maker thread.
    fn map(&self, at: usize, from: usize, len: usize) -> bool;
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    Copies,
}

static BOOKS: OnceLock<&'static dyn Books> = OnceLock::new();
/// Whether notes are kept at all.
static ON: AtomicBool = AtomicBool::new(false);
/// Views noted, not mapped.
static SOFT: Bits = Bits::new();
/// The pages they show, by the view page's address.
static SOURCE: Mutex<Option<std::collections::HashMap<usize, usize>>> = Mutex::new(None);

static NOTED: AtomicU64 = AtomicU64::new(0);
static MADE_REAL: AtomicU64 = AtomicU64::new(0);
static GONE: AtomicU64 = AtomicU64::new(0);
static READ_BYTES: AtomicU64 = AtomicU64::new(0);
static STUCK: AtomicU64 = AtomicU64::new(0);
static FAILED: AtomicU64 = AtomicU64::new(0);

pub fn mode() -> Mode {
    match std::env::var("VICTORY_SOFT_VIEWS").as_deref() {
        Ok("copies") | Ok("1") | Ok("verify") => Mode::Copies,
        _ => Mode::Off,
    }
}

/// Gives the books a reader and a mapper. Called once, by the layer that owns
/// the title's memory records.
pub fn use_books(books: &'static dyn Books) {
    let _ = BOOKS.set(books);
    ON.store(mode() == Mode::Copies, Ordering::Release);
}

#[inline]
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

fn books() -> Option<&'static dyn Books> {
    BOOKS.get().copied()
}

fn block(at: usize) -> usize {
    at / PAGE
}

/// Whether the page at `at` is noted and not mapped.
#[inline]
pub fn any(at: usize) -> bool {
    on() && SOFT.any(block(at), block(at) + 1)
}

fn sources() -> std::sync::MutexGuard<'static, Option<std::collections::HashMap<usize, usize>>> {
    SOURCE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Notes `[at, at + len)` as a soft view showing `[from, from + len)`: the
/// kernel mapping of `at` must be gone already, and `from` must be the title's.
pub fn noted(at: usize, from: usize, len: usize) {
    if !on() || len == 0 {
        return;
    }
    let (first, until) = (block(at), block(at + len - 1) + 1);
    let mut sources = sources();
    let table = sources.get_or_insert_with(Default::default);
    for b in first..until {
        table.insert(b * PAGE, from + (b * PAGE - at));
        SOFT.set(b);
    }
    NOTED.fetch_add((until - first) as u64, Ordering::Relaxed);
}

/// Lets the noted pages of `[at, at + len)` go: the title unmapped them.
pub fn gone(at: usize, len: usize) {
    if !on() || len == 0 {
        return;
    }
    let (first, until) = (block(at), block(at + len - 1) + 1);
    let mut sources = sources();
    for b in first..until {
        SOFT.take(b);
        if let Some(table) = sources.as_mut() {
            table.remove(&(b * PAGE));
        }
    }
    GONE.fetch_add((until - first) as u64, Ordering::Relaxed);
}

/// A page the faulting thread waits for.
struct Request {
    at: usize,
    from: usize,
    len: usize,
    done: Arc<(Mutex<bool>, Condvar)>,
}

static QUEUE: OnceLock<Option<mpsc::Sender<Request>>> = OnceLock::new();

fn queue() -> Option<&'static mpsc::Sender<Request>> {
    QUEUE.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Request>();
        std::thread::Builder::new()
            .name("soft pages".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    let ok = match books() {
                        Some(books) => books.map(request.at, request.from, request.len),
                        None => false,
                    };
                    let (done, woke) = &*request.done;
                    *done.lock().unwrap_or_else(|e| e.into_inner()) = ok;
                    woke.notify_all();
                }
            })
            .ok()
            .map(|_| sender)
    })
    .as_ref()
}

/// Asks the maker thread for `len` bytes at `at` showing `from`; `Some(mapped)`
/// once it has run, `None` if it did not come back in [`PATIENCE`].
fn ask(at: usize, from: usize, len: usize) -> Option<bool> {
    let queue = queue()?;
    let done = Arc::new((Mutex::new(false), Condvar::new()));
    queue.send(Request { at, from, len, done: done.clone() }).ok()?;
    let (lock, woke) = &*done;
    let waiting = lock.lock().unwrap_or_else(|e| e.into_inner());
    let (waiting, _) = woke
        .wait_timeout_while(waiting, PATIENCE, |done| !*done)
        .unwrap_or_else(|e| e.into_inner());
    if !*waiting {
        return None;
    }
    Some(true)
}

/// Maps the noted pages of `[at, at + len)` for real; false if one could not be
/// mapped, in which case the fault cannot be taken and saying so is all that is
/// left.
pub fn real(at: usize, len: usize) -> bool {
    if !on() || len == 0 {
        return true;
    }
    let (first, until) = (block(at), block(at + len - 1) + 1);
    let mut sources = sources();
    let mut b = first;
    while b < until {
        if !SOFT.any(b, b + 1) {
            b += 1;
            continue;
        }
        let table = sources.as_ref();
        let Some(&from) = table.and_then(|table| table.get(&(b * PAGE))) else {
            // Noted without a source: nothing to map it from, so it is not
            // ours to hold. Let it go and let the fault go on.
            SOFT.take(b);
            b += 1;
            continue;
        };
        // The pages after it that show the pages after `from`: one ask.
        let mut n = 1;
        while b + n < until {
            let here = (b + n) * PAGE;
            match table.and_then(|table| table.get(&here)) {
                Some(&next) if next == from + n * PAGE => n += 1,
                _ => break,
            }
        }
        let (to, bytes) = (b * PAGE, n * PAGE);
        // Let the table go while the maker takes it: it is its lock.
        drop(sources);
        let asked = ask(to, from, bytes);
        sources = self::sources();
        match asked {
            Some(true) => {
                for k in b..b + n {
                    SOFT.take(k);
                    if let Some(table) = sources.as_mut() {
                        table.remove(&(k * PAGE));
                    }
                }
                MADE_REAL.fetch_add(n as u64, Ordering::Relaxed);
            }
            Some(false) => {
                FAILED.fetch_add(n as u64, Ordering::Relaxed);
                tracing::error!(at = format_args!("{to:#x}"), pages = n, "a soft page could not be mapped");
                return false;
            }
            None => {
                STUCK.fetch_add(n as u64, Ordering::Relaxed);
                tracing::error!(
                    at = format_args!("{to:#x}"),
                    pages = n,
                    "a soft page was not made real in time: a thread holding the title's views asked for it itself, or the maker is gone"
                );
                return false;
            }
        }
        b += n;
    }
    true
}

/// Reads `to` from the books at `at`; false if any page of it is not noted, or
/// the books cannot read it (the caller reads it the old way then).
pub fn read(at: usize, to: &mut [u8]) -> bool {
    if !on() || to.is_empty() {
        return false;
    }
    let Some(books) = books() else {
        return false;
    };
    let mut page = [0u8; PAGE];
    let mut done = 0;
    while done < to.len() {
        let here = at + done;
        let base = here - here % PAGE;
        if !any(base) {
            return false;
        }
        let from = match sources().as_ref().and_then(|table| table.get(&base)) {
            Some(&from) => from,
            None => return false,
        };
        if !books.read(from, base, &mut page) {
            return false;
        }
        let within = here - base;
        let n = (PAGE - within).min(to.len() - done);
        to[done..done + n].copy_from_slice(&page[within..within + n]);
        done += n;
    }
    READ_BYTES.fetch_add(to.len() as u64, Ordering::Relaxed);
    true
}

/// `(noted, made real, let go, bytes read, stuck, failed)` since the start.
pub fn counters() -> (u64, u64, u64, u64, u64, u64) {
    (
        NOTED.load(Ordering::Relaxed),
        MADE_REAL.load(Ordering::Relaxed),
        GONE.load(Ordering::Relaxed),
        READ_BYTES.load(Ordering::Relaxed),
        STUCK.load(Ordering::Relaxed),
        FAILED.load(Ordering::Relaxed),
    )
}
