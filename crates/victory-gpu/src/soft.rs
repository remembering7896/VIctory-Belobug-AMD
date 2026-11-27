//! Views of title memory the layer keeps in its own books instead of asking
//! the kernel to map them.
//!
//! The title's batch page-mapping copies (`CopyPageMappingsBatchX`) ask for
//! thousands of pages at a time, a kernel `map` each: one call of 3,693 pages
//! cost 214 ms in a measured run. Most of those pages are never touched. A
//! soft view is noted here, mapped only when something reads it, and let go
//! when the title unmaps it.
//!
//! `VICTORY_SOFT_VIEWS`: unset or `0` notes nothing (`copies` notes and maps
//! on touch). `verify` also maps each page for real and compares what the
//! books read against what the kernel has, to prove the books do not miss a
//! page.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::watch::{Bits, PAGE};

/// Where a soft view's bytes come from.
pub trait Books: Send + Sync {
    /// Reads up to `to.len()` bytes of the title's memory at `at`; the number
    /// read, or `None` if that page is not the title's to read.
    fn read(&self, at: usize, to: &mut [u8]) -> Option<usize>;
    /// Makes the kernel map `len` bytes at `at` for real; false if it cannot.
    fn map(&self, at: usize, len: usize) -> bool;
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    Copies,
    Verify,
}

static BOOKS: std::sync::OnceLock<&'static dyn Books> = std::sync::OnceLock::new();
/// Views noted, not mapped.
static SOFT: Bits = Bits::new();
/// Of them, ones only ever read.
static SOFT_RO: Bits = Bits::new();

static NOTED: AtomicU64 = AtomicU64::new(0);
static MADE_REAL: AtomicU64 = AtomicU64::new(0);
static GONE: AtomicU64 = AtomicU64::new(0);
static READ_BYTES: AtomicU64 = AtomicU64::new(0);

pub fn mode() -> Mode {
    match std::env::var("VICTORY_SOFT_VIEWS").as_deref() {
        Ok("copies") | Ok("1") => Mode::Copies,
        Ok("verify") => Mode::Verify,
        Ok("0") => Mode::Off,
        _ => Mode::Off,
    }
}

/// Gives the books a reader and a mapper. Called once, by the layer that owns
/// the title's memory records.
pub fn use_books(books: &'static dyn Books) {
    let _ = BOOKS.set(books);
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
    SOFT.any(block(at), block(at) + 1)
}

/// Whether any page of `[at, at + len)` is noted and not mapped.
#[inline]
pub fn any_in(at: usize, len: usize) -> bool {
    if len == 0 {
        return false;
    }
    SOFT.any(block(at), block(at + len - 1) + 1)
}

/// Notes `[at, at + len)` as a soft view: the title's, not mapped.
pub fn noted(at: usize, len: usize, read_only: bool) {
    if mode() == Mode::Off || len == 0 {
        return;
    }
    let (first, until) = (block(at), block(at + len - 1) + 1);
    for b in first..until {
        SOFT.set(b);
        if read_only {
            SOFT_RO.set(b);
        }
    }
    NOTED.fetch_add((until - first) as u64, Ordering::Relaxed);
}

/// Maps the noted pages of `[at, at + len)` for real.
pub fn real(at: usize, len: usize) {
    if len == 0 {
        return;
    }
    let (first, until) = (block(at), block(at + len - 1) + 1);
    let mut any_noted = false;
    for b in first..until {
        if SOFT.take(b) {
            SOFT_RO.clear(b);
            any_noted = true;
        }
    }
    if !any_noted {
        return;
    }
    if let Some(books) = books() {
        if books.map(at, len) {
            MADE_REAL.fetch_add((until - first) as u64, Ordering::Relaxed);
        }
    }
}

/// Lets the noted pages of `[at, at + len)` go: the title unmapped them.
pub fn gone(at: usize, len: usize) {
    if len == 0 {
        return;
    }
    let (first, until) = (block(at), block(at + len - 1) + 1);
    for b in first..until {
        SOFT.take(b);
        SOFT_RO.clear(b);
    }
    GONE.fetch_add((until - first) as u64, Ordering::Relaxed);
}

/// Reads `to` from the books at `at`; false if any page of it is not noted, or
/// the books cannot read it (the caller reads it the old way then).
pub fn read(at: usize, to: &mut [u8]) -> bool {
    if to.is_empty() {
        return false;
    }
    let Some(books) = books() else {
        return false;
    };
    let mut done = 0;
    while done < to.len() {
        let here = at + done;
        if !any(here) {
            return false;
        }
        let n = (PAGE - here % PAGE).min(to.len() - done);
        match books.read(here, &mut to[done..done + n]) {
            Some(got) if got == n => done += n,
            _ => return false,
        }
    }
    READ_BYTES.fetch_add(to.len() as u64, Ordering::Relaxed);
    true
}

/// `(noted, made real, let go, bytes read)` since the start.
pub fn counters() -> (u64, u64, u64, u64) {
    (
        NOTED.load(Ordering::Relaxed),
        MADE_REAL.load(Ordering::Relaxed),
        GONE.load(Ordering::Relaxed),
        READ_BYTES.load(Ordering::Relaxed),
    )
}
