
// "platform-$(PLATFORM).h" is inserted by cpp

#define STACK_TOP	(RAM_START+RAM_LENGTH)

#define ENTRY(n)	.globl n ; n:

.extern _binary_______bin_kernel_armv7_bin_start

.section .text
	b .
	b .
	b .
	b .
	b .
ENTRY(start)
	mov sp, #STACK_TOP
	mov r0, #UART_BASE
	push {r0}
	
	ldr r8, =_binary_______bin_kernel_armv7_bin_start

	@ Get the size of the kernel image
	mov r0, r8
	bl elf_get_size
	mov r12, r0
	
	add r11, r0, #RAM_START
	
#if defined(FDT_BASE)
	mov r10, #0	@ Pre-set FDT base register to 0 (invalid)
	ldr r0, =FDT_BASE
	ldr r1, [r0]
	ldr r2, =0xedfe0dd0
	teq r1, r2
	bne 1f
	ldr r1, [r0, #4]
	rev r1, r1
	@ R1 = Size of FDT
	
	mov r10, r11	@ Save FDT base
	add r11, r11, r1	@ Increment free RAM base by size of FDT
	
	mov r2, r1	@ Count
	mov r1, r0	@ Source
	mov r0, r10	@ Destination
	bl __aeabi_memcpy4
1:
#else
.extern fdt_start
.extern fdt_len
	ldr r2, =fdt_end
	ldr r1, =fdt_start
	sub r2, r2, r1
	mov r10, r11	@ Save FDT base
	add r11, r11, r2	@ Increment free RAM base by size of FDT
	
	mov r0, r10	@ Destination
	bl __aeabi_memcpy4
#endif
	
	@ Load image
	mov r0, r8
	mov r1, #RAM_START
	bl elf_load_segments
	mov r9, r0	@ Save entrypoint
	
	@ Load symbols
	mov r0, r8
	mov r1, r11
	bl elf_load_symbols
	add r12, r11, r0
	
	mov r8, r9
	@ R8: Kernel entrypoint
	@ R9: UART Address
	@ R10: FDT base address
	@ R11: Symbol information base
	@ R12: End of used RAM
	@ R13: Magic
	ldr r9, =UART_BASE
	ldr r13, =0x1badb002	@ Just ninja the mulitboot magic value
	mov pc, r8

ENTRY(__aeabi_memcpy4)
1:
	LDR r3, [r1], #4
	STR r3, [r0], #4
	SUBS r2, r2, #4
	BGE 1b
	BX lr
ENTRY(__aeabi_memcpy)
1:
	LDRB r3, [r1], #1
	STRB r3, [r0], #1
	SUBS r2, r2, #1
	BGE 1b
	BX lr
ENTRY(memcpy)
1:
	LDRB r3, [r1], #1
	STRB r3, [r0], #1
	SUBS r2, r2, #1
	BGE 1b
	BX lr
ENTRY(__aeabi_memclr8)
ENTRY(__aeabi_memclr4)
	mov r2, #0
ENTRY(__aeabi_memset8)
ENTRY(__aeabi_memset4)
1:
	str r2, [r0], #4
	subs r1, r1, #4
	bge 1b
	bx lr

ENTRY(putb)
	mov r1, #STACK_TOP
	ldr r1, [r1, #-4]
	strb r0, [r1]
	bx lr
@ puts(*const u8, usize)
ENTRY(puts)
	mov r2, #STACK_TOP
	ldr r2, [r2, #-4]
	movs r1, r1
	beq 2f
1:
	ldrb r3, [r0], #1
	strb r3, [r2]
	subs r1, #1
	bne 1b
2:
	bx lr

.section .text.eh_prs
ENTRY(__aeabi_unwind_cpp_pr0)
ENTRY(__aeabi_unwind_cpp_pr1)
	b .

// vim: ft=armasm
