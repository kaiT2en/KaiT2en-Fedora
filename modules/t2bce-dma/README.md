# t2bce-dma

DMA queue engine for Apple T2 BCE clients.

This module is split out from `modules/t2bce-core` as part of the upstreaming
cleanup. `t2bce-core` still owns the PCI device, mailbox, command queue policy, and
client transport API. `t2bce-dma` owns the shared queue implementation and the BCE
submission element layout.

Build `t2bce-dma` before `t2bce-core` so `t2bce-core` can consume `t2bce-dma`'s exported symbols.
