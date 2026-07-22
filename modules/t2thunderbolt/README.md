# t2thunderbolt

`t2thunderbolt` supplies the missing power-management dependencies between
Thunderbolt PCIe ports and their NHI on Apple T2 Macs. These device links make
the driver core resume the NHI before ports whose PCIe tunnels it restores.

The module is a quirk helper and does not bind to the Thunderbolt controller or
replace the in-tree `thunderbolt` driver. Titan Ridge controllers are matched
through their PCIe switch topology. Ice Lake controllers use Apple's `TRP*`
ACPI root-port names and are limited to the two Ice Lake NHI PCI IDs.

Apple's Darwin ACPI path powers down the Thunderbolt and USB topology, but
Linux cannot restore it in time for platform resume. On Titan Ridge, the module
keeps only the downstream ports hosting xHCI and their xHCI functions in D0.
Ice Lake implements Thunderbolt as an integrated TCSS power domain, so its NHI,
integrated xHCI function, and associated `TRP*` root ports remain in D0 as one
unit. This avoids changing unrelated platform power-management policy.

## Known issue

The JHL7540 xHCI controllers still report `USBSTS 0x401` during resume. Their
internal context is lost even though PCI keeps the controllers in D0, so the
xHCI driver detects the failed restore and reinitializes them. A future in-tree
xHCI quirk should mark these controllers for reset on resume instead of first
attempting to restore a context the platform does not preserve.
