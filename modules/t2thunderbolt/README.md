# t2thunderbolt

`t2thunderbolt` supplies the missing power-management dependencies between
Thunderbolt PCIe ports and their NHI on Apple T2 Macs. These device links make
the driver core resume the NHI before ports whose PCIe tunnels it restores.

The module is a quirk helper for T2 Macs with discrete Titan Ridge controllers.
It does not bind to the Thunderbolt controller or replace the in-tree
`thunderbolt` driver. Ice Lake systems are deliberately excluded because their
integrated TCSS topology requires a different suspend fix.

Apple's Darwin ACPI path powers down the switch ports hosting the Thunderbolt
xHCI controllers, but the controllers subsequently report a context
save/restore error. The module keeps only those downstream ports out of D3.
It also keeps their xHCI functions in D0. This avoids slow CPU bring-up and PCIe
bridge timeouts during resume while leaving the rest of Apple's platform
power-management policy unchanged.

## Known issue

The JHL7540 xHCI controllers still report `USBSTS 0x401` during resume. Their
internal context is lost even though PCI keeps the controllers in D0, so the
xHCI driver detects the failed restore and reinitializes them. ~~A future in-tree
xHCI quirk should mark these controllers for reset on resume instead of first
attempting to restore a context the platform does not preserve.~~ It seems like
the failed attempt is necessary to Initalize the xHCI controller, we need to save the context like using pci_save_state, to remove the warning.
