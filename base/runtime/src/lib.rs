#![feature(f16)]
//! Backend-agnostic functions a compiled Oi program calls at runtime.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString, c_char};
use std::mem::size_of;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI64, Ordering};

// Flush stdout, then fail without a core dump.
fn die() -> ! {
	let _ = std::io::Write::flush(&mut std::io::stdout());
	std::process::exit(101);
}

macro_rules! fatal {
	($($t:tt)*) => {{
		eprintln!($($t)*);
		die()
	}};
}

// A `repr(i64)` enum the JIT passes across the ABI as a raw i64.
trait Decode: Copy {
	const COUNT: i64;
	const WHAT: &str;

	fn from_i64(v: i64) -> Self {
		if !(0..Self::COUNT).contains(&v) {
			fatal!("invalid {}: {v}", Self::WHAT);
		}
		unsafe { std::mem::transmute_copy(&v) }
	}
}

// Type tag shared with the compiler.
#[repr(i64)]
#[derive(Clone, Copy)]
pub enum Tag {
	Bool,
	Int,
	UInt,
	Float,
	Str,
	Raw,
}

impl Decode for Tag {
	const COUNT: i64 = 6;
	const WHAT: &str = "tag";
}

// Output sink for writing.
#[repr(i64)]
#[derive(Clone, Copy)]
pub enum Sink {
	Out,
	Err,
	Buf,
}

impl Decode for Sink {
	const COUNT: i64 = 3;
	const WHAT: &str = "sink";
}

thread_local! {
	static BUF: RefCell<String> = const { RefCell::new(String::new()) };
}

// Route a fragment to its sink.
fn emit(sink: i64, s: &str) {
	match Sink::from_i64(sink) {
		Sink::Out => print!("{s}"),
		Sink::Err => eprint!("{s}"),
		Sink::Buf => BUF.with(|b| b.borrow_mut().push_str(s)),
	}
}

// String header layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct StrHeader {
	data: i64,
	len: i64,
}

/// Read a string handle's bytes, excluding the trailing NUL.
/// # Safety
/// `header` must point to a valid string header.
pub unsafe fn str_bytes<'a>(header: *const StrHeader) -> &'a [u8] {
	let StrHeader { data, len } = unsafe { *header };
	unsafe { bytes(data, len) }
}

// Null-safe byte slice.
unsafe fn bytes<'a>(data: i64, len: i64) -> &'a [u8] {
	if data == 0 || len <= 0 {
		&[]
	} else {
		unsafe { std::slice::from_raw_parts(data as *const u8, len as usize) }
	}
}

// A string handle's bytes.
unsafe fn str_lossy<'a>(header: *const StrHeader) -> std::borrow::Cow<'a, str> {
	String::from_utf8_lossy(unsafe { str_bytes(header) })
}

/// Allocate a fresh string handle owning a copy of `bytes`, plus a trailing NUL for C interop.
pub fn str_new(a: *const Allocator, bytes: &[u8]) -> *const StrHeader {
	let len = bytes.len() as i64;
	unsafe {
		let data = raw_alloc(a, len + 1);
		std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
		str_header(a, data as i64, len)
	}
}

// Strings are never freed, so their handles skip alloc's counted prefix.
unsafe fn str_header(a: *const Allocator, data: i64, len: i64) -> *const StrHeader {
	let out = unsafe { raw_alloc(a, size_of::<StrHeader>() as i64) } as *mut StrHeader;
	unsafe { *out = StrHeader { data, len } };
	out
}

// Render one value to a string.
fn render(tag: Tag, bits: i64, width: i64, quote: bool) -> String {
	match tag {
		Tag::Bool => (bits == 1).to_string(),
		Tag::Int => bits.to_string(),
		Tag::UInt => (bits as u64).to_string(),
		Tag::Float => match width {
			16 => format!("{:?}", f16::from_bits(bits as u16)),
			32 => format!("{:?}", f32::from_bits(bits as u32)),
			_ => format!("{:?}", f64::from_bits(bits as u64)),
		},
		Tag::Str | Tag::Raw => {
			let s = unsafe { str_lossy(bits as *const StrHeader) };
			if quote && matches!(tag, Tag::Str) {
				format!("{s:?}")
			} else {
				s.into_owned()
			}
		}
	}
}

// Write a rendered value fragment.
#[unsafe(export_name = "oi_write")]
pub extern "C" fn write(tag: i64, bits: i64, width: i64, quote: i64, sink: i64) {
	let s = render(Tag::from_i64(tag), bits, width, quote != 0);
	emit(sink, &s);
}

// Write the ", " separator before every element but the first.
#[unsafe(export_name = "oi_write_sep")]
pub extern "C" fn write_sep(i: i64, sink: i64) {
	if i > 0 {
		emit(sink, ", ");
	}
}

/// Panic with an out-of-bounds message.
/// # Safety
/// `ctx` must be null or point to a valid `Context` record, and `at` must be a valid handle.
#[unsafe(export_name = "oi_panic_oob")]
pub unsafe extern "C" fn panic_oob(ctx: *const i64, index: i64, len: i64, at: *const i64) -> ! {
	let text = format!("index out of range: the length is {len} but the index is {index}");
	// borrowed, not allocated
	let msg = StrHeader {
		data: text.as_ptr() as i64,
		len: text.len() as i64,
	};
	unsafe { abort_with(ctx, at, "", &msg) }
}

// Wrap integer exponents.
#[unsafe(export_name = "oi_pow_int")]
pub extern "C" fn pow_int(base: i64, exp: i64) -> i64 {
	if exp < 0 {
		fatal!("negative exponent: {exp}");
	}
	base.wrapping_pow(exp as u32)
}

#[unsafe(export_name = "oi_pow_float")]
pub extern "C" fn pow_float(base: f64, exp: f64) -> f64 {
	base.powf(exp)
}

// Print `{prefix}{msg}` and abort.
unsafe fn abort_with(ctx: *const i64, at: *const i64, prefix: &str, msg: *const StrHeader) -> ! {
	let obj = if ctx.is_null() { 0 } else { unsafe { *ctx.add(3) } };
	if obj != 0 {
		let hook: extern "C" fn(i64, i64, i64, i64) = unsafe { std::mem::transmute(*(obj as *const i64)) };
		hook(msg as i64, at as i64, ctx as i64, obj);
	}
	let msg = unsafe { str_lossy(msg) };
	fatal!("{prefix}{msg}");
}

/// Print an assertion failure message and abort.
/// # Safety
/// `ctx` must be null or a valid `Context` record, and `msg` and `at` must be valid handles.
#[unsafe(export_name = "oi_assert_fail")]
pub unsafe extern "C" fn assert_fail(ctx: *const i64, msg: *const StrHeader, at: *const i64) {
	unsafe { abort_with(ctx, at, "assertion failed: ", msg) }
}

/// Print a panic message and abort.
/// # Safety
/// `ctx` must be null or a valid `Context` record, and `msg` and `at` must be valid handles.
#[unsafe(export_name = "oi_panic")]
pub unsafe extern "C" fn panic(ctx: *const i64, msg: *const StrHeader, at: *const i64) {
	unsafe { abort_with(ctx, at, "panic: ", msg) }
}

/// Report main's error and exit 1.
/// # Safety
/// `msg` must be a valid string handle.
#[unsafe(export_name = "oi_fail")]
pub unsafe extern "C" fn fail(msg: *const StrHeader) {
	eprintln!("error: {}", unsafe { str_lossy(msg) });
	let _ = std::io::Write::flush(&mut std::io::stdout());
	std::process::exit(1);
}

/// Copy a c-string's bytes into a fresh string handle.
/// # Safety
/// `header` must point to a valid array header.
#[unsafe(export_name = "oi_str_from_bytes")]
pub unsafe extern "C" fn str_from_bytes(a: *const Allocator, header: *const Header) -> *const StrHeader {
	let Header { data, len, .. } = unsafe { *header };
	str_new(a, unsafe { bytes(data, len) })
}

/// Compare two string handles.
/// # Safety
/// `a` and `b` must be valid string handles.
#[unsafe(export_name = "oi_str_eq")]
pub unsafe extern "C" fn str_eq(a: *const StrHeader, b: *const StrHeader) -> i64 {
	let a = unsafe { str_bytes(a) };
	let b = unsafe { str_bytes(b) };
	(a == b) as i64
}

/// Concatenate two string handles into a fresh one.
/// # Safety
/// `a` and `b` must be valid string handles.
#[unsafe(export_name = "oi_str_concat")]
pub unsafe extern "C" fn str_concat(
	alloc: *const Allocator,
	a: *const StrHeader,
	b: *const StrHeader,
) -> *const StrHeader {
	let a = unsafe { str_bytes(a) };
	let b = unsafe { str_bytes(b) };
	let mut out = Vec::with_capacity(a.len() + b.len());
	out.extend_from_slice(a);
	out.extend_from_slice(b);
	str_new(alloc, &out)
}

/// NUL-terminated pointer to the string's bytes.
/// For owned strings (already NUL-terminated) this is the buffer as-is, and for views it's a fresh leaked copy.
/// # Safety
/// `header` must point to a valid string header.
#[unsafe(export_name = "oi_str_cstr")]
pub unsafe extern "C" fn str_cstr(header: *const StrHeader) -> i64 {
	let StrHeader { data, len } = unsafe { *header };
	if data != 0 && unsafe { *((data + len) as *const u8) } == 0 {
		return data;
	}
	let mut buf = unsafe { str_bytes(header) }.to_vec();
	buf.push(0);
	Box::leak(buf.into_boxed_slice()).as_ptr() as i64
}

/// Build a string handle by copying a NUL-terminated C string's bytes.
/// # Safety
/// `ptr` must be null or point to a valid NUL-terminated C string.
#[unsafe(export_name = "oi_cstr_str")]
pub unsafe extern "C" fn cstr_str(a: *const Allocator, ptr: i64) -> *const StrHeader {
	if ptr == 0 {
		return str_new(a, &[]);
	}
	let bytes = unsafe { std::ffi::CStr::from_ptr(ptr as *const std::ffi::c_char) }.to_bytes();
	str_new(a, bytes)
}

/// Build a string handle by copying `len` bytes from `data`.
/// # Safety
/// `data` must be null or point to at least `len` readable bytes.
#[unsafe(export_name = "oi_ptr_string")]
pub unsafe extern "C" fn ptr_string(a: *const Allocator, data: i64, len: i64) -> *const StrHeader {
	str_new(a, unsafe { bytes(data, len) })
}

/// Copy bytes from `data` into a fresh rc'd array buffer.
/// # Safety
/// `data` must be null or point to at least `bytes` readable bytes.
#[unsafe(export_name = "oi_ptr_buffer")]
pub unsafe extern "C" fn ptr_buffer(a: *const Allocator, data: i64, bytes: i64) -> i64 {
	let bytes = if data == 0 { 0 } else { bytes.max(0) };
	let buf = unsafe { buffer_alloc(a, bytes) };
	if bytes > 0 {
		unsafe { std::ptr::copy_nonoverlapping(data as *const u8, buf, bytes as usize) };
	}
	buf as i64
}

// Resolve a trait object's field address.
#[unsafe(export_name = "oi_trait_field")]
pub extern "C" fn trait_field(data: i64, off: i64) -> i64 {
	if off & 2 != 0 {
		return off & !2;
	}
	match off & 1 {
		0 => data + off,
		_ => unsafe { *((data + (off & 0xFFFF_FFFE)) as *const i64) + (off >> 32) },
	}
}

// Current buffer length.
#[unsafe(export_name = "oi_str_mark")]
pub extern "C" fn str_mark() -> i64 {
	BUF.with(|b| b.borrow().len() as i64)
}

// Split the buffer tail from `mark` into a fresh string handle.
#[unsafe(export_name = "oi_str_take")]
pub extern "C" fn str_take(a: *const Allocator, mark: i64) -> *const StrHeader {
	BUF.with(|b| str_new(a, b.borrow_mut().split_off(mark as usize).as_bytes()))
}

// Active managed allocations, for leak checks.
static LIVE: AtomicI64 = AtomicI64::new(0);

pub fn leaked() -> i64 {
	LIVE.load(Ordering::Relaxed)
}

type AllocProc = unsafe extern "C" fn(*mut u8, i64, i64, i64, *mut u8, i64) -> *mut u8;

/// `core.Alloc`, a C-callable proc plus the state it owns.
#[repr(C)]
#[derive(Clone, Copy, PartialEq)]
pub struct Allocator {
	proc: i64,
	data: i64,
}

// `proc` modes, shared with core/context.oi.
const ALLOC: i64 = 0;
const FREE: i64 = 1;

// `alloc` prefixes each block with its size and a copy of its allocator, so `free` outlives both ctx and record.
const PREFIX: usize = 24;

fn layout(size: i64) -> std::alloc::Layout {
	std::alloc::Layout::from_size_align(size.max(1) as usize, 8).unwrap()
}

fn record(proc: AllocProc, data: i64) -> *const Allocator {
	Box::leak(Box::new(Allocator {
		proc: proc as i64,
		data,
	}))
}

/// The system heap, and what the root context allocates from.
#[unsafe(export_name = "oi_system_allocator")]
pub extern "C" fn system_allocator() -> *const Allocator {
	static SYSTEM: OnceLock<usize> = OnceLock::new();
	*SYSTEM.get_or_init(|| record(sys_proc, 0) as usize) as *const Allocator
}

/// The system heap, untracked by LIVE, for the root context's defaults.
#[unsafe(export_name = "oi_root_allocator")]
pub extern "C" fn root_allocator() -> *const Allocator {
	static ROOT: OnceLock<usize> = OnceLock::new();
	*ROOT.get_or_init(|| record(sys_proc, 1) as usize) as *const Allocator
}

unsafe extern "C" fn sys_proc(_: *mut u8, mode: i64, size: i64, _: i64, old: *mut u8, old_size: i64) -> *mut u8 {
	match mode {
		ALLOC => unsafe { std::alloc::alloc_zeroed(layout(size)) },
		FREE => unsafe {
			std::alloc::dealloc(old, layout(old_size));
			std::ptr::null_mut()
		},
		_ => std::ptr::null_mut(),
	}
}

/// Drive an allocator's proc.
/// Procs zero what they hand back and return null only on failure.
unsafe fn call_proc(a: *const Allocator, mode: i64, size: i64, old: *mut u8, old_size: i64) -> *mut u8 {
	let p: AllocProc = unsafe { std::mem::transmute((*a).proc) };
	let out = unsafe { p((*a).data as *mut u8, mode, size, 8, old, old_size) };
	if mode == ALLOC && out.is_null() {
		fatal!("allocator returned null for {size} bytes");
	}
	out
}

/// Bytes straight from the given allocator, with no `alloc` prefix.
unsafe fn raw_alloc(a: *const Allocator, size: i64) -> *mut u8 {
	unsafe { call_proc(a, ALLOC, size, std::ptr::null_mut(), 0) }
}

/// Allocate `size` zeroed bytes for a composite value (e.g. a tuple's field slots).
/// # Safety
/// `a` must point to a valid allocator record.
#[unsafe(export_name = "oi_alloc")]
pub unsafe extern "C" fn alloc(a: *const Allocator, size: i64) -> *mut u8 {
	let size = size.max(1) + PREFIX as i64;
	unsafe {
		if *a != *root_allocator() {
			LIVE.fetch_add(1, Ordering::Relaxed);
		}
		let base = raw_alloc(a, size);
		*(base as *mut i64) = size;
		*(base.add(8) as *mut Allocator) = *a;
		base.add(PREFIX)
	}
}

/// Free an `alloc` result through the allocator in its prefix.
/// # Safety
/// `ptr` must be null or a live `alloc` result.
#[unsafe(export_name = "oi_free")]
pub unsafe extern "C" fn free(ptr: *mut u8) {
	if ptr.is_null() {
		return;
	}
	unsafe {
		if *owner(ptr) != *root_allocator() {
			LIVE.fetch_sub(1, Ordering::Relaxed);
		}
		let base = ptr.sub(PREFIX);
		call_proc(owner(ptr), FREE, 0, base, *(base as *const i64));
	}
}

// The allocator an `alloc` result came from.
unsafe fn owner(ptr: *const u8) -> *const Allocator {
	unsafe { ptr.sub(PREFIX).add(8) as *const Allocator }
}

// The refcount sitting before a buffer or box.
unsafe fn rc(p: *const u8) -> *mut i64 {
	unsafe { p.sub(8) as *mut i64 }
}

// Drop one ref, true at zero.
unsafe fn rc_dec(p: *const u8) -> bool {
	unsafe {
		*rc(p) -= 1;
		*rc(p) == 0
	}
}

// Copy `n` elements of `w` bytes.
unsafe fn copy_elems(src: *const u8, dst: *mut u8, n: i64, w: i64) {
	unsafe { std::ptr::copy_nonoverlapping(src, dst, (n * w) as usize) }
}

// Allocate an element buffer with its refcount at data[-8], count starting at 1.
unsafe fn buffer_alloc(a: *const Allocator, bytes: i64) -> *mut u8 {
	unsafe {
		let base = alloc(a, bytes + 8);
		*(base as *mut i64) = 1;
		base.add(8)
	}
}

// Array header layout shared with the compiler (lower/array.rs, offsets 0/8/16).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Header {
	data: i64,
	len: i64,
	cap: i64,
}

// A counted array handle holding the given header.
unsafe fn new_header(a: *const Allocator, h: Header) -> *const Header {
	let out = unsafe { alloc(a, size_of::<Header>() as i64) } as *mut Header;
	unsafe { *out = h };
	out
}

/// Clone a header, sharing its buffer with a refcount bump.
/// # Safety
/// `header` must point to a valid array header.
#[unsafe(export_name = "oi_array_share")]
pub unsafe extern "C" fn array_share(header: *const Header) -> *const Header {
	let h = unsafe { *header };
	if h.data != 0 {
		unsafe { *rc(h.data as *const u8) += 1 };
	}
	unsafe { new_header(owner(header.cast()), h) }
}

/// Drop one ref to an array.
/// The buffer frees at zero.
/// # Safety
/// `header` must be null or point to a valid array header.
#[unsafe(export_name = "oi_array_release")]
pub unsafe extern "C" fn array_release(header: *mut Header) {
	if header.is_null() {
		return;
	}
	let Header { data, .. } = unsafe { *header };
	if data != 0 && unsafe { rc_dec(data as *const u8) } {
		unsafe { free((data - 8) as *mut u8) };
	}
	unsafe { free(header as *mut u8) };
}

/// Give a shared array its own buffer before a write.
/// No-op when the buffer is null or unshared.
/// # Safety
/// `header` must point to a valid array header.
#[unsafe(export_name = "oi_array_cow")]
pub unsafe extern "C" fn array_cow(a: *const Allocator, header: *mut Header, elem_size: i64) {
	let Header { data, len, .. } = unsafe { *header };
	if data == 0 || unsafe { *rc(data as *const u8) } <= 1 {
		return;
	}
	let new_data = unsafe { buffer_alloc(a, len * elem_size) };
	unsafe {
		copy_elems(data as *const u8, new_data, len, elem_size);
		*rc(data as *const u8) -= 1;
		(*header).data = new_data as i64;
		(*header).cap = len;
	}
}

/// A fresh array owning `elems`, each packed into `width` bytes.
pub fn array_of(elems: &[i64], width: i64) -> *const Header {
	let len = elems.len() as i64;
	let data = unsafe { buffer_alloc(system_allocator(), len * width) };
	for (i, v) in elems.iter().enumerate() {
		unsafe {
			std::ptr::copy_nonoverlapping(v.to_le_bytes().as_ptr(), data.add(i * width as usize), width as usize)
		};
	}
	let h = Header {
		data: data as i64,
		len,
		cap: len,
	};
	unsafe { new_header(system_allocator(), h) }
}

/// Read an array header's elements as pointer-sized ints.
/// # Safety
/// `header` must point to a valid array header.
pub unsafe fn array_elems<'a>(header: *const Header) -> &'a [i64] {
	let Header { data, len, .. } = unsafe { *header };
	if data == 0 {
		&[]
	} else {
		unsafe { std::slice::from_raw_parts(data as *const i64, len as usize) }
	}
}

/// Copy the range of an array into a fresh array.
/// Panics if out of range.
/// # Safety
/// `header` must point to a valid array header.
#[unsafe(export_name = "oi_slice")]
pub unsafe extern "C" fn slice(
	a: *const Allocator,
	header: *const Header,
	start: i64,
	end: i64,
	elem_size: i64,
) -> *const Header {
	let Header { data, len, .. } = unsafe { *header };
	if start < 0 || start > end || end > len {
		eprintln!("slice range {start}..{end} out of bounds for array of length {len}");
		die();
	}
	let view_len = end - start;
	let new_data = unsafe { buffer_alloc(a, view_len * elem_size) };
	unsafe { copy_elems((data + start * elem_size) as *const u8, new_data, view_len, elem_size) };
	let h = Header {
		data: new_data as i64,
		len: view_len,
		cap: view_len,
	};
	unsafe { new_header(a, h) }
}

/// View a range of a string through a fresh handle sharing the same buffer.
/// # Safety
/// `header` must point to a valid string header.
#[unsafe(export_name = "oi_str_slice")]
pub unsafe extern "C" fn str_slice(
	a: *const Allocator,
	header: *const StrHeader,
	start: i64,
	end: i64,
) -> *const StrHeader {
	let StrHeader { data, len } = unsafe { *header };
	if start < 0 || start > end || end > len {
		eprintln!("slice range {start}..{end} out of bounds for string of length {len}");
		die();
	}
	unsafe { str_header(a, data + start, end - start) }
}

/// Write a `mut` slice projection back into its parent buffer at `lo`.
/// # Safety
/// `parent` and `src` must point to valid array headers.
#[unsafe(export_name = "oi_array_write_back")]
pub unsafe extern "C" fn array_write_back(parent: *mut Header, lo: i64, len: i64, src: *const Header, elem_size: i64) {
	let Header { data, len: slen, .. } = unsafe { *src };
	if slen != len {
		fatal!("projection changed length: expected {len} elements, got {slen}");
	}
	unsafe {
		copy_elems(
			data as *const u8,
			((*parent).data + lo * elem_size) as *mut u8,
			len,
			elem_size,
		);
	}
}

/// Ensure the array has capacity for at least `min_cap` elements.
/// Grows by doubling, at least to `min_cap`. Updates data and cap in place.
/// # Safety
/// `header` must point to a valid array header.
#[unsafe(export_name = "oi_array_reserve")]
pub unsafe extern "C" fn array_reserve(a: *const Allocator, header: *mut Header, min_cap: i64, elem_size: i64) {
	let Header { data, len, cap } = unsafe { *header };
	if min_cap <= cap {
		return;
	}
	let new_cap = (cap.max(1) * 2).max(min_cap);
	let new_data = unsafe { buffer_alloc(a, new_cap * elem_size) };
	unsafe {
		copy_elems(data as *const u8, new_data, len, elem_size);
		(*header).data = new_data as i64;
		(*header).cap = new_cap;
		if data != 0 {
			free((data - 8) as *mut u8);
		}
	}
}

/// Append all elements of `src` to `dst`, growing dst's buffer as needed.
/// # Safety
/// `dst` and `src` must point to valid array headers.
#[unsafe(export_name = "oi_array_extend")]
pub unsafe extern "C" fn array_extend(a: *const Allocator, dst: *mut Header, src: *const Header, elem_size: i64) {
	let dst_len = unsafe { (*dst).len };
	let Header {
		data: src_data,
		len: src_len,
		..
	} = unsafe { *src };
	unsafe { array_reserve(a, dst, dst_len + src_len, elem_size) };
	unsafe {
		let dst_data = (*dst).data as *mut u8;
		let dst_tail = dst_data.add((dst_len * elem_size) as usize);
		copy_elems(src_data as *const u8, dst_tail, src_len, elem_size);
		(*dst).len = dst_len + src_len;
	}
}

/// Share a `&T`, bumping its refcount.
/// # Safety
/// `ptr` must point to a valid boxed struct's field slots (or be null).
#[unsafe(export_name = "oi_ref_share")]
pub unsafe extern "C" fn ref_share(ptr: *mut u8) -> *mut u8 {
	if ptr.is_null() {
		return ptr;
	}
	unsafe { *rc(ptr) += 1 };
	ptr
}

// Walk a box's trace descriptor, calling `visit` on each live ref slot.
// Array/map handles aren't graph edges, so when `drop` they're only released.
unsafe fn trace(fields: *mut u8, desc: *const i64, drop: bool, visit: &mut dyn FnMut(*mut u8)) {
	if desc.is_null() {
		return;
	}
	unsafe {
		let mut p = desc.add(2);
		for _ in 0..*desc {
			let e = *p;
			p = p.add(1);
			let slot = *(fields.add((e >> 2) as usize) as *const *mut u8);
			if slot.is_null() {
				p = p.add((e & 3 == 1) as usize);
				continue;
			}
			match e & 3 {
				0 => visit(slot),
				1 => {
					trace(slot, *p as *const i64, drop, &mut *visit);
					p = p.add(1);
				}
				2 if drop => array_release(slot.cast()),
				3 if drop => map_release(slot.cast()),
				_ => {}
			}
		}
	}
}

thread_local! {
	// Boxes whose non-zero release marked them as possible cycle roots.
	static ROOTS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
	// Root count that triggers a collection, doubled when one frees under half.
	static THRESHOLD: Cell<usize> = const { Cell::new(10_000) };
}

// The descriptor (box[-16]).
unsafe fn desc(s: *mut u8) -> *const i64 {
	unsafe { *(s.sub(16) as *const *const i64) }
}

/// Drop one ref to a boxed struct.
/// # Safety
/// `ptr` must be null or point to a valid boxed struct's field slots.
#[unsafe(export_name = "oi_ref_release")]
pub unsafe extern "C" fn ref_release(ptr: *mut u8) {
	if ptr.is_null() {
		return;
	}
	unsafe {
		if rc_dec(ptr) {
			// remove freed boxes to avoid `collect_cycles` walking freed memory
			ROOTS.with(|r| r.borrow_mut().remove(&(ptr as usize)));
			trace(ptr, desc(ptr), true, &mut |c| ref_release(c));
			free(ptr.sub(16));
		} else if !desc(ptr).is_null() && *desc(ptr).add(1) != 0 {
			// still alive and can reach a ref, a possible cycle root
			let n = ROOTS.with(|r| {
				let mut r = r.borrow_mut();
				r.insert(ptr as usize);
				r.len()
			});
			if n >= THRESHOLD.get() && collect_cycles() < n / 2 {
				THRESHOLD.set(n * 2);
			}
		}
	}
}

#[derive(PartialEq)]
enum Color {
	Gray,
	White,
	Black,
}

/// Bacon-Rajan synchronous trial deletion over the buffered cyclic roots.
/// Returns how many boxes it freed.
pub fn collect_cycles() -> usize {
	let roots: Vec<usize> = ROOTS.with(|r| r.borrow_mut().drain().collect());
	let mut c = HashMap::new();
	for &s in &roots {
		mark_gray(s as *mut u8, &mut c);
	}
	for &s in &roots {
		scan(s as *mut u8, &mut c);
	}
	let freed = c.values().filter(|&k| *k == Color::White).count();
	for &s in &roots {
		collect_white(s as *mut u8, &mut c);
	}
	freed
}

// Attempt to decrement children, painting the candidate subgraph gray.
fn mark_gray(s: *mut u8, c: &mut HashMap<usize, Color>) {
	if c.insert(s as usize, Color::Gray) == Some(Color::Gray) {
		return;
	}
	unsafe {
		trace(s, desc(s), false, &mut |t| {
			*rc(t) -= 1;
			mark_gray(t, c);
		})
	};
}

// Scan and repaint nodes based on refcounts.
fn scan(s: *mut u8, c: &mut HashMap<usize, Color>) {
	if c.get(&(s as usize)) != Some(&Color::Gray) {
		return;
	}
	if unsafe { *rc(s) } > 0 {
		scan_black(s, c);
	} else {
		c.insert(s as usize, Color::White);
		unsafe { trace(s, desc(s), false, &mut |t| scan(t, c)) };
	}
}

fn scan_black(s: *mut u8, c: &mut HashMap<usize, Color>) {
	c.insert(s as usize, Color::Black);
	unsafe {
		trace(s, desc(s), false, &mut |t| {
			*rc(t) += 1;
			if c.get(&(t as usize)) != Some(&Color::Black) {
				scan_black(t, c);
			}
		})
	};
}

// Free the white subgraph.
fn collect_white(s: *mut u8, c: &mut HashMap<usize, Color>) {
	if c.get(&(s as usize)) != Some(&Color::White) {
		return;
	}
	c.insert(s as usize, Color::Black);
	unsafe {
		trace(s, desc(s), true, &mut |t| collect_white(t, c));
		free(s.sub(16));
	}
}

#[unsafe(export_name = "oi_epilogue")]
pub extern "C" fn epilogue() {
	collect_cycles();
	if std::env::var_os("OI_LEAK_CHECK").is_some() {
		eprintln!("leaked allocations: {}", leaked());
	}
}

// The root context's `core.Logger`.
fn logger_record() -> *const i64 {
	static LOGGER: OnceLock<usize> = OnceLock::new();
	*LOGGER.get_or_init(|| Box::leak(Box::new([0i64; 3])).as_ptr() as usize) as *const i64
}

// The root context's `core.Rng`.
// fastrand's per-thread wyrand, seeded from OS entropy.
fn rng_record() -> *const i64 {
	static RNG: OnceLock<usize> = OnceLock::new();
	*RNG.get_or_init(|| Box::leak(Box::new([rng_proc as *const () as i64, 0])).as_ptr() as usize) as *const i64
}

extern "C" fn rng_proc(_: *mut u8) -> u64 {
	fastrand::u64(..)
}

/// Reseed this thread's root generator. No effect on a custom `ctx.rand`.
#[unsafe(export_name = "oi_rand_seed")]
pub extern "C" fn rand_seed(seed: u64) {
	fastrand::seed(seed)
}

// A slot per `core.Context` field, with room for amendments.
pub const CTX_FIELDS: usize = 28;
thread_local! {
	static CTX_ROOT: RefCell<[i64; CTX_FIELDS]> = const { RefCell::new([0; CTX_FIELDS]) };
}

// The thread's root `core.Context`, bound by `main`, `@test` and `@c` bodies.
#[unsafe(export_name = "oi_ctx_root")]
pub extern "C" fn ctx_root() -> *mut i64 {
	CTX_ROOT.with(|c| {
		let root: *mut i64 = c.as_ptr().cast();
		unsafe {
			if *root == 0 {
				*root = system_allocator() as i64;
				*root.add(2) = logger_record() as i64;
				*root.add(4) = rng_record() as i64;
			}
		}
		root
	})
}

static ARGS: OnceLock<Vec<CString>> = OnceLock::new();

/// Record the process arguments, once, for `os.args`.
/// # Safety
/// `argv` must point to `argc` live NUL-terminated strings.
#[unsafe(export_name = "oi_set_args")]
pub unsafe extern "C" fn set_args(argc: i32, argv: *const *const c_char) {
	let args = (0..argc.max(0)).map(|i| unsafe { CStr::from_ptr(*argv.add(i as usize)) }.to_owned());
	let _ = ARGS.set(args.collect());
}

// Argument count the process was given.
#[unsafe(export_name = "oi_argc")]
pub extern "C" fn argc() -> i64 {
	ARGS.get().map_or(0, |a| a.len() as i64)
}

// The `i`th argument, or null when out of range.
#[unsafe(export_name = "oi_argv")]
pub extern "C" fn argv(i: i64) -> *const c_char {
	let arg = ARGS.get().and_then(|a| a.get(i as usize));
	arg.map_or(std::ptr::null(), |a| a.as_ptr())
}

// Get dirent name.
#[unsafe(export_name = "oi_dirent_name")]
pub extern "C" fn dirent_name(e: *const u8) -> *const c_char {
	let off = if cfg!(target_os = "macos") { 21 } else { 19 };
	e.wrapping_add(off).cast()
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum MapKey {
	Raw(i64),
	Str(Vec<u8>),
}

fn map_key(tag: i64, bits: i64) -> MapKey {
	match Tag::from_i64(tag) {
		Tag::Str => MapKey::Str(unsafe { str_bytes(bits as *const StrHeader) }.to_vec()),
		_ => MapKey::Raw(bits),
	}
}

pub struct OiMap {
	entries: HashMap<MapKey, i64>,
	rc: i64,
}

// Only the box routes through the given allocator. The entries stay on the heap.
unsafe fn map_box(a: *const Allocator, map: OiMap) -> *mut OiMap {
	unsafe {
		let out = alloc(a, size_of::<OiMap>() as i64) as *mut OiMap;
		std::ptr::write(out, map);
		out
	}
}

#[unsafe(export_name = "oi_map_new")]
pub unsafe extern "C" fn map_new(a: *const Allocator) -> *mut OiMap {
	unsafe {
		map_box(
			a,
			OiMap {
				entries: HashMap::new(),
				rc: 1,
			},
		)
	}
}

/// Drop one ref to a map.
/// The box frees at zero.
/// # Safety
/// `map` must be null or a valid live `OiMap` pointer.
#[unsafe(export_name = "oi_map_release")]
pub unsafe extern "C" fn map_release(map: *mut OiMap) {
	if map.is_null() {
		return;
	}
	unsafe { (*map).rc -= 1 };
	if unsafe { (*map).rc } == 0 {
		unsafe {
			std::ptr::drop_in_place(map);
			free(map.cast());
		}
	}
}

/// Share a map handle.
/// RC bump.
/// # Safety
/// `map` must be a valid, live `OiMap` pointer.
#[unsafe(export_name = "oi_map_share")]
pub unsafe extern "C" fn map_share(map: *mut OiMap) -> *mut OiMap {
	unsafe { (*map).rc += 1 };
	map
}

// Give a shared map its own entries before a write.
unsafe fn map_cow(a: *const Allocator, map: *mut OiMap) -> *mut OiMap {
	if unsafe { (*map).rc } <= 1 {
		return map;
	}
	unsafe { (*map).rc -= 1 };
	let entries = unsafe { (*map).entries.clone() };
	unsafe { map_box(a, OiMap { entries, rc: 1 }) }
}

/// # Safety
/// `map` must be a valid, live `OiMap` pointer.
#[unsafe(export_name = "oi_map_get")]
pub unsafe extern "C" fn map_get(map: *mut OiMap, tag: i64, bits: i64) -> i64 {
	let map = unsafe { &*map };
	match map.entries.get(&map_key(tag, bits)) {
		Some(v) => *v,
		None => {
			fatal!("key not found in map");
		}
	}
}

/// Set a map entry, cloning shared entries first.
/// # Safety
/// `map` must be a valid, live `OiMap` pointer.
#[unsafe(export_name = "oi_map_set")]
pub unsafe extern "C" fn map_set(a: *const Allocator, map: *mut OiMap, tag: i64, bits: i64, value: i64) -> *mut OiMap {
	let map = unsafe { map_cow(a, map) };
	unsafe { &mut *map }.entries.insert(map_key(tag, bits), value);
	map
}

/// Remove a map entry if present, cloning shared entries first.
/// # Safety
/// `map` must be a valid, live `OiMap` pointer.
#[unsafe(export_name = "oi_map_delete")]
pub unsafe extern "C" fn map_delete(a: *const Allocator, map: *mut OiMap, tag: i64, bits: i64) -> *mut OiMap {
	let map = unsafe { map_cow(a, map) };
	unsafe { &mut *map }.entries.remove(&map_key(tag, bits));
	map
}

/// The number of entries in a map.
/// # Safety
/// `map` must be a valid, live `OiMap` pointer.
#[unsafe(export_name = "oi_map_len")]
pub unsafe extern "C" fn map_len(map: *mut OiMap) -> i64 {
	unsafe { &*map }.entries.len() as i64
}

/// Look a key up and write its value to `out` if found.
/// # Safety
/// `map` must be a valid, live `OiMap` pointer and `out` a writable i64 slot.
#[unsafe(export_name = "oi_map_find")]
pub unsafe extern "C" fn map_find(map: *mut OiMap, tag: i64, bits: i64, out: *mut i64) -> i64 {
	let Some(v) = unsafe { &*map }.entries.get(&map_key(tag, bits)) else {
		return 0;
	};
	unsafe { *out = *v };
	1
}

/// The keys or values of a map as an array the caller releases.
/// # Safety
/// `map` must be a valid, live `OiMap` pointer.
#[unsafe(export_name = "oi_map_entries")]
pub unsafe extern "C" fn map_entries(map: *mut OiMap, keys: i64, width: i64) -> *const Header {
	let map = unsafe { &*map };
	let key_bits = |k: &MapKey| match k {
		MapKey::Raw(bits) => *bits,
		MapKey::Str(bytes) => str_new(system_allocator(), bytes) as i64,
	};
	let bits: Vec<i64> = match keys {
		0 => map.entries.values().copied().collect(),
		_ => map.entries.keys().map(key_bits).collect(),
	};
	array_of(&bits, width)
}
