#![feature(f16)]
//! Backend-agnostic functions a compiled Oi program calls at runtime.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::sync::atomic::{AtomicI64, Ordering};

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

impl Tag {
	// Checked conversion from the raw i64 the JIT passes across the ABI.
	fn from_i64(v: i64) -> Tag {
		match v {
			0 => Tag::Bool,
			1 => Tag::Int,
			2 => Tag::UInt,
			3 => Tag::Float,
			4 => Tag::Str,
			5 => Tag::Raw,
			_ => {
				eprintln!("invalid tag: {v}");
				die();
			}
		}
	}
}

// Flush stdout, then fail without a core dump.
fn die() -> ! {
	let _ = std::io::Write::flush(&mut std::io::stdout());
	std::process::exit(101);
}

// Output sink for writing.
#[repr(i64)]
#[derive(Clone, Copy)]
pub enum Sink {
	Out,
	Err,
	Buf,
}

impl Sink {
	fn from_i64(v: i64) -> Sink {
		match v {
			0 => Sink::Out,
			1 => Sink::Err,
			2 => Sink::Buf,
			_ => {
				eprintln!("invalid sink: {v}");
				die();
			}
		}
	}
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
	if data == 0 {
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
pub fn str_new(bytes: &[u8]) -> *const StrHeader {
	let mut buf = bytes.to_vec();
	buf.push(0);
	let data = Box::leak(buf.into_boxed_slice()).as_ptr() as i64;
	Box::leak(Box::new(StrHeader {
		data,
		len: bytes.len() as i64,
	})) as *const StrHeader
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

// Panic with an out-of-bounds message.
#[unsafe(export_name = "oi_panic_oob")]
pub extern "C" fn panic_oob(index: i64, len: i64) {
	eprintln!("index out of range: the length is {len} but the index is {index}");
	die();
}

// Wrap integer exponents.
#[unsafe(export_name = "oi_pow_int")]
pub extern "C" fn pow_int(base: i64, exp: i64) -> i64 {
	if exp < 0 {
		eprintln!("negative exponent: {exp}");
		die();
	}
	base.wrapping_pow(exp as u32)
}

#[unsafe(export_name = "oi_pow_float")]
pub extern "C" fn pow_float(base: f64, exp: f64) -> f64 {
	base.powf(exp)
}

// Print `{prefix}{msg}` and abort.
unsafe fn abort_with(prefix: &str, msg: *const StrHeader) -> ! {
	let msg = unsafe { str_lossy(msg) };
	eprintln!("{prefix}{msg}");
	die();
}

/// Print an assertion failure message and abort.
/// # Safety
/// `msg` must be a valid string handle.
#[unsafe(export_name = "oi_assert_fail")]
pub unsafe extern "C" fn assert_fail(msg: *const StrHeader) {
	unsafe { abort_with("assertion failed: ", msg) }
}

/// Print a panic message and abort.
/// # Safety
/// `msg` must be a valid string handle.
#[unsafe(export_name = "oi_panic")]
pub unsafe extern "C" fn panic(msg: *const StrHeader) {
	unsafe { abort_with("panic: ", msg) }
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
pub unsafe extern "C" fn str_from_bytes(header: *const Header) -> *const StrHeader {
	let Header { data, len, .. } = unsafe { *header };
	if data == 0 {
		return str_new(&[]);
	}
	str_new(unsafe { std::slice::from_raw_parts(data as *const u8, len as usize) })
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
pub unsafe extern "C" fn str_concat(a: *const StrHeader, b: *const StrHeader) -> *const StrHeader {
	let a = unsafe { str_bytes(a) };
	let b = unsafe { str_bytes(b) };
	let mut out = Vec::with_capacity(a.len() + b.len());
	out.extend_from_slice(a);
	out.extend_from_slice(b);
	str_new(&out)
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
pub unsafe extern "C" fn cstr_str(ptr: i64) -> *const StrHeader {
	if ptr == 0 {
		return str_new(&[]);
	}
	let bytes = unsafe { std::ffi::CStr::from_ptr(ptr as *const std::ffi::c_char) }.to_bytes();
	str_new(bytes)
}

/// Build a string handle by copying `len` bytes from `data`.
/// # Safety
/// `data` must be null or point to at least `len` readable bytes.
#[unsafe(export_name = "oi_ptr_string")]
pub unsafe extern "C" fn ptr_string(data: i64, len: i64) -> *const StrHeader {
	if data == 0 || len <= 0 {
		return str_new(&[]);
	}
	str_new(unsafe { std::slice::from_raw_parts(data as *const u8, len as usize) })
}

/// Copy bytes from `data` into a fresh rc'd array buffer.
/// # Safety
/// `data` must be null or point to at least `bytes` readable bytes.
#[unsafe(export_name = "oi_ptr_buffer")]
pub unsafe extern "C" fn ptr_buffer(data: i64, bytes: i64) -> i64 {
	let bytes = if data == 0 { 0 } else { bytes.max(0) };
	let buf = buffer_alloc(bytes);
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
pub extern "C" fn str_take(mark: i64) -> *const StrHeader {
	BUF.with(|b| str_new(b.borrow_mut().split_off(mark as usize).as_bytes()))
}

// Active managed allocations, for leak checks.
static LIVE: AtomicI64 = AtomicI64::new(0);

pub fn leaked() -> i64 {
	LIVE.load(Ordering::Relaxed)
}

// Allocate `size` zeroed bytes for a composite value (e.g. a tuple's field slots).
#[unsafe(export_name = "oi_alloc")]
pub extern "C" fn alloc(size: i64) -> *mut u8 {
	let size = size.max(1) as usize + 8;
	let layout = std::alloc::Layout::from_size_align(size, 8).unwrap();
	LIVE.fetch_add(1, Ordering::Relaxed);
	unsafe {
		let base = std::alloc::alloc_zeroed(layout);
		*(base as *mut i64) = size as i64;
		base.add(8)
	}
}

// Free an `alloc` result.
unsafe fn free(ptr: *mut u8) {
	if ptr.is_null() {
		return;
	}
	LIVE.fetch_sub(1, Ordering::Relaxed);
	unsafe {
		let base = ptr.sub(8);
		let size = *(base as *const i64) as usize;
		std::alloc::dealloc(base, std::alloc::Layout::from_size_align_unchecked(size, 8));
	}
}

// Allocate an element buffer with its refcount at data[-8], count starting at 1.
fn buffer_alloc(bytes: i64) -> *mut u8 {
	let base = alloc(bytes + 8);
	unsafe {
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

/// Clone a header, sharing its buffer with a refcount bump.
/// # Safety
/// `header` must point to a valid array header.
#[unsafe(export_name = "oi_array_share")]
pub unsafe extern "C" fn array_share(header: *const Header) -> *const Header {
	let h = unsafe { *header };
	if h.data != 0 {
		unsafe { *((h.data - 8) as *mut i64) += 1 };
	}
	let out = alloc(size_of::<Header>() as i64) as *mut Header;
	unsafe { *out = h };
	out
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
	if data != 0 {
		let rc = (data - 8) as *mut i64;
		unsafe { *rc -= 1 };
		if unsafe { *rc } == 0 {
			unsafe { free((data - 8) as *mut u8) };
		}
	}
	unsafe { free(header as *mut u8) };
}

/// Give a shared array its own buffer before a write.
/// No-op when the buffer is null or unshared.
/// # Safety
/// `header` must point to a valid array header.
#[unsafe(export_name = "oi_array_cow")]
pub unsafe extern "C" fn array_cow(header: *mut Header, elem_size: i64) {
	let Header { data, len, .. } = unsafe { *header };
	if data == 0 || unsafe { *((data - 8) as *const i64) } <= 1 {
		return;
	}
	let new_data = buffer_alloc(len * elem_size);
	unsafe {
		std::ptr::copy_nonoverlapping(data as *const u8, new_data, (len * elem_size) as usize);
		*((data - 8) as *mut i64) -= 1;
		(*header).data = new_data as i64;
		(*header).cap = len;
	}
}

/// A fresh array owning `elems`, each packed into `width` bytes.
pub fn array_of(elems: &[i64], width: i64) -> *const Header {
	let len = elems.len() as i64;
	let data = buffer_alloc(len * width);
	for (i, v) in elems.iter().enumerate() {
		unsafe {
			std::ptr::copy_nonoverlapping(v.to_le_bytes().as_ptr(), data.add(i * width as usize), width as usize)
		};
	}
	let out = alloc(size_of::<Header>() as i64) as *mut Header;
	unsafe {
		*out = Header {
			data: data as i64,
			len,
			cap: len,
		}
	};
	out
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
pub unsafe extern "C" fn slice(header: *const Header, start: i64, end: i64, elem_size: i64) -> *const Header {
	let Header { data, len, .. } = unsafe { *header };
	if start < 0 || start > end || end > len {
		eprintln!("slice range {start}..{end} out of bounds for array of length {len}");
		die();
	}
	let view_len = end - start;
	let new_data = buffer_alloc(view_len * elem_size);
	let out = alloc(size_of::<Header>() as i64) as *mut Header;
	unsafe {
		let src = (data + start * elem_size) as *const u8;
		std::ptr::copy_nonoverlapping(src, new_data, (view_len * elem_size) as usize);
		*out = Header {
			data: new_data as i64,
			len: view_len,
			cap: view_len,
		};
	}
	out
}

/// View a range of a string through a fresh handle sharing the same buffer.
/// # Safety
/// `header` must point to a valid string header.
#[unsafe(export_name = "oi_str_slice")]
pub unsafe extern "C" fn str_slice(header: *const StrHeader, start: i64, end: i64) -> *const StrHeader {
	let StrHeader { data, len } = unsafe { *header };
	if start < 0 || start > end || end > len {
		eprintln!("slice range {start}..{end} out of bounds for string of length {len}");
		die();
	}
	let out = alloc(size_of::<StrHeader>() as i64) as *mut StrHeader;
	unsafe {
		*out = StrHeader {
			data: data + start,
			len: end - start,
		}
	};
	out
}

/// Write a `mut` slice projection back into its parent buffer at `lo`.
/// # Safety
/// `parent` and `src` must point to valid array headers.
#[unsafe(export_name = "oi_array_write_back")]
pub unsafe extern "C" fn array_write_back(parent: *mut Header, lo: i64, len: i64, src: *const Header, elem_size: i64) {
	let Header { data, len: slen, .. } = unsafe { *src };
	if slen != len {
		eprintln!("projection changed length: expected {len} elements, got {slen}");
		die();
	}
	unsafe {
		let dst = ((*parent).data + lo * elem_size) as *mut u8;
		std::ptr::copy_nonoverlapping(data as *const u8, dst, (len * elem_size) as usize);
	}
}

/// Ensure the array has capacity for at least `min_cap` elements.
/// Grows by doubling, at least to `min_cap`. Updates data and cap in place.
/// # Safety
/// `header` must point to a valid array header.
#[unsafe(export_name = "oi_array_reserve")]
pub unsafe extern "C" fn array_reserve(header: *mut Header, min_cap: i64, elem_size: i64) {
	let Header { data, len, cap } = unsafe { *header };
	if min_cap <= cap {
		return;
	}
	let new_cap = (cap.max(1) * 2).max(min_cap);
	let new_data = buffer_alloc(new_cap * elem_size);
	unsafe {
		std::ptr::copy_nonoverlapping(data as *const u8, new_data, (len * elem_size) as usize);
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
pub unsafe extern "C" fn array_extend(dst: *mut Header, src: *const Header, elem_size: i64) {
	let dst_len = unsafe { (*dst).len };
	let Header {
		data: src_data,
		len: src_len,
		..
	} = unsafe { *src };
	unsafe { array_reserve(dst, dst_len + src_len, elem_size) };
	unsafe {
		let dst_data = (*dst).data as *mut u8;
		let dst_tail = dst_data.add((dst_len * elem_size) as usize);
		std::ptr::copy_nonoverlapping(src_data as *const u8, dst_tail, (src_len * elem_size) as usize);
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
	unsafe { *(ptr.sub(8) as *mut i64) += 1 };
	ptr
}

// Walk a box's trace descriptor, calling `visit` on each live ref slot.
unsafe fn trace(fields: *mut u8, desc: *const i64, visit: &mut dyn FnMut(*mut u8)) {
	if desc.is_null() {
		return;
	}
	unsafe {
		let mut p = desc.add(1);
		for _ in 0..*desc {
			let e = *p;
			p = p.add(1);
			let slot = *(fields.add((e >> 1) as usize) as *const *mut u8);
			if slot.is_null() {
				p = p.add((e & 1) as usize);
				continue;
			}
			if e & 1 == 0 {
				visit(slot);
			} else {
				trace(slot, *p as *const i64, &mut *visit);
				p = p.add(1);
			}
		}
	}
}

// Boxes whose non-zero release marked them as possible cycle roots.
thread_local! {
	static ROOTS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
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
		let rc = ptr.sub(8) as *mut i64;
		*rc -= 1;
		if *rc == 0 {
			// remove freed boxes to avoid `collect_cycles` walking freed memory
			ROOTS.with(|r| r.borrow_mut().remove(&(ptr as usize)));
			trace(ptr, desc(ptr), &mut |c| ref_release(c));
			free(ptr.sub(16));
		} else if !desc(ptr).is_null() {
			// still alive and holding refs
			// NOTE: possible cycle root
			ROOTS.with(|r| r.borrow_mut().insert(ptr as usize));
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
pub fn collect_cycles() {
	let roots: Vec<usize> = ROOTS.with(|r| r.borrow_mut().drain().collect());
	let mut c = HashMap::new();
	for &s in &roots {
		mark_gray(s as *mut u8, &mut c);
	}
	for &s in &roots {
		scan(s as *mut u8, &mut c);
	}
	for &s in &roots {
		collect_white(s as *mut u8, &mut c);
	}
}

// Attempt to decrement children, painting the candidate subgraph gray.
fn mark_gray(s: *mut u8, c: &mut HashMap<usize, Color>) {
	if c.insert(s as usize, Color::Gray) == Some(Color::Gray) {
		return;
	}
	unsafe {
		trace(s, desc(s), &mut |t| {
			*(t.sub(8) as *mut i64) -= 1;
			mark_gray(t, c);
		})
	};
}

// Scan and repaint nodes based on refcounts.
fn scan(s: *mut u8, c: &mut HashMap<usize, Color>) {
	if c.get(&(s as usize)) != Some(&Color::Gray) {
		return;
	}
	if unsafe { *(s.sub(8) as *const i64) } > 0 {
		scan_black(s, c);
	} else {
		c.insert(s as usize, Color::White);
		unsafe { trace(s, desc(s), &mut |t| scan(t, c)) };
	}
}

fn scan_black(s: *mut u8, c: &mut HashMap<usize, Color>) {
	c.insert(s as usize, Color::Black);
	unsafe {
		trace(s, desc(s), &mut |t| {
			*(t.sub(8) as *mut i64) += 1;
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
		trace(s, desc(s), &mut |t| collect_white(t, c));
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

#[derive(Clone, PartialEq, Eq, Hash)]
enum MapKey {
	Raw(i64),
	Str(Vec<u8>),
}

fn map_key(tag: Tag, bits: i64) -> MapKey {
	match tag {
		Tag::Str => MapKey::Str(unsafe { str_bytes(bits as *const StrHeader) }.to_vec()),
		_ => MapKey::Raw(bits),
	}
}

pub struct OiMap {
	entries: HashMap<MapKey, i64>,
	rc: i64,
}

#[unsafe(export_name = "oi_map_new")]
pub extern "C" fn map_new() -> *mut OiMap {
	LIVE.fetch_add(1, Ordering::Relaxed);
	Box::into_raw(Box::new(OiMap {
		entries: HashMap::new(),
		rc: 1,
	}))
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
		LIVE.fetch_sub(1, Ordering::Relaxed);
		drop(unsafe { Box::from_raw(map) });
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
unsafe fn map_cow(map: *mut OiMap) -> *mut OiMap {
	if unsafe { (*map).rc } <= 1 {
		return map;
	}
	unsafe { (*map).rc -= 1 };
	let entries = unsafe { (*map).entries.clone() };
	LIVE.fetch_add(1, Ordering::Relaxed);
	Box::into_raw(Box::new(OiMap { entries, rc: 1 }))
}

/// # Safety
/// `map` must be a valid, live `OiMap` pointer.
#[unsafe(export_name = "oi_map_get")]
pub unsafe extern "C" fn map_get(map: *mut OiMap, tag: i64, bits: i64) -> i64 {
	let map = unsafe { &*map };
	match map.entries.get(&map_key(Tag::from_i64(tag), bits)) {
		Some(v) => *v,
		None => {
			eprintln!("key not found in map");
			die();
		}
	}
}

/// Set a map entry, cloning shared entries first.
/// # Safety
/// `map` must be a valid, live `OiMap` pointer.
#[unsafe(export_name = "oi_map_set")]
pub unsafe extern "C" fn map_set(map: *mut OiMap, tag: i64, bits: i64, value: i64) -> *mut OiMap {
	let map = unsafe { map_cow(map) };
	unsafe { &mut *map }.entries.insert(map_key(Tag::from_i64(tag), bits), value);
	map
}

/// Remove a map entry if present, cloning shared entries first.
/// # Safety
/// `map` must be a valid, live `OiMap` pointer.
#[unsafe(export_name = "oi_map_delete")]
pub unsafe extern "C" fn map_delete(map: *mut OiMap, tag: i64, bits: i64) -> *mut OiMap {
	let map = unsafe { map_cow(map) };
	unsafe { &mut *map }.entries.remove(&map_key(Tag::from_i64(tag), bits));
	map
}

/// The number of entries in a map.
/// # Safety
/// `map` must be a valid, live `OiMap` pointer.
#[unsafe(export_name = "oi_map_len")]
pub unsafe extern "C" fn map_len(map: *mut OiMap) -> i64 {
	unsafe { &*map }.entries.len() as i64
}

/// The keys or values of a map as an array the caller releases.
/// # Safety
/// `map` must be a valid, live `OiMap` pointer.
#[unsafe(export_name = "oi_map_entries")]
pub unsafe extern "C" fn map_entries(map: *mut OiMap, keys: i64, width: i64) -> *const Header {
	let map = unsafe { &*map };
	let key_bits = |k: &MapKey| match k {
		MapKey::Raw(bits) => *bits,
		MapKey::Str(bytes) => str_new(bytes) as i64,
	};
	let bits: Vec<i64> = match keys {
		0 => map.entries.values().copied().collect(),
		_ => map.entries.keys().map(key_bits).collect(),
	};
	array_of(&bits, width)
}
