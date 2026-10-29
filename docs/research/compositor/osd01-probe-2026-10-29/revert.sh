echo "revert $(date -u +%T) uptime $(cut -d' ' -f1 /proc/uptime)" >> /tmp/osd01/revert.log
/sbin/devmem 0xCC480400 32 0x00080010
/sbin/devmem 0xCC480440 32 0x00000000
/sbin/devmem 0xCC480404 32 0x8D0B0000
/sbin/devmem 0xCC480420 32 0x00000000
/sbin/devmem 0xCC480424 32 0x07800870
/sbin/devmem 0xCC480428 32 0x07800870
/sbin/devmem 0xCC48042C 32 0x6EFF03C0
/sbin/devmem 0xCC480430 32 0x00000000
/sbin/devmem 0xCC480434 32 0x8D0B0000
/sbin/devmem 0xCC480438 32 0x8D0B0000
