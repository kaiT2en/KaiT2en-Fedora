# SPDX-License-Identifier: GPL-3.0-or-later
from dasbus.server.interface import dbus_interface

from pyanaconda.modules.common.base import KickstartModuleInterface

from com_kait2en_input.service.constants import KAIT2EN


@dbus_interface(KAIT2EN.interface_name)
class KaiT2enInterface(KickstartModuleInterface):
    """DBus interface for the KaiT2en transition-driver add-on."""
