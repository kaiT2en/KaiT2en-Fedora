# SPDX-License-Identifier: GPL-3.0-or-later
from pyanaconda.modules.common import init

init()

from com_kait2en_input.service.kait2en import KaiT2enService

service = KaiT2enService()
service.run()
