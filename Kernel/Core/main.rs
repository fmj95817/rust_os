// "Tifflin" Kernel
// - By John Hodge (thePowersGang)
//
// Core/main.rs
// - Kernel main
#![crate_name="kernel"]
#![crate_type="lib"]
#![feature(no_std)]
#![feature(asm)]	// Enables the asm! syntax extension
#![feature(box_syntax)]	// Enables 'box' syntax
#![feature(thread_local)]	// Allows use of thread_local
#![feature(lang_items)]	// Allow definition of lang_items
#![feature(core)]	// silences warnings about write!
#![feature(optin_builtin_traits)]	// Negative impls
#![feature(unique)]	// Unique
#![feature(slice_patterns)]	// Slice (array) destructuring patterns, used by multiboot code
#![feature(step_by)]	// Range::step_by
#![feature(linkage)]	// allows using #[linkage="external"]
#![feature(const_fn)]	// Allows defining `const fn`
#![no_std]

#![feature(plugin)]
#![feature(custom_attribute)]
#![plugin(tag_safe)]
use prelude::*;

#[macro_use]
extern crate core;

extern crate stack_dst;

pub use arch::memory::PAGE_SIZE;

#[doc(hidden)]
#[macro_use] pub mod logmacros;
#[doc(hidden)]
#[macro_use] pub mod macros;
#[doc(hidden)]
#[macro_use] #[cfg(arch__amd64)] #[path="arch/amd64/mod-macros.rs"] pub mod arch_macros;

// Evil Hack: For some reason, write! (and friends) will expand pointing to std instead of core
#[doc(hidden)]
mod std {
	pub use core::option;
	pub use core::{default,fmt,cmp};
	pub use core::marker;	// needed for derive(Copy)
	pub use core::iter;	// needed for 'for'
}

/// Kernel's version of 'std::prelude'
pub mod prelude;

/// Library datatypes (Vec, Queue, ...)
#[macro_use]
pub mod lib;	// Clone of libstd

/// Heavy synchronisation primitives (Mutex, Semaphore, RWLock, ...)
#[macro_use]
pub mod sync;

/// Asynchrnous wait support
pub mod async;

/// Logging framework
pub mod logging;
/// Memory management (physical, virtual, heap)
pub mod memory;
/// Thread management
#[macro_use]
pub mod threads;
/// Timekeeping (timers and wall time)
pub mod time;

// Module/Executable loading (and symbol lookup)
pub mod loading;
/// Module management (loading and initialisation of kernel modules)
pub mod modules;

/// Meta devices (the Hardware Abstraction Layer)
pub mod metadevs;
/// Device to driver mapping manager
///
/// Starts driver instances for the devices it sees
pub mod device_manager;

/// User output, via a kernel-provided compositing "WM"
pub mod gui;

// Public for driver modules
pub mod vfs;

mod config;

/// Stack unwinding (panic) handling
pub mod unwind;

pub mod irqs;

pub mod syscalls;

/// Built-in device drivers
mod hw;

/// Achitecture-specific code - AMD64 (aka x86-64)
#[macro_use]
#[cfg(arch__amd64)] #[path="arch/amd64/mod.rs"] pub mod arch;	// Needs to be pub for exports to be avaliable

/// Kernel entrypoint
#[no_mangle]
pub extern "C" fn kmain()
{
	log_notice!("Tifflin Kernel v{} build {} starting", env!("TK_VERSION"), env!("TK_BUILD"));
	log_notice!("> Git state : {}", env!("TK_GITSPEC"));
	log_notice!("> Built with {}", env!("RUST_VERSION"));
	
	// Initialise core services before attempting modules
	::memory::phys::init();
	::memory::virt::init();
	::memory::heap::init();
	::threads::init();
	
	log_log!("Command line = '{}'", ::arch::boot::get_boot_string());
	::config::init( ::arch::boot::get_boot_string() );
	
	// Dump active video mode
	let vidmode = ::arch::boot::get_video_mode();
	match vidmode {
	Some(m) => {
		log_debug!("Video mode : {}x{} @ {:#x}", m.width, m.height, m.base);
		::metadevs::video::set_boot_mode(m);
		},
	None => log_debug!("No video mode present")
	}
	
	// Modules (dependency tree included)
	// - Requests that the GUI be started as soon as possible
	::modules::init(&["GUI"]);
	
	// Yield to allow init threads to run
	::threads::yield_time();
	
	// Run system init
	sysinit();
	
	// Thread 0 idle loop
	log_info!("Entering idle");
	loop
	{
		log_trace!("TID0 napping");
		::threads::yield_time();
	}
}

// Initialise the system once drivers are up
fn sysinit()
{
	use metadevs::storage::VolumeHandle;
	use vfs::{mount,handle};
	use vfs::Path;
	
	// 1. Mount /system to the specified volume
	let sysdisk = ::config::get_string(::config::Value::SysDisk);
	match VolumeHandle::open_named(sysdisk)
	{
	Err(e) => {
		log_error!("Unable to open /system volume {}: {}", sysdisk, e);
		return ;
		},
	Ok(vh) => match mount::mount("/system".as_ref(), vh, "", &[])
		{
		Ok(_) => {},
		Err(e) => {
			log_error!("Unable to mount /system from {}: {:?}", sysdisk, e);
			return ;
			},
		},
	}
	
	// 2. Symbolic link /sysroot to the specified folder
	let sysroot = ::config::get_string(::config::Value::SysRoot);
	handle::Dir::open(Path::new("/")).unwrap()
		.symlink("sysroot", Path::new(sysroot)).unwrap();
	
	
	// 3. Start 'init' (parent process)
	// XXX: hard-code the sysroot path here to avoid having to handle symlinks yet
	spawn_init("/system/Tifflin/bin/loader", "/system/Tifflin/bin/init");
	//spawn_init("/sysroot/bin/loader", "/sysroot/bin/init");

	fn ls(p: &Path) {
		// - Iterate root dir
		match handle::Dir::open(p)
		{
		Err(e) => log_warning!("'{:?}' cannot be opened: {:?}", p, e),
		Ok(h) =>
			for name in h.iter() {
				log_log!("{:?}", name);
			},
		}
	}

	// *. Testing: open a file known to exist on the testing disk	
	{
		match handle::File::open( Path::new("/system/1.TXT"), handle::FileOpenMode::SharedRO )
		{
		Err(e) => log_warning!("VFS test file can't be opened: {:?}", e),
		Ok(h) => {
			log_debug!("VFS open test = {:?}", h);
			let mut buf = [0; 16];
			let sz = h.read(0, &mut buf).unwrap();
			log_debug!("- Contents: {:?}", ::lib::RawString(&buf[..sz]));
			},
		}
		
		ls(Path::new("/"));
		ls(Path::new("/system"));
	}
	
	// *. TEST Automount
	// - Probably shouldn't be included in the final version, but works for testing filesystem and storage drivers
	let mountdir = handle::Dir::open( Path::new("/") ).and_then(|h| h.mkdir("mount")).unwrap();
	for (_,v) in ::metadevs::storage::enum_lvs()
	{
		let vh = match VolumeHandle::open_named(&v)
			{
			Err(e) => {
				log_log!("Unable to open '{}': {}", v, e);
				continue;
				},
			Ok(v) => v,
			};
		mountdir.mkdir(&v).unwrap();
		let mountpt = format!("/mount/{}",v);
		match mount::mount( mountpt.as_ref(), vh, "", &[] )
		{
		Ok(_) => log_log!("Auto-mounted to {}", mountpt),
		Err(e) => log_notice!("Unable to automount '{}': {:?}", v, e),
		}
	}
	ls(Path::new("/mount/ATA-2w"));
}

fn spawn_init(loader_path: &str, init_cmdline: &str)
{
	use vfs::handle;
	use vfs::Path;
	use core::mem::forget;
	
	#[repr(C)]
	struct LoaderHeader
	{
		magic: u32,
		info: u32,
		codesize: u32,
		memsize: u32,
		init_path_ofs: u32,
		init_path_len: u32,
		entrypoint: usize,
	}
	const MAGIC: u32 = 0x71FF1013;
	#[cfg(arch__amd64)]
	const INFO: u32 = (3*4+2*8) | (2 << 8);
	#[cfg(arch__amd64)]
	const LOAD_MAX: usize = 1 << 47;
	
	log_log!("Loading userland '{}' args '{}'", loader_path, init_cmdline);
	
	// - 1. Memory-map the loader binary to a per-architecture location
	//  > E.g. for x86 it'd be 0xBFFF0000 - Limiting it to 64KiB
	//  > For amd64: 1<<48-64KB
	//  > PANIC if the binary (or its memory size) is too large
	let loader = match handle::File::open(Path::new(loader_path), handle::FileOpenMode::Execute)
		{
		Ok(v) => v,
		Err(e) => {
			log_error!("Unable to open initial userland loader '{}': {:?}", loader_path, e);
			return ;
			},
		};
	let max_size: usize = 2*64*1024;
	let load_base: usize = LOAD_MAX - max_size;
	let ondisk_size = loader.size();
	let mh_firstpage = {
		if ondisk_size > max_size as u64 {
			log_error!("Loader is too large to fit in reserved region ({}, max {})",
				ondisk_size, max_size);
			return ;
		}
		loader.memory_map(load_base,  0, ::PAGE_SIZE,  handle::MemoryMapMode::Execute)
		};
	// - 2. Parse the header
	let header_ptr = unsafe { &*(load_base as *const LoaderHeader) };
	if header_ptr.magic != 0x71FF1013 || header_ptr.info != INFO {
		log_error!("Loader header is invalid: magic {:#x} != {:#x} or info {:#x} != {:#x}",
			header_ptr.magic, MAGIC, header_ptr.info, INFO);
		return ;
	}
	// - 3. Map the remainder of the image into memory (with correct permissions)
	let codesize = header_ptr.codesize as usize;
	let memsize = header_ptr.memsize as usize;
	let datasize = ondisk_size as usize - codesize;
	let bss_size = memsize - ondisk_size as usize;
	log_debug!("Executable size: {}, rw data size: {}", codesize, datasize);
	let mh_code = loader.memory_map(load_base + ::PAGE_SIZE, ::PAGE_SIZE as u64, codesize - ::PAGE_SIZE,  handle::MemoryMapMode::Execute);
	assert!(codesize % ::PAGE_SIZE == 0, "Loader code doesn't end on a page boundary - {:#x}", codesize);
	let mh_data = loader.memory_map(load_base + codesize, codesize as u64, datasize,  handle::MemoryMapMode::COW);
	
	// - 4. Allocate the loaders's BSS
	assert!(ondisk_size as usize % ::PAGE_SIZE == 0, "Loader file size is not aligned to a page - {:#x}", ondisk_size);
	let pages = (bss_size + ::PAGE_SIZE) / ::PAGE_SIZE;
	let bss_start = (load_base + ondisk_size as usize) as *mut ();
	let ah_bss = ::memory::virt::allocate_user(bss_start, pages);
	
	// - 5. Write loader arguments
	if (header_ptr.init_path_ofs as usize) < codesize || (header_ptr.init_path_ofs as usize + header_ptr.init_path_len as usize) >= memsize {
		log_error!("Userland init string location out of range: {:#x}", header_ptr.init_path_ofs);
		return ;
	}
	// TODO: Write loader arguments into the provided location
	// TODO: Should the argument string length be passed down to the user? In memory, or via a register?
	let argslen = unsafe {
		::core::slice::from_raw_parts_mut(
				(load_base + header_ptr.init_path_ofs as usize) as *mut u8,
				header_ptr.init_path_len as usize
				)
			.clone_from_slice(init_cmdline.as_bytes())
		};
	
	// - 6. Enter userland
	if header_ptr.entrypoint < load_base || header_ptr.entrypoint >= load_base + LOAD_MAX {
		log_error!("Userland entrypoint out of range: {:#x}", header_ptr.entrypoint);
		return ;
	}
	
	// > Forget about all maps and allocations
	//  ... Is this strictly nessesary? This function never resumes after drop_to_user.
	//  May be needed if the loader gets moved
	forget(mh_firstpage);
	forget(mh_code);
	forget(mh_data);
	forget(ah_bss);
	// > Forget the loader handle too
	// TODO: Instead hand this handle over to the syscall layer, as the first user file
	//forget(loader);
	// SAFE: This pointer is as validated as it can be...
	log_notice!("Entering userland at {:#x} '{}' '{}'", header_ptr.entrypoint, loader_path, init_cmdline);
	unsafe {
		::arch::drop_to_user(header_ptr.entrypoint, argslen);
	}
}

// vim: ft=rust

