//! Each guard is built exactly once from a non-null ncnn handle and, in
//! `Drop`, destroys the handle before decrementing its counter.

use std::ffi::CStr;
use std::marker::PhantomData;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use ncnn_sys::{
    ncnn_extractor_destroy, ncnn_extractor_t, ncnn_mat_destroy, ncnn_mat_t, ncnn_net_destroy,
    ncnn_net_t,
};

#[derive(Debug, PartialEq, Eq)]
pub struct LiveHandles {
    pub nets: usize,
    pub extractors: usize,
    pub mats: usize,
}

static NETS: AtomicUsize = AtomicUsize::new(0);
static EXTRACTORS: AtomicUsize = AtomicUsize::new(0);
static MATS: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn live_handles() -> LiveHandles {
    LiveHandles {
        nets: live_nets(),
        extractors: EXTRACTORS.load(Ordering::Relaxed),
        mats: MATS.load(Ordering::Relaxed),
    }
}

pub(crate) fn live_nets() -> usize {
    NETS.load(Ordering::Relaxed)
}

static INSTANCE: Mutex<()> = Mutex::new(());

/// The lock every path that creates or destroys the ncnn instance or a net
/// holds; taken when poisoned too, as it guards ncnn's state, not Rust data.
pub(crate) fn lock_instance() -> MutexGuard<'static, ()> {
    INSTANCE.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the pointer is non-null, and the caller guarantees the pointee
    // is a valid NUL-terminated string for the duration of this call.
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    (!text.is_empty()).then_some(text)
}

#[derive(Debug)]
pub(crate) struct NetGuard {
    handle: ncnn_net_t,
}

impl NetGuard {
    pub(crate) fn new(handle: ncnn_net_t) -> Self {
        NETS.fetch_add(1, Ordering::Relaxed);
        Self { handle }
    }

    pub(crate) const fn handle(&self) -> ncnn_net_t {
        self.handle
    }
}

impl Drop for NetGuard {
    fn drop(&mut self) {
        // SAFETY: the handle came from ncnn_net_create through `new` and this
        // guard destroys it exactly once, after any extractor that borrows it.
        unsafe { ncnn_net_destroy(self.handle) }
        NETS.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub(crate) struct ExtractorGuard<'net> {
    handle: ncnn_extractor_t,
    _net: PhantomData<&'net ()>,
}

impl<'net> ExtractorGuard<'net> {
    pub(crate) fn new(handle: ncnn_extractor_t, net: &'net NetGuard) -> Self {
        let _ = net;
        EXTRACTORS.fetch_add(1, Ordering::Relaxed);
        Self {
            handle,
            _net: PhantomData,
        }
    }

    pub(crate) const fn handle(&self) -> ncnn_extractor_t {
        self.handle
    }
}

impl Drop for ExtractorGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: `new` made this handle, borrowed its net for the extractor's
        // whole life, and destroys it exactly once, here.
        unsafe { ncnn_extractor_destroy(self.handle) }
        EXTRACTORS.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub(crate) struct MatGuard<'a> {
    handle: ncnn_mat_t,
    _borrow: PhantomData<&'a mut ()>,
}

impl<'a> MatGuard<'a> {
    pub(crate) fn external(handle: ncnn_mat_t, buffer: &'a mut [f32]) -> Self {
        let _ = buffer.as_mut_ptr();
        MATS.fetch_add(1, Ordering::Relaxed);
        Self {
            handle,
            _borrow: PhantomData,
        }
    }

    pub(crate) fn extracted(handle: ncnn_mat_t, extractor: &'a ExtractorGuard<'_>) -> Self {
        let _ = extractor;
        MATS.fetch_add(1, Ordering::Relaxed);
        Self {
            handle,
            _borrow: PhantomData,
        }
    }

    pub(crate) const fn handle(&self) -> ncnn_mat_t {
        self.handle
    }
}

impl Drop for MatGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: created by `external` or `extracted`, destroyed exactly once; each
        // borrows the Mat's backing (a buffer ncnn may write, or the extractor).
        unsafe { ncnn_mat_destroy(self.handle) }
        MATS.fetch_sub(1, Ordering::Relaxed);
    }
}
