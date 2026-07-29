from pyanaconda.core.dbus import DBus
from pyanaconda.modules.common.base import KickstartService
from pyanaconda.modules.common.containers import TaskContainer

from com_kait2en_input.service.constants import KAIT2EN
from com_kait2en_input.service.installation import (
    ConfigureKernelArgumentsTask,
    InstallBluetoothFirmwareTask,
    InstallGuidedInstallerTask,
    InstallTransitionDriversTask,
    InstallWifiFirmwareTask,
)
from com_kait2en_input.service.kait2en_interface import KaiT2enInterface


class KaiT2enService(KickstartService):
    """Install the temporary T2 input drivers into the target system."""

    def publish(self):
        TaskContainer.set_namespace(KAIT2EN.namespace)
        DBus.publish_object(KAIT2EN.object_path, KaiT2enInterface(self))
        DBus.register_service(KAIT2EN.service_name)

    def install_with_tasks(self):
        return [
            InstallWifiFirmwareTask(),
            InstallBluetoothFirmwareTask(),
            InstallGuidedInstallerTask(),
            InstallTransitionDriversTask(),
        ]

    def configure_bootloader_with_tasks(self, kernel_versions):
        return [ConfigureKernelArgumentsTask()]
