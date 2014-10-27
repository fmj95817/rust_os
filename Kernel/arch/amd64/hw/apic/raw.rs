// "Tifflin" Kernel
// - By John Hodge (thePowersGang)
//
// arch/amd64/hw/apic/raw.rs
// - x86 APIC Raw hardware API
use _common::*;
use core::ptr::RawPtr;

static TIMER_VEC: u8 = 0x7E;

pub struct LAPIC
{
	paddr: u64,
	mapping: ::memory::virt::AllocHandle,
	timer_isr: ::arch::interrupts::ISRHandle,
}

pub struct IOAPIC
{
	regs: ::sync::Mutex<IOAPICRegs>,
	num_lines: uint,
	first_irq: uint,
	handlers: Vec<Option<super::IRQHandler>>,
}

struct IOAPICRegs
{
	mapping: ::memory::virt::AllocHandle,
}

#[allow(dead_code)]
#[deriving(Show)]
pub enum TriggerMode
{
	TriggerLevelHi,
	TriggerLevelLow,
	TriggerEdgeHi,
	TriggerEdgeLow,
}

#[allow(dead_code)]
#[repr(C)]
enum ApicRegisters
{
	ApicReg_LAPIC_ID  = 0x2,
	ApicReg_LAPIC_Ver = 0x3,
	ApicReg_TPR       = 0x8,	// Task Priority
	ApicReg_APR       = 0x9,	// Arbitration Priority
	ApicReg_PPR       = 0xA,	// Processor Priority
	ApicReg_EOI       = 0xB,
	ApicReg_RRD       = 0xC,	// Remote Read
	ApicReg_LocalDest = 0xD,	// Local Destination
	ApicReg_DestFmt   = 0xE,	// Destination Format
	ApicReg_SIR       = 0xF,	// Spurious Interrupt Vector
	ApicReg_InService = 0x10,	// In-Service Register (First of 8)
	ApicReg_TMR       = 0x18,	// Trigger Mode (1/8)
	ApicReg_IRR       = 0x20,	// Interrupt Request Register (1/8)
	ApicReg_ErrStatus = 0x28,	// Error Status
	ApicReg_LVTCMCI   = 0x2F,	// LVT CMCI Registers (?)
	ApicReg_ICR       = 0x30,	// Interrupt Command Register (1/2)
	ApicReg_LVTTimer  = 0x32,
	ApicReg_LVTThermalSensor = 0x33,
	ApicReg_LVTPermCounters  = 0x34,
	ApicReg_LVT_LINT0 = 0x35,
	ApicReg_LVT_LINT1 = 0x36,
	ApicReg_LVT_Error = 0x37,
	ApicReg_InitCount = 0x38,
	ApicReg_CurCount  = 0x39,
	ApicReg_TmrDivide = 0x3E,
}

#[repr(C,packed)]
struct APICReg
{
	data: u32,
	_rsvd: [u32,..3],
}

impl LAPIC
{
	pub fn new(paddr: u64) -> LAPIC
	{
		let ret = LAPIC {
			paddr: paddr,
			mapping: ::memory::virt::map_hw_rw(paddr, 1, "APIC").unwrap(),
			timer_isr: Default::default(),
			};
		
		log_debug!("LAPIC {{ IDReg={:x}, Ver={:x}, SIR={:#x} }}",
			ret.read_reg(ApicReg_LAPIC_ID as uint),
			ret.read_reg(ApicReg_LAPIC_Ver as uint),
			ret.read_reg(ApicReg_SIR as uint)
			);
		
		ret
	}
	/// Initialise the LAPIC structures once self is in its final location
	pub fn global_init(&mut self)
	{
		self.timer_isr = match ::arch::interrupts::bind_isr(
			//TIMER_VEC, LAPIC::local_timer, self as *mut _ as *const (), 0
			TIMER_VEC, lapic_timer, self as *mut _ as *const (), 0
			)
			{
			Ok(v) => v,
			Err(e) => fail!("Unable to bind LAPIC timer: {}", e),
			};
	}
	/// Initialise the LAPIC (for this CPU)
	pub fn init(&self)
	{
		let oldaddr = unsafe{
			let mut a: u64;
			let mut d: u64;
			asm!("rdmsr" : "={eax}" (a), "={edx}" (d) : "{rcx}" (0x1Bu) : "rdx");
			d << 32 | a
			};
		log_debug!("oldaddr = {:#x}", oldaddr);
		let is_bsp = oldaddr & 0x100;
		for i in range(0, 8) {
			log_debug!("IRR{} = {:#x}", i, self.read_reg(ApicReg_IRR as uint + i));
		}
		
		//self.write_reg(ApicReg_SIR as uint, self.read_reg(ApicReg_SIR as uint) | (1 << 8));
		self.write_reg(ApicReg_SIR as uint, 0x7F | (1 << 8));	// Enable LAPIC (and set Spurious to 127)
		self.write_reg(ApicReg_InitCount as uint, 0x100000);
		self.write_reg(ApicReg_TmrDivide as uint, 3);	// Timer Divide = 16
		self.write_reg(ApicReg_LVTTimer as uint, TIMER_VEC as u32);	// Enable Timer
		self.write_reg(ApicReg_LVTThermalSensor as uint, 0);	// "Disable" Thermal Sensor
		self.write_reg(ApicReg_LVTPermCounters as uint, 0);	// "Disable" ? Counters
		self.write_reg(ApicReg_LVT_LINT0 as uint, 0);	// "Disable" LINT0
		self.write_reg(ApicReg_LVT_LINT1 as uint, 0);	// "Disable" LINT1
		self.write_reg(ApicReg_LVT_Error as uint, 0);	// "Disable" Error
		// EOI - Just to make sure
		self.eoi(0);
		unsafe {
		asm!("wrmsr\nsti"
			: /* no out */
			: "{ecx}" (0x1Bu), "{edx}" (self.paddr >> 32), "{eax}" (self.paddr | is_bsp | 0x800)
			: /* no clobbers */
			: "volatile"
			);
		}
	
		unsafe {
			let mut ef: u64;
			asm!("pushf\npop $0" : "=r" (ef));
			log_debug!("EFLAGS = {:#x}", ef);
		}
	}
	pub fn eoi(&self, num: uint)
	{
		self.write_reg(ApicReg_EOI as uint, num as u32);
	}
	
	fn read_reg(&self, idx: uint) -> u32
	{
		//let regs = self.mapping.as_ref::<[APICReg,..2]>(0);
		//regs[0].data = idx as u32;
		//regs[1].data
		let regs = self.mapping.as_ref::<[APICReg,..64]>(0);
		unsafe { ::core::intrinsics::volatile_load( &regs[idx].data as *const _ ) }
	}
	fn write_reg(&self, idx: uint, value: u32)
	{
		let regs = self.mapping.as_ref::<[APICReg,..64]>(0);
		unsafe { ::core::intrinsics::volatile_store( &mut regs[idx].data as *mut _, value ) }
	}
	
	pub fn get_vec_status(&self, idx: uint) -> (bool,bool,bool, u32)
	{
		let reg = idx / 32;
		let bit = idx % 32;
		let mask = 1 << bit;
		let in_svc = self.read_reg(ApicReg_InService as uint + reg) & mask != 0;
		let mode   = self.read_reg(ApicReg_TMR as uint + reg) & mask != 0;
		let in_req = self.read_reg(ApicReg_IRR as uint + reg) & mask != 0;
		let err = self.read_reg(ApicReg_ErrStatus as uint);
		
		(in_svc, mode, in_req, err)
	}
	
	fn local_timer(isr: uint, sp: *const (), _idx: uint)
	{
		assert!( !sp.is_null() );
		let s: &LAPIC = unsafe { &*(sp as *const LAPIC) };
		log_trace!("LAPIC Timer");
		s.eoi(isr);
	}
}
extern "C" fn lapic_timer(isr: uint, sp: *const (), _idx: uint)
{
	LAPIC::local_timer(isr, sp, _idx);	
}

impl IOAPIC
{
	pub fn new(paddr: u64, base: uint) -> IOAPIC
	{
		let regs = IOAPICRegs::new(paddr);
		let v = regs.read(1);
		log_debug!("{:x} {:x} {:x}", v, v>>16, (v >> 16) & 0xFF);
		let num_lines = ((v >> 16) & 0xFF) as uint + 1;
		log_debug!("regs=[{:#x},{:#x},{:#x}]", regs.read(0), regs.read(1), regs.read(2));
		
		log_debug!("IOAPIC: {{ {:#x} - {} + {} }}", paddr, base, num_lines);
		IOAPIC {
			regs: mutex_init!( regs ),
			num_lines: num_lines,
			first_irq: base,
			handlers: Vec::from_fn(num_lines, |_|None),
			}
	}
	
	pub fn contains(&self, gsi: uint) -> bool {
		self.first_irq <= gsi && gsi < self.first_irq + self.num_lines
	}
	pub fn first(&self) -> uint {
		self.first_irq
	}
	pub fn get_callback(&self, idx: uint) -> super::IRQHandler {
		assert!( idx < self.num_lines );
		self.handlers[idx].unwrap()
	}
	
	pub fn eoi(&mut self, _idx: uint)
	{
		// TODO: EOI in IOAPIC
	}
	pub fn set_irq(&mut self, idx: uint, vector: u8, apic: uint, mode: TriggerMode, cb: super::IRQHandler)
	{
		let rh = self.regs.lock();
		log_trace!("set_irq(idx={},vector={},apic={},mode={})", idx, vector, apic, mode);
		log_debug!("Info = {:#x}", (*rh).read(0x10 + idx*2));
		assert!( idx < self.num_lines );

		*self.handlers.get_mut(idx) = Some( cb );
		let flags: u32 = match mode {
			TriggerEdgeHi   => (0<<13)|(0<<15),
			TriggerEdgeLow  => (1<<13)|(0<<15),
			TriggerLevelHi  => (0<<13)|(1<<15),
			TriggerLevelLow => (1<<13)|(1<<15),
			};
		(*rh).write(0x10 + idx*2 + 1, (apic as u32 << 56-32) );
		(*rh).write(0x10 + idx*2 + 0, flags | (vector as u32) );
	}
	pub fn disable_irq(&mut self, idx: uint)
	{
		let rh = self.regs.lock();
		log_debug!("Disable {}: Info = {:#x}", idx, (*rh).read(0x10 + idx*2));
		(*rh).write(0x10 + idx*2 + 0, 1<<16);
	}

	pub fn get_irq_reg(&mut self, idx: uint) -> u64
	{
		let rh = self.regs.lock();
		
		((*rh).read(0x10 + idx*2 + 0) as u64) | ((*rh).read(0x10 + idx*2 + 1) as u64 << 32)
	}
}

impl IOAPICRegs
{
	fn new( paddr: u64 ) -> IOAPICRegs
	{
		let mapping = ::memory::virt::map_hw_rw(paddr, 1, "IOAPIC").unwrap();
		IOAPICRegs {
			mapping: mapping
		}
	}
	fn read(&self, idx: uint) -> u32
	{
		let regs = self.mapping.as_ref::<[APICReg,..2]>(0);
		unsafe {
		::core::intrinsics::volatile_store(&mut regs[0].data as *mut _, idx as u32);
		::core::intrinsics::volatile_load(&regs[1].data as *const _)
		}
	}
	fn write(&self, idx: uint, data: u32)
	{
		let regs = self.mapping.as_ref::<[APICReg,..2]>(0);
		unsafe {
		::core::intrinsics::volatile_store(&mut regs[0].data as *mut _, idx as u32);
		::core::intrinsics::volatile_store(&mut regs[1].data as *mut _, data)
		}
	}
	
}

// vim: ft=rust