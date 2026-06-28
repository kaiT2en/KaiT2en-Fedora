use std::process::Command;
use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;

const APP_ID: &str = "org.t2gpuswitch.gtk";
const HELPER: &str = "/usr/local/libexec/t2-gpu-switch-helper";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveGpu {
    Igd,
    Dis,
    Unknown,
}

struct GpuStatus {
    active: ActiveGpu,
    raw: String,
}

fn helper_output(args: &[&str]) -> Result<String, String> {
    let output = Command::new("pkexec")
        .arg(HELPER)
        .args(args)
        .output()
        .map_err(|err| format!("Cannot start pkexec: {err}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("Helper exited with status {}", output.status))
        } else {
            Err(stderr)
        }
    }
}

fn read_status() -> Result<GpuStatus, String> {
    let raw = helper_output(&["status"])?;
    let mut active = ActiveGpu::Unknown;

    for line in raw.lines() {
        if line.contains("IGD:+") {
            active = ActiveGpu::Igd;
            break;
        }
        if line.contains("DIS:+") {
            active = ActiveGpu::Dis;
            break;
        }
    }

    Ok(GpuStatus { active, raw })
}

fn set_status(label: &gtk4::Label, text: &str, error: bool) {
    label.set_text(text);
    if error {
        label.add_css_class("error");
        label.remove_css_class("dim-label");
    } else {
        label.remove_css_class("error");
        label.add_css_class("dim-label");
    }
}

fn refresh(
    status_label: &gtk4::Label,
    detail_label: &gtk4::Label,
    switch_button: &gtk4::Button,
    switch_target: &Rc<RefCell<Option<&'static str>>>,
) {
    match read_status() {
        Ok(status) => {
            match status.active {
                ActiveGpu::Igd => {
                    status_label.set_text("Integrated GPU active");
                    switch_button.set_label("Switch to dGPU");
                    switch_button.set_sensitive(true);
                    *switch_target.borrow_mut() = Some("dis");
                }
                ActiveGpu::Dis => {
                    status_label.set_text("Discrete GPU active");
                    switch_button.set_label("Switch to iGPU");
                    switch_button.set_sensitive(true);
                    *switch_target.borrow_mut() = Some("igd");
                }
                ActiveGpu::Unknown => {
                    status_label.set_text("GPU state unknown");
                    switch_button.set_label("Switch unavailable");
                    switch_button.set_sensitive(false);
                    *switch_target.borrow_mut() = None;
                }
            }
            set_status(detail_label, &status.raw, false);
        }
        Err(err) => {
            status_label.set_text("Unable to read GPU state");
            switch_button.set_label("Switch unavailable");
            switch_button.set_sensitive(false);
            *switch_target.borrow_mut() = None;
            set_status(detail_label, &err, true);
        }
    }
}

fn build_ui(app: &adw::Application) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("T2 GPU Switch")
        .default_width(420)
        .default_height(220)
        .build();

    let header = adw::HeaderBar::new();
    let refresh_button = gtk4::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Refresh")
        .build();
    header.pack_end(&refresh_button);

    let status_label = gtk4::Label::builder()
        .label("Reading GPU state...")
        .xalign(0.0)
        .css_classes(["title-2"])
        .build();

    let detail_label = gtk4::Label::builder()
        .xalign(0.0)
        .selectable(true)
        .wrap(true)
        .css_classes(["dim-label"])
        .build();

    let switch_button = gtk4::Button::builder()
        .label("Switch unavailable")
        .sensitive(false)
        .css_classes(["suggested-action"])
        .build();
    let switch_target = Rc::new(RefCell::new(None));

    let content = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(16)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    content.append(&status_label);
    content.append(&detail_label);
    content.append(&switch_button);

    let layout = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .build();
    layout.append(&header);
    layout.append(&content);
    window.set_content(Some(&layout));

    {
        let status_label = status_label.clone();
        let detail_label = detail_label.clone();
        let switch_button = switch_button.clone();
        let switch_target = switch_target.clone();
        refresh_button.connect_clicked(move |_| {
            refresh(&status_label, &detail_label, &switch_button, &switch_target);
        });
    }

    {
        let detail_label = detail_label.clone();
        let switch_target = switch_target.clone();
        switch_button.connect_clicked(move |button| {
            let target = *switch_target.borrow();
            let Some(target) = target else {
                return;
            };

            button.set_sensitive(false);
            set_status(
                &detail_label,
                "Switch request started. The display manager will restart.",
                false,
            );

            if let Err(err) = helper_output(&[target]) {
                set_status(&detail_label, &err, true);
                button.set_sensitive(true);
            }
        });
    }

    refresh(&status_label, &detail_label, &switch_button, &switch_target);
    window.present();
}

fn main() {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run();
}
