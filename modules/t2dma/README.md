# t2dma

DMA queue engine for Apple T2 BCE clients.

This module is split out from `modules/t2bce` as part of the upstreaming
cleanup. `t2bce` still owns the PCI device, mailbox, command queue policy, and
client transport API. `t2dma` owns the shared queue implementation and the BCE
submission element layout.

Build `t2dma` before `t2bce` so `t2bce` can consume `t2dma`'s exported symbols.
