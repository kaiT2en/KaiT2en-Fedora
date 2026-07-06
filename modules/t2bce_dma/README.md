# t2bce_dma

DMA queue engine for Apple T2 BCE clients.

This module is split out from `modules/t2bce_core` as part of the upstreaming
cleanup. `t2bce_core` still owns the PCI device, mailbox, command queue policy, and
client transport API. `t2bce_dma` owns the shared queue implementation and the BCE
submission element layout.

Build `t2bce_dma` before `t2bce_core` so `t2bce_core` can consume `t2bce_dma`'s exported symbols.
