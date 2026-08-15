# t2thunderbolt

`t2thunderbolt` supplies the missing power-management dependencies between
Thunderbolt PCIe ports and their NHI on Apple T2 Macs. These device links make
the driver core resume the NHI before ports whose PCIe tunnels it restores.

The module is a quirk helper and does not bind to the Thunderbolt controller or
replace the in-tree `thunderbolt` driver. Titan Ridge controllers are matched
through their PCIe switch topology. Ice Lake controllers use Apple's `TRP*`
ACPI root-port names and are limited to the two Ice Lake NHI PCI IDs.

On Titan Ridge systems the module keeps only the xHCI downstream ports out of
D3. The xHCI controllers may runtime suspend. This preserves fast system
resume while allowing the controllers themselves to enter D3hot. PM ordering
links remain active for the other Thunderbolt hotplug ports and NHIs.
