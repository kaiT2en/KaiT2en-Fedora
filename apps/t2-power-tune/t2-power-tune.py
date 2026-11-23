#!/usr/bin/python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 André Eikmeyer <andre.eikmeyer@kait2en.org>
import json, os, subprocess, threading
import gi
gi.require_version("Gtk","4.0"); gi.require_version("Adw","1")
from gi.repository import Adw, Gdk, GdkPixbuf, GLib, Gtk

APP_ID="org.t2powertune.gtk"
APP_VERSION="0.02"
HELPER="/usr/local/libexec/t2-power-tune-helper"
STATUS="/usr/local/libexec/t2-power-tune-status"
SERVICE="/etc/systemd/system/kait2en-power-tune.service"

def kait2en_brand():
    # White type for dark surfaces, dark type for light ones. Without the
    # light file the white one is used.
    dark=GdkPixbuf.Pixbuf.new_from_file("/usr/local/share/kait2en/kait2en-wordmark.png")
    try: light=GdkPixbuf.Pixbuf.new_from_file("/usr/local/share/kait2en/kait2en-wordmark-light.png")
    except GLib.Error: light=dark
    manager=Adw.StyleManager.get_default()
    brand=Gtk.DrawingArea(content_width=80,content_height=21)
    def draw(_area,context,width,height):
        pixbuf=dark if manager.get_dark() else light
        scale=min(width/pixbuf.get_width(),height/pixbuf.get_height())
        context.save(); context.translate((width-pixbuf.get_width()*scale)/2,(height-pixbuf.get_height()*scale)/2)
        context.scale(scale,scale); Gdk.cairo_set_source_pixbuf(context,pixbuf,0,0); context.paint(); context.restore()
    brand.set_draw_func(draw); brand.set_size_request(80,21); manager.connect("notify::dark",lambda *_:brand.queue_draw()); return brand

SECTION_HELP={
    "PCIe ASPM candidates":("PCIe ASPM link states",
        "L0s and L1 reduce the power used by an idle PCIe link. The states are selected independently and are only written after Apply.\n\nThe numerically highest or most complete combination is not always the best one. Hardware and drivers may behave better with L1 alone. On some platforms L0s can even prevent deeper package C-states. Test each link and keep only combinations that remain stable across suspend and resume before writing to systemd."),
    "LTR requirements":("Latency Tolerance Reporting",
        "LTR tells the platform how long a device can tolerate delayed service. A strict requirement can keep the package in a shallower C-state.\n\nIgnore does not rewrite the device's Snoop or No-snoop latency. It only tells the PMC not to use that source as a constraint. This can improve residency, but may cause timeouts, audio dropouts or unstable I/O. Apply one source at a time and verify the device afterwards."),
    "Runtime power management":("Runtime power management",
        "Runtime PM lets the kernel suspend an idle device on runtime. A checked device is changed from on to auto after Apply.\n\nSome drivers cannot resume every device reliably. Symptoms include failed probe, missing Wi-Fi or Bluetooth, and PCIe errors after suspend. In particular, leave BCM4377 runtime PM disabled unless suspend and resume have been tested successfully."),
    "Wake sources":("Wake sources",
        "Enabled USB wake sources and Wake-on-LAN can prevent the platform from reaching deeper package C-states. A checked source is changed from enabled to disabled after Apply.\n\nDisabling a source also prevents that device from waking the computer. Test suspend and resume before making the selection persistent."),
    "Other power tunables":("Other power tunables",
        "These controls reduce periodic wakeups and are independent of PCIe ASPM, device runtime PM and wake-source settings.\n\nChanges can affect diagnostics and writeback timing. Apply them individually when troubleshooting, and verify suspend, resume and normal operation before making them persistent."),
}

CSTATE_HELP={
    "C2":"PC2 is a package state. To enter it, every individual CPU core must already be in Core C6 or deeper; Core C6 and Package C6 are different states. Graphics must be in RC6. Timers, device traffic or LTR constraints can keep the package at PC2.",
    "C3":"PC3 requires every individual CPU core to be in Core C6 or deeper, graphics in RC6 and device LTR that permits PC3. Memory can enter self-refresh, its clock can stop, and the LLC may be flushed. This does not require prior Package C6 residency.",
    "C6":"PC6 builds on the PC3 platform conditions. BCLK can stop and voltage regulators can reduce voltage. Frequent core wakeups, near timers or restrictive device latency requirements prevent useful residency.",
    "C7":"A platform-specific intermediate deep state. It requires the platform conditions for PC6 plus sufficiently long, uninterrupted core and device idle. Intel does not publish one universal entry delay for all systems.",
    "C8":"Requires all cores to request C8 or deeper, graphics in RC6 and device LTR that permits PC8. The LLC is flushed and its voltage can be removed.",
    "C9":"A platform-specific step between PC8 and PC10. It needs deeper core and platform idle; display configuration, PSR, device latency and wake activity can limit entry.",
    "C10":"Requires PC8 conditions, deep idle on every core, graphics in RC6, suitable device LTR and low wake activity. With the internal display active, the panel must normally be in PSR; otherwise the display must be powered off. Regulators reach their deep power state and the crystal clock can stop.",
}

ACTION_HELP=("Actions",
    "Apply selected changes writes the currently chosen values for this boot and verifies them. Nothing is written merely by changing a checkbox.\n\nCreate or Update systemd service stores the same selection in kait2en-power-tune.service so it is restored at boot. Unselected managed values are removed from the service.\n\nRescan reads the current hardware state again. It does not apply settings, but hardware values such as LTR can change dynamically while devices are active.")

def palette_css(dark):
    if dark: window,fg,panel,line,rim,hairline="#151517","#e8e8e8","#0d0d0f","rgba(255,255,255,0.13)","rgba(255,255,255,0.24)","rgba(255,255,255,0.06)"
    else: window,fg,panel,line,rim,hairline="#ffffff","#242424","#e2e2e8","rgba(0,0,0,0.16)","rgba(255,255,255,0.70)","rgba(0,0,0,0.08)"
    return f"window{{color:alpha({fg},0.72);}}button,button label,entry,spinbutton,dropdown{{color:alpha({fg},0.72);}}.monospace{{font-family:'JetBrains Mono';}}window.csd{{border-top:1px solid {rim};}}window.csd.maximized,window.csd.tiled,window.csd.tiled-top,window.csd.tiled-left,window.csd.tiled-right,window.csd.tiled-bottom,window.csd.fullscreen{{border-top-width:0;}}.app-background{{background:{window};color:alpha({fg},0.72);}}.app-background .title-1,.app-background .title-2,.app-background .title-3,.app-background .title-4,.app-background .title,.app-background .heading,windowtitle .title{{color:{fg};font-weight:600;}}.app-background .dim-label,windowtitle .subtitle{{color:alpha({fg},0.72);}}.app-background headerbar{{background:{window};color:alpha({fg},0.72);}}.unified-box{{background:{panel};color:alpha({fg},0.72);border-radius:12px;padding:14px;box-shadow:inset 0 0 0 1px {hairline};}}.sticky-bar{{padding:9px 18px;}}.cstate-cell{{border:1px solid {line};border-radius:7px;padding:3px 6px;font-family:'JetBrains Mono';font-feature-settings:'tnum';}}.app-background .cstate-zero{{color:#808080;border-color:#808080;}}.app-background .cstate-positive{{color:#3b8132;border-color:#3b8132;}}.donate-link,.donate-link>label{{color:alpha({fg},0.72);}}"

class PowerTune(Adw.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID); self.connect("activate",self.activate)
        self.items=[]; self.rows=[]; self.cstate_process=None
    def activate(self,_app):
        self.win=Adw.ApplicationWindow(application=self,title="T2 Power Tune"); self.win.set_default_size(950,780)
        manager=Adw.StyleManager.get_default(); css=Gtk.CssProvider(); css.load_from_string(palette_css(manager.get_dark()))
        Gtk.StyleContext.add_provider_for_display(self.win.get_display(),css,Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        manager.connect("notify::dark",lambda m,_p:css.load_from_string(palette_css(m.get_dark())))
        root=Gtk.Box(orientation=Gtk.Orientation.VERTICAL); root.add_css_class("app-background")
        header=Adw.HeaderBar(); header.set_title_widget(Adw.WindowTitle(title="Power Tune",subtitle="PCIe ASPM and power tunables"))
        brand=kait2en_brand(); brand.set_margin_start(10); brand.set_margin_end(10); header.pack_start(brand); root.append(header)
        metrics=Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL,spacing=8); metrics.add_css_class("sticky-bar"); metrics.set_margin_bottom(8)
        title=Gtk.Label(label="Package C-states",xalign=0); title.add_css_class("heading"); metrics.append(title)
        self.cstate_values={}
        for state in ("C2","C3","C6","C7","C8","C9","C10"):
            cell=Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL,spacing=2)
            cell.set_size_request(86,30); cell.set_valign(Gtk.Align.CENTER)
            cell.add_css_class("cstate-cell")
            name=Gtk.Label(label=state)
            value=Gtk.Label(label="…",xalign=1); value.set_width_chars(7); value.set_max_width_chars(7)
            cell.append(name); cell.append(value); metrics.append(cell)
            self.cstate_values[state]=(cell,name,value)
            self.add_delayed_tooltip(cell,CSTATE_HELP[state])
        window=Gtk.Label(label="Last 1 s · live"); window.add_css_class("dim-label"); metrics.append(window)
        root.append(metrics)
        scroll=Gtk.ScrolledWindow(vexpand=True); body=Gtk.Box(orientation=Gtk.Orientation.VERTICAL,spacing=14)
        for side in ("top","bottom","start","end"): getattr(body,f"set_margin_{side}")(18)
        scroll.set_child(body); root.append(scroll)
        self.aspm_box=self.section(body,"PCIe ASPM candidates")
        self.constraint_box=self.section(body,"LTR requirements")
        self.tunable_box=self.section(body,"Runtime power management")
        self.wakeup_box=self.section(body,"Wake sources")
        self.other_box=self.section(body,"Other power tunables")
        footer=Gtk.Box(orientation=Gtk.Orientation.VERTICAL,spacing=6); footer.add_css_class("sticky-bar")
        actions=Gtk.Box(spacing=8); self.apply_btn=Gtk.Button(label="Apply selected changes"); self.apply_btn.add_css_class("suggested-action")
        self.service_btn=Gtk.Button(); self.update_service_button(); self.rescan_btn=Gtk.Button(label="Rescan")
        for button in (self.apply_btn,self.service_btn,self.rescan_btn): actions.append(button)
        action_help=Gtk.Button(icon_name="help-about-symbolic",has_frame=False)
        action_help.set_tooltip_text("Explain the actions")
        action_help.connect("clicked",lambda *_:self.show_help(*ACTION_HELP)); actions.append(action_help)
        self.apply_btn.set_tooltip_text("Write and verify the selected values for this boot")
        self.service_btn.set_tooltip_text("Make the selected values persistent at boot")
        self.rescan_btn.set_tooltip_text("Read the current hardware state again")
        footer.append(actions)
        self.notice=Gtk.Label(xalign=0,wrap=True); self.notice.add_css_class("dim-label"); footer.append(self.notice)
        root.append(footer)
        status=Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL,spacing=8)
        status.set_margin_start(18); status.set_margin_end(18); status.set_margin_bottom(8)
        donate=Gtk.LinkButton(uri="https://donate.stripe.com/eVq14n8a7agh2lQdqq14400",label="Fund our bugs")
        donate.add_css_class("donate-link"); status.append(donate)
        status.append(Gtk.Box(hexpand=True))
        version=Gtk.Label(label=f"v{APP_VERSION}"); version.add_css_class("dim-label"); status.append(version)
        root.append(status); self.win.set_content(root)
        self.apply_btn.connect("clicked",lambda *_:self.run_action("apply")); self.service_btn.connect("clicked",lambda *_:self.run_action("persist")); self.rescan_btn.connect("clicked",lambda *_:self.scan())
        self.win.connect("close-request",self.close); self.win.present(); self.scan(); self.start_cstates()
    def section(self,parent,title):
        panel=Gtk.Box(orientation=Gtk.Orientation.VERTICAL,spacing=8); panel.add_css_class("unified-box")
        heading_row=Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL,spacing=6)
        heading=Gtk.Label(label=title,xalign=0,hexpand=True); heading.add_css_class("title-4")
        help_button=Gtk.Button(icon_name="help-about-symbolic",has_frame=False,valign=Gtk.Align.CENTER)
        help_button.set_tooltip_text(f"Explain {title}")
        help_button.connect("clicked",lambda *_args,t=title:self.show_help(*SECTION_HELP[t]))
        heading_row.append(heading); heading_row.append(help_button)
        content=Gtk.Box(orientation=Gtk.Orientation.VERTICAL,spacing=5); panel.append(heading_row); panel.append(content); parent.append(panel); return content
    def show_help(self,title,text):
        dialog=Adw.Window(transient_for=self.win,modal=True,title=title); dialog.set_default_size(560,360)
        content=Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        header=Adw.HeaderBar(); header.set_title_widget(Adw.WindowTitle(title=title)); content.append(header)
        label=Gtk.Label(label=text,xalign=0,yalign=0,wrap=True,selectable=False)
        label.set_focusable(False)
        for side in ("top","bottom","start","end"):getattr(label,f"set_margin_{side}")(20)
        content.append(label); dialog.set_content(content); dialog.present()
    def add_delayed_tooltip(self,widget,text):
        popover=Gtk.Popover(autohide=False,has_arrow=True)
        label=Gtk.Label(label=text,xalign=0,wrap=True); label.set_max_width_chars(48)
        for side in ("top","bottom","start","end"):getattr(label,f"set_margin_{side}")(10)
        popover.set_child(label); popover.set_parent(widget)
        timer={"id":0}
        def show():
            timer["id"]=0; popover.popup(); return False
        def enter(*_args):
            if not timer["id"]:timer["id"]=GLib.timeout_add(2000,show)
        def leave(*_args):
            if timer["id"]:GLib.source_remove(timer["id"]); timer["id"]=0
            popover.popdown()
        motion=Gtk.EventControllerMotion(); motion.connect("enter",enter); motion.connect("leave",leave); widget.add_controller(motion)
    def busy(self,value,message=""):
        for button in (self.apply_btn,self.service_btn,self.rescan_btn): button.set_sensitive(not value)
        if message:self.notice.set_text(message)
    def update_service_button(self):
        action="Update" if os.path.isfile(SERVICE) else "Create"
        self.service_btn.set_label(f"{action} systemd service")
    def scan(self): self.busy(True,"Scanning hardware…"); threading.Thread(target=self._scan,daemon=True).start()
    def _scan(self):
        result=subprocess.run(["pkexec","--disable-internal-agent",HELPER,"scan"],text=True,capture_output=True)
        GLib.idle_add(self.finish_scan,result.returncode,result.stdout,result.stderr)
    def finish_scan(self,code,stdout,stderr):
        self.busy(False)
        if code:
            self.notice.set_text(stderr.strip() or "Scan failed or authentication was cancelled.")
            return False
        try:scan_data=json.loads(stdout)
        except ValueError as error:
            self.notice.set_text(f"Invalid scan result: {error}")
            return False
        self.items=scan_data["items"] if isinstance(scan_data,dict) else scan_data
        constraints=scan_data.get("constraints",[]) if isinstance(scan_data,dict) else []
        def item_group(item):
            if item["kind"]=="aspm":return 0
            if item["kind"]=="ltr":return 1
            return {"runtime":2,"wakeup":3,"other":4}.get(item.get("group"),4)
        self.items.sort(key=lambda item:(item_group(item),item["label"].casefold()))
        self.rows=[]
        for box in (self.aspm_box,self.constraint_box,self.tunable_box,self.wakeup_box,self.other_box):
            while (child:=box.get_first_child()):box.remove(child)
        for item in self.items:
            row=Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL,spacing=10)
            labels=Gtk.Box(orientation=Gtk.Orientation.VERTICAL,spacing=2,hexpand=True,valign=Gtk.Align.CENTER); title=Gtk.Label(label=item["label"],xalign=0,wrap=True)
            labels.append(title)
            if item["kind"]=="aspm":
                desired=int(item["target"] if item.get("selected",False) else item["current"])
                supported=int(item.get("supported",int(item["target"])))
                choices=Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL,spacing=12)
                l0s=Gtk.CheckButton(label="L0s",active=bool(desired & 1),sensitive=bool(supported & 1))
                l1=Gtk.CheckButton(label="L1",active=bool(desired & 2),sensitive=bool(supported & 2))
                choices.append(l0s); choices.append(l1)
                row.append(labels); row.append(choices)
                control={"kind":"aspm","l0s":l0s,"l1":l1}
            else:
                check=Gtk.CheckButton(active=item.get("selected",False),valign=Gtk.Align.CENTER)
                row.append(check); row.append(labels)
                control={"kind":"check","check":check}
            target=(self.aspm_box if item["kind"]=="aspm" else
                    self.constraint_box if item["kind"]=="ltr" else
                    self.wakeup_box if item.get("group")=="wakeup" else
                    self.other_box if item.get("group")=="other" else self.tunable_box)
            target.append(row); self.rows.append(control)
        if not any(i["kind"]=="aspm" for i in self.items):self.aspm_box.append(Gtk.Label(label="No additional ASPM states found.",xalign=0))
        for constraint in sorted(constraints,key=lambda item:item["label"].casefold()):
            labels=Gtk.Box(orientation=Gtk.Orientation.VERTICAL,spacing=2)
            labels.append(Gtk.Label(label=constraint["label"],xalign=0,wrap=True))
            values=[]
            if constraint["snoop_ns"] is not None:values.append(f'Snoop {constraint["snoop_ns"]:,} ns')
            if constraint["nonsnoop_ns"] is not None:values.append(f'No-snoop {constraint["nonsnoop_ns"]:,} ns')
            values.append(f'PMC{constraint["pmc"]} · index {constraint["index"]} · raw 0x{constraint["raw"]}')
            detail=Gtk.Label(label=" · ".join(values),xalign=0,wrap=True); detail.add_css_class("dim-label"); labels.append(detail)
            self.constraint_box.append(labels)
        if not constraints and not any(i["kind"]=="ltr" for i in self.items):self.constraint_box.append(Gtk.Label(label="No active LTR requirements found.",xalign=0))
        if not any(i["kind"]=="tunable" and i.get("group")=="runtime" for i in self.items):self.tunable_box.append(Gtk.Label(label="Runtime PM is already allowed for every discovered device.",xalign=0))
        if not any(i.get("group")=="wakeup" for i in self.items):self.wakeup_box.append(Gtk.Label(label="No active wake sources found.",xalign=0))
        if not any(i.get("group")=="other" for i in self.items):self.other_box.append(Gtk.Label(label="No additional inactive tunables found.",xalign=0))
        self.notice.set_text(f"{len(self.items)} configurable item(s) found.")
        return False
    def payload(self):
        result=[]
        for item,row in zip(self.items,self.rows):
            if row["kind"]=="aspm":
                target=(1 if row["l0s"].get_active() else 0) | (2 if row["l1"].get_active() else 0)
                enabled=target != int(item["current"])
            else:
                target=item["target"]; enabled=row["check"].get_active()
            result.append({"kind":item["kind"],"id":item["id"],"original":item["current"],
                           "target":str(target),"enabled":enabled})
        return result
    def row_active(self,row):
        if row["kind"]=="aspm":return row["l0s"].get_active() or row["l1"].get_active()
        return row["check"].get_active()
    def run_action(self,action):
        self.start_action(action)
    def start_action(self,action):
        self.service_existed=os.path.isfile(SERVICE)
        service_action="Updating" if self.service_existed else "Creating"
        self.busy(True,"Applying changes…" if action=="apply" else f"{service_action} systemd service…")
        threading.Thread(target=self._action,args=(action,json.dumps(self.payload())),daemon=True).start()
    def _action(self,action,payload):
        result=subprocess.run(["pkexec","--disable-internal-agent",HELPER,action],input=payload,text=True,capture_output=True)
        GLib.idle_add(self.finish_action,action,result.returncode,result.stdout,result.stderr)
    def finish_action(self,action,code,stdout,stderr):
        self.busy(False)
        if code:self.notice.set_text(stderr.strip() or "Operation failed or authentication was cancelled.")
        elif action=="apply":
            self.notice.set_text("Selection applied and verified.")
            try:changes=json.loads(stdout)
            except ValueError:changes=[]
            self.show_apply_result(changes)
            if changes:GLib.timeout_add_seconds(2,self.rescan_after_apply)
        elif any(item["enabled"] for item in self.payload()):
            verb="updated" if self.service_existed else "created"
            self.notice.set_text(f"kait2en-power-tune.service was {verb} and enabled.")
        else:self.notice.set_text("The empty service configuration was removed.")
        if action=="persist" and not code:self.update_service_button()
        return False
    def rescan_after_apply(self):
        self.scan()
        return False
    def show_apply_result(self,changes):
        lines=[]
        for change in changes:
            if change["kind"]=="aspm":
                states={"0":"Disabled","1":"L0s","2":"L1","3":"L0s + L1"}
                lines.append(f'PCI {change["id"]}\n  ASPM = {states.get(change["value"],change["value"])}')
            elif change["kind"]=="ltr":
                state="ignored" if change["value"]=="1" else "restored"
                lines.append(f'{change["id"]}\n  {change["location"]} = {state}')
            else:
                path=change["id"]
                if "/usb/devices/" in path:address="USB "+path.split("/devices/",1)[1].split("/",1)[0]
                elif "/pci/devices/" in path:address="PCI "+path.split("/devices/",1)[1].split("/",1)[0]
                else:address="System tunable"
                lines.append(f'{address}\n  {change["location"]} = {change["value"]}')
        body="\n\n".join(lines) if lines else "No values needed changing."
        dialog=Adw.Window(transient_for=self.win,modal=True,title="Applied and verified")
        dialog.set_default_size(620,440)
        content=Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        header=Adw.HeaderBar(); header.set_title_widget(Adw.WindowTitle(title="Applied and verified")); content.append(header)
        view=Gtk.TextView(editable=False,cursor_visible=False,monospace=True,wrap_mode=Gtk.WrapMode.WORD_CHAR)
        view.get_buffer().set_text(body)
        scroll=Gtk.ScrolledWindow(vexpand=True); scroll.set_child(view)
        scroll.set_margin_top(12); scroll.set_margin_start(18); scroll.set_margin_end(18); content.append(scroll)
        actions=Gtk.Box(halign=Gtk.Align.END); actions.set_margin_top(12); actions.set_margin_bottom(12); actions.set_margin_end(18)
        ok=Gtk.Button(label="OK"); ok.add_css_class("suggested-action"); ok.connect("clicked",lambda *_:dialog.close())
        actions.append(ok); content.append(actions); dialog.set_content(content); dialog.present()
    def start_cstates(self):
        try:
            self.cstate_process=subprocess.Popen(["pkexec","--disable-internal-agent",STATUS,str(os.getpid())],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,bufsize=1)
            threading.Thread(target=self.read_cstates,daemon=True).start()
        except OSError as error:self.update_cstates({"error":str(error)})
    def read_cstates(self):
        for line in self.cstate_process.stdout:
            try:data=json.loads(line)
            except ValueError:continue
            GLib.idle_add(self.update_cstates,data)
        error=self.cstate_process.stderr.read().strip()
        if error:GLib.idle_add(self.update_cstates,{"error":error})
    def update_cstates(self,data):
        if "error" in data:
            for cell,name,label in self.cstate_values.values():
                label.set_text("Unavailable")
                cell.set_css_classes(["cstate-cell"])
            return False
        package=data.get("package",{})
        for state,(cell,name,label) in self.cstate_values.items():
            value=package.get(state)
            if value is None:
                label.set_text("—"); cell.set_css_classes(["cstate-cell"])
            elif value > 0:
                label.set_text("<0.01%" if value < 0.01 else f"{value:.2f}%")
                cell.set_css_classes(["cstate-cell","cstate-positive"])
            else:
                label.set_text(f"{value:.2f}%")
                cell.set_css_classes(["cstate-cell","cstate-zero"])
        return False
    def close(self,*_args):
        return False

PowerTune().run()
