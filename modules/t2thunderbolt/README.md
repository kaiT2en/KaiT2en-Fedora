# t2thunderbolt

`t2thunderbolt` supplies the missing power-management dependencies between
Thunderbolt PCIe ports and their NHI on Apple T2 Macs. These device links make
the driver core resume the NHI before ports whose PCIe tunnels it restores.

The module is a quirk helper and does not bind to the Thunderbolt controller or
replace the in-tree `thunderbolt` driver. Titan Ridge controllers are matched
through their PCIe switch topology. Ice Lake controllers use Apple's `TRP*`
ACPI root-port names and are limited to the two Ice Lake NHI PCI IDs.

Apple's Darwin ACPI path powers down the Titan Ridge xHCI controllers, but the
controllers subsequently report a context save/restore error. The module
matches the JHL7540 2C and 4C and JHL7440 DD xHCI IDs and keeps each controller
and its immediate downstream port out of D3. This avoids slow CPU bring-up and
PCIe bridge timeouts during resume while leaving the rest of Apple's platform
power-management policy unchanged.

On Ice Lake, the module only supplies the missing PM ordering links. It does
not alter PCI D-states because the integrated TCSS topology remains under
firmware power management.

## Known issue

The JHL7540 xHCI controllers still report `USBSTS 0x401` during resume. Their
internal context is lost even though PCI keeps the controllers in D0, so the
xHCI driver detects the failed restore and reinitializes them. ~~A future in-tree
xHCI quirk should mark these controllers for reset on resume instead of first
attempting to restore a context the platform does not preserve.~~ It seems like
the failed attempt is necessary to Initalize the xHCI controller, we need to save the context like using pci_save_state, to remove the warning.
