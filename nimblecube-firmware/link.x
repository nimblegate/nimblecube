MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20000000, LENGTH = 64K
}

EXTERN(RESET_VECTOR);
ENTRY(Reset);

SECTIONS
{
  .vector_table ORIGIN(FLASH) :
  {
    LONG(ORIGIN(RAM) + LENGTH(RAM));   /* initial stack pointer */
    KEEP(*(.vector_table.reset_vector));
  } > FLASH

  .text :
  {
    *(.text .text.*)
  } > FLASH

  .rodata :
  {
    *(.rodata .rodata.*)
  } > FLASH

  .bss :
  {
    *(.bss .bss.*)
  } > RAM

  .data :
  {
    *(.data .data.*)
  } > RAM

  /DISCARD/ :
  {
    *(.ARM.exidx .ARM.exidx.*)
  }
}
