//! Views of title memory the layer keeps in its own books instead of asking
//! the kernel to map them.
//!
//! The title's batch page-mapping copies (`CopyPageMappingsBatchX`) ask for
//! thousands of pages at a time, a kernel `map` each: one call of 3,693 pages
//! cost 214 ms in a measured run. Most of those pages are never touched. A
//! soft view is noted here with the page it shows, mapped only when something
//! touches it, and let go when the title unmaps it.
//!
//! Nothing on this path takes a lock the title's own code can hold: a noted
//! page carries a [`Pin`], which knows its section and its page in it, and a
//! fault or a read asks the pin directly. An earlier version handed the page
//! to a maker thread and parked, and died three times in one startup run with
//! `a thread holding the title's views asked for it itself`: the title reads
//! title memory while holding its view table, and no help thread can take a
//! lock the faulting thread is already holding.
//!
//! `VICTORY_SOFT_VIEWS`: unset or `0` notes nothing. `copies` notes and maps
//! on touch.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::watch::{Bits, PAGE};

/// A page the title's memory shows, enough to read it or map it anywhere
/// without the title's view table.
pub trait Pin: Send + Sync {
    /// Reads `to` from the page; false if it cannot be read that way.
    fn read(&self, to: &mut [u8]) -> bool;
    /// Makes the kernel map this page at `at`; false if it cannot. Takes
    /// `Arc<Self>` so a page that maps can record itself: the view table may
    /// not have it, and a later unmap must still keep its placeholder.
    fn map(self: Arc<Self>, at: usize) -> bool;
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    Copies,
}

/// Views noted, not mapped.
static SOFT: Bits = Bits::new();
/// The pins they show, by the view page's address.
static PINS: Mutex<Option<HashMap<usize, Arc<dyn Pin>>>> = Mutex::new(None);
/// Pages a pin mapped, by address. The title's view table may not have them:
/// a fault must not wait for it. A later unmap asks here instead, so the
/// placeholder it leaves is one a pin can map again.
static MADE_AT: Mutex<Option<HashMap<usize, Arc<dyn Pin>>>> = Mutex::new(None);

/// Notes on: a view made soft, a page made real, a view let go.
static NOTED: AtomicU64 = AtomicU64::new(0);
static MADE_REAL: AtomicU64 = AtomicU64::new(0);
static GONE: AtomicU64 = AtomicU64::new(0);
static READ_BYTES: AtomicU64 = AtomicU64::new(0);
static FAILED: AtomicU64 = AtomicU64::new(0);

pub fn mode() -> Mode {
    match std::env::var("VICTORY_SOFT_VIEWS").as_deref() {
        Ok("copies") | Ok("1") | Ok("verify") => Mode::Copies,
        _ => Mode::Off,
    }
}

#[inline]
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| mode() == Mode::Copies)
}

fn block(at: usize) -> usize {
    at / PAGE
}

fn pins() -> std::sync::MutexGuard<'static, Option<HashMap<usize, Arc<dyn Pin>>>> {
    PINS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether the page at `at` is noted and not mapped.
#[inline]
pub fn any(at: usize) -> bool {
    on() && SOFT.any(block(at), block(at) + 1)
}

/// Notes the page at `at` as a soft view of `pin`: the kernel mapping of `at`
/// must be a placeholder still, and the pin must keep its section alive.
pub fn noted(at: usize, pin: Arc<dyn Pin>) {
    if !on() {
        return;
    }
    let base = at - at % PAGE;
    pins().get_or_insert_with(Default::default).insert(base, pin);
    SOFT.set(block(base));
    NOTED.fetch_add(1, Ordering::Relaxed);
}

/// Lets the noted pages of `[at, at + len)` go: the title unmapped them.
pub fn gone(at: usize, len: usize) {
    if !on() || len == 0 {
        return;
    }
    let (first, until) = (block(at), block(at + len - 1) + 1);
    let mut pins = pins();
    for b in first..until {
        SOFT.take(b);
        if let Some(table) = pins.as_mut() {
            table.remove(&(b * PAGE));
        }
    }
    GONE.fetch_add((until - first) as u64, Ordering::Relaxed);
}

/// Maps the noted pages of `[at, at + len)` for real, from their pins: no lock
/// is taken here, so a fault on a thread that holds the title's view table is
/// served like any other.
pub fn real(at: usize, len: usize) -> bool {
    if !on() || len == 0 {
        return true;
    }
    let (first, until) = (block(at), block(at + len - 1) + 1);
    for b in first..until {
        if !SOFT.any(b, b + 1) {
            continue;
        }
        let base = b * PAGE;
        let pin = pins().as_ref().and_then(|table| table.get(&base)).cloned();
        let Some(pin) = pin else {
            // Noted without a pin: nothing to map it from, so it is not ours
            // to hold. Let it go and let the fault go on.
            SOFT.take(b);
            continue;
        };
        if !pin.clone().map(base) {
            FAILED.fetch_add(1, Ordering::Relaxed);
            tracing::error!(
                at = format_args!("{base:#x}"),
                error = %std::io::Error::last_os_error(),
                "a soft page could not be mapped"
            );
            return false;
        }
        SOFT.take(b);
        if let Some(table) = pins().as_mut() {
            table.remove(&base);
        }
        MADE_REAL.fetch_add(1, Ordering::Relaxed);
    }
    true
}

/// Reads `to` from the books at `at`; false if any page of it is not noted, or
/// the pin cannot read it (the caller reads it the old way then).
pub fn read(at: usize, to: &mut [u8]) -> bool {
    if !on() || to.is_empty() {
        return false;
    }
    let mut page = [0u8; PAGE];
    let mut done = 0;
    while done < to.len() {
        let here = at + done;
        let base = here - here % PAGE;
        if !any(base) {
            return false;
        }
        let pin = pins().as_ref().and_then(|table| table.get(&base)).cloned();
        let Some(pin) = pin else {
            return false;
        };
        if !pin.read(&mut page) {
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

/// Records a page a pin has mapped at `at`.
pub fn made(at: usize, pin: Arc<dyn Pin>) {
    if let Some(table) = made_at().as_mut() {
        table.insert(at - at % PAGE, pin);
    }
}

fn made_at() -> std::sync::MutexGuard<'static, Option<HashMap<usize, Arc<dyn Pin>>>> {
    let mut guard = MADE_AT.lock().unwrap_or_else(|e| e.into_inner());
    guard.get_or_insert_with(Default::default);
    guard
}

/// The addresses in `[at, at + len)` a pin has mapped and nobody has let go:
/// they are mapped now, and their placeholders must be kept when they go.
pub fn taken(at: usize, len: usize) -> Vec<usize> {
    let mut taken = Vec::new();
    if len == 0 {
        return taken;
    }
    if let Some(table) = made_at().as_mut() {
        let (first, until) = (block(at), block(at + len - 1) + 1);
        for b in first..until {
            if table.remove(&(b * PAGE)).is_some() {
                taken.push(b * PAGE);
            }
        }
    }
    taken
}

/// `(noted, made real, let go, bytes read, failed)` since the start.
pub fn counters() -> (u64, u64, u64, u64, u64) {
    (
        NOTED.load(Ordering::Relaxed),
        MADE_REAL.load(Ordering::Relaxed),
        GONE.load(Ordering::Relaxed),
        READ_BYTES.load(Ordering::Relaxed),
        FAILED.load(Ordering::Relaxed),
    )
}
