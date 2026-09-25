MEMORY {
  FLASH : ORIGIN = 0x10000000, LENGTH = 2048K
  RAM   : ORIGIN = 0x20000000, LENGTH = 512K
}

SECTIONS {
  .start_block : ALIGN(4) {
    __start_block_addr = .;
    KEEP(*(.start_block));
    KEEP(*(.boot_info));
    . = ALIGN(8);
  } > FLASH
} INSERT AFTER .vector_table;

_stext = ADDR(.start_block) + SIZEOF(.start_block);
